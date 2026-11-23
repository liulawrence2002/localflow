//! Judges from the sound of a pause whether the speaker has finished their turn.
//!
//! The speech detector ([`super::speech_detector`]) only knows that the user went quiet. A
//! [`TurnCompletionModel`] listens to the last few seconds before the pause and says how
//! likely it is that they are done, so conversation mode can answer quickly after a finished
//! sentence and keep waiting after "I think we should, um".
//!
//! One model exists:
//!
//! - [`SmartTurn`]: Pipecat Smart Turn v3.2 (BSD-2-Clause), a Whisper Tiny encoder with a
//!   small classifier on top. It runs in-process on the CPU through `tract`, an ONNX runtime
//!   written in Rust: nothing is downloaded at build time and no runtime DLL is needed.
//!   Installed by `scripts/Get-SmartTurnModel.ps1`.
//!
//! Smart Turn is published as two files. This uses the full-precision one (32 MB, named
//! `-gpu` upstream although it runs fine on a CPU), because `tract` reproduces the reference
//! runtime's answers for it to five decimal places. The 8 MB 8-bit file (`-cpu`) loads in
//! `tract` but gives different, wrong answers there, and is no faster, so it is not used.
//!
//! When the model is not installed, [`best_turn_model`] returns none and says why; the caller
//! then keeps the fixed silence wait.
//!
//! The model does not take audio directly. It takes the picture of the sound that Whisper
//! uses (a log-mel spectrogram), and the picture has to be computed exactly the way the model
//! was trained, or the answers are quietly wrong. [`LogMel`] reproduces the reference
//! (`WhisperFeatureExtractor(chunk_length=8)` with `do_normalize=True`, as called by
//! `inference.py` in pipecat-ai/smart-turn) and is tested against numbers produced by it.

use std::{f64::consts::PI, path::Path, sync::Arc};

use tract_linalg::multithread::Executor;
use tract_onnx::prelude::*;

/// The model takes mono audio at this rate.
pub const SMART_TURN_SAMPLE_RATE: u32 = 16_000;
/// File name of the Smart Turn model in the runtime `models` folder.
pub const SMART_TURN_MODEL_FILE: &str = "smart-turn-v3.2-gpu.onnx";

/// The model always looks at exactly this much audio: 8 seconds.
const WINDOW_SAMPLES: usize = 8 * SMART_TURN_SAMPLE_RATE as usize;
/// Samples per analysis frame: 25 ms.
const FFT_LEN: usize = 400;
/// Samples between frames: 10 ms.
const HOP: usize = 160;
/// Frequencies from one frame, 0 to 8 kHz inclusive.
const BINS: usize = FFT_LEN / 2 + 1;
/// Rows of the picture: bands spaced the way hearing spaces pitch.
const MEL_BANDS: usize = 80;
/// Columns of the picture: one per 10 ms.
const FRAMES: usize = WINDOW_SAMPLES / HOP;
/// Power below this is treated as this, so silence has a finite logarithm.
const MEL_FLOOR: f64 = 1e-10;
/// Added to the variance so that digital silence does not divide by zero.
const VARIANCE_FLOOR: f64 = 1e-7;

/// Decides from the sound of the last few seconds whether the speaker has finished their turn.
pub trait TurnCompletionModel: Send {
    /// Stable identifier for diagnostics, for example `smart-turn-v3.2`.
    fn id(&self) -> &'static str;

    /// `audio` is mono 16 kHz, the most recent speech ending at the pause. Any length is
    /// accepted: the last 8 seconds are used, and shorter audio is treated as preceded by
    /// silence. Give the whole turn so far, not only the words since the last pause.
    ///
    /// Returns the probability in `[0, 1]` that the turn is complete. Errors are fixed
    /// descriptions and never contain the audio or a path.
    fn probability_complete(&mut self, audio: &[f32]) -> Result<f32, String>;
}

/// Pipecat Smart Turn v3.2 through `tract`.
pub struct SmartTurn {
    plan: Arc<TypedRunnableModel>,
    front_end: LogMel,
}

impl SmartTurn {
    /// Load the model at `model_path`. Errors name the kind of failure, not the path.
    ///
    /// Loading parses and optimises the network and takes a noticeable fraction of a second:
    /// do it once, off the audio thread. Any Smart Turn v3 file with the same input loads, but
    /// only the full-precision one gives trustworthy answers (see the module notes).
    pub fn load(model_path: &Path) -> Result<Self, String> {
        if !model_path.is_file() {
            return Err(NOT_INSTALLED.to_string());
        }
        let features = f32::fact([1, MEL_BANDS, FRAMES]);
        let plan = tract_onnx::onnx()
            // The file declares its shapes for "any number of clips at once". This runs one
            // clip, so the shapes are worked out again from the input given below instead.
            .with_ignore_value_info(true)
            .with_ignore_output_shapes(true)
            .model_for_path(model_path)
            .map_err(|_| "Could not read the turn-completion model file.".to_string())?
            .with_input_fact(0, features.into())
            .map_err(|_| "The turn-completion model expects a different input.".to_string())?
            .into_optimized()
            .map_err(|_| {
                "The turn-completion model uses operations that are not supported.".to_string()
            })?
            .into_runnable_with_options(&RunOptions {
                executor: Some(model_executor()),
                ..RunOptions::default()
            })
            .map_err(|_| "Could not prepare the turn-completion model to run.".to_string())?;

        Ok(Self {
            plan,
            front_end: LogMel::new(),
        })
    }
}

/// A small private thread pool for the model's matrix products. Measured on a 16-core laptop,
/// model only: 1 thread 108 ms a call, 2 threads 74 ms, 4 threads 50 ms, 6 threads 53 ms,
/// 8 threads 62 ms. The threads sleep between calls, and a call only happens at a pause.
fn model_executor() -> Executor {
    const MOST_USEFUL: usize = 4;
    let threads = std::thread::available_parallelism()
        .map_or(1, |count| count.get())
        .min(MOST_USEFUL);
    if threads < 2 {
        return Executor::SingleThread;
    }
    // The library panics if the operating system refuses the threads; one thread still works.
    std::panic::catch_unwind(|| Executor::multithread_with_name(threads, "smart-turn"))
        .unwrap_or(Executor::SingleThread)
}

impl TurnCompletionModel for SmartTurn {
    fn id(&self) -> &'static str {
        "smart-turn-v3.2"
    }

    fn probability_complete(&mut self, audio: &[f32]) -> Result<f32, String> {
        if audio.is_empty() {
            return Err("There is no audio to judge.".to_string());
        }
        let features = self.front_end.features(audio);
        let input = Tensor::from_shape(&[1, MEL_BANDS, FRAMES], features)
            .map_err(|_| "Could not hand the audio to the turn-completion model.".to_string())?;
        let outputs = self
            .plan
            .run(tvec!(input.into()))
            .map_err(|_| "The turn-completion model failed to run.".to_string())?;
        // The output is called `logits` in the file but is already passed through a sigmoid.
        let probability = outputs
            .first()
            .and_then(|output| output.to_plain_array_view::<f32>().ok())
            .and_then(|values| values.iter().next().copied())
            .ok_or_else(|| "The turn-completion model returned no answer.".to_string())?;
        if !probability.is_finite() {
            return Err("The turn-completion model returned an unusable answer.".to_string());
        }
        Ok(probability.clamp(0.0, 1.0))
    }
}

const NOT_INSTALLED: &str =
    "The turn-completion model is not installed. Run scripts\\Get-SmartTurnModel.ps1.";

/// The turn-completion model if it is installed and loads, otherwise none and the reason.
/// Without one the caller ends a turn on silence alone.
pub fn best_turn_model(
    smart_turn_model: Option<&Path>,
) -> (Option<Box<dyn TurnCompletionModel>>, Option<String>) {
    match smart_turn_model.map(SmartTurn::load) {
        Some(Ok(model)) => (Some(Box::new(model)), None),
        Some(Err(reason)) => (None, Some(reason)),
        None => (None, Some(NOT_INSTALLED.to_string())),
    }
}

/// Turns up to 8 seconds of audio into the 80 x 800 log-mel picture the model was trained on.
///
/// Steps, each matching the reference:
/// 1. Keep the last 8 s; shorter audio gets silence in front, so the speech ends at the end.
/// 2. Shift and scale the 8 s (silence included) to zero mean and unit variance.
/// 3. Mirror 200 samples at each end, then take 25 ms frames every 10 ms under a periodic
///    Hann window and keep the power at each of 201 frequencies. The 801st frame is dropped.
/// 4. Sum the power into 80 triangular bands on the Slaney mel scale, each scaled by
///    2 / its width.
/// 5. Take log10 (of at least 1e-10), raise everything to within 8 of the loudest value,
///    then map with `(x + 4) / 4`.
///
/// The arithmetic is in f64, as in the reference, and the result is f32.
struct LogMel {
    window: [f64; FFT_LEN],
    fft: Fft,
    bands: Vec<MelBand>,
    padded: Vec<f64>,
    frame: Vec<Complex>,
    spectrum: Vec<Complex>,
    power: [f64; BINS],
    features: Vec<f32>,
}

/// One triangular band: weights for consecutive frequencies starting at `first_bin`.
struct MelBand {
    first_bin: usize,
    weights: Vec<f64>,
}

impl LogMel {
    fn new() -> Self {
        let mut window = [0.0; FFT_LEN];
        for (index, value) in window.iter_mut().enumerate() {
            *value = 0.5 - 0.5 * (2.0 * PI * index as f64 / FFT_LEN as f64).cos();
        }
        Self {
            window,
            fft: Fft::new(FFT_LEN),
            bands: mel_bands(),
            padded: vec![0.0; WINDOW_SAMPLES + FFT_LEN],
            frame: vec![Complex::default(); FFT_LEN],
            spectrum: vec![Complex::default(); FFT_LEN],
            power: [0.0; BINS],
            features: vec![0.0; MEL_BANDS * FRAMES],
        }
    }

    /// The picture for `audio`, row by row: `features[band * 800 + frame]`.
    fn features(&mut self, audio: &[f32]) -> &[f32] {
        self.fill_normalised_window(audio);

        let mut loudest = f64::NEG_INFINITY;
        let mut log_mel = vec![0.0f64; MEL_BANDS * FRAMES];
        for frame_index in 0..FRAMES {
            let start = frame_index * HOP;
            for (slot, (sample, weight)) in self
                .frame
                .iter_mut()
                .zip(self.padded[start..start + FFT_LEN].iter().zip(&self.window))
            {
                *slot = Complex {
                    re: sample * weight,
                    im: 0.0,
                };
            }
            self.fft.forward(&self.frame, &mut self.spectrum);
            for (power, bin) in self.power.iter_mut().zip(&self.spectrum) {
                *power = bin.re * bin.re + bin.im * bin.im;
            }
            for (band_index, band) in self.bands.iter().enumerate() {
                let energy: f64 = band
                    .weights
                    .iter()
                    .zip(&self.power[band.first_bin..])
                    .map(|(weight, power)| weight * power)
                    .sum();
                let value = energy.max(MEL_FLOOR).log10();
                loudest = loudest.max(value);
                log_mel[band_index * FRAMES + frame_index] = value;
            }
        }

        let floor = loudest - 8.0;
        for (feature, value) in self.features.iter_mut().zip(&log_mel) {
            *feature = ((value.max(floor) + 4.0) / 4.0) as f32;
        }
        &self.features
    }

    /// Steps 1 to 3a: the last 8 s, silence in front, normalised, with mirrored ends, written
    /// to `self.padded`.
    fn fill_normalised_window(&mut self, audio: &[f32]) {
        let half = FFT_LEN / 2;
        let tail = &audio[audio.len().saturating_sub(WINDOW_SAMPLES)..];
        let silence = WINDOW_SAMPLES - tail.len();
        let window = &mut self.padded[half..half + WINDOW_SAMPLES];
        window[..silence].fill(0.0);
        for (slot, sample) in window[silence..].iter_mut().zip(tail) {
            // A stray NaN from a driver would otherwise poison the whole window.
            *slot = if sample.is_finite() {
                f64::from(*sample)
            } else {
                0.0
            };
        }

        let count = WINDOW_SAMPLES as f64;
        let mean = window.iter().sum::<f64>() / count;
        let variance = window
            .iter()
            .map(|sample| (sample - mean) * (sample - mean))
            .sum::<f64>()
            / count;
        let scale = 1.0 / (variance + VARIANCE_FLOOR).sqrt();
        for sample in window.iter_mut() {
            *sample = (*sample - mean) * scale;
        }

        // Mirror without repeating the end sample, as `numpy.pad(mode="reflect")` does.
        for offset in 0..half {
            self.padded[offset] = self.padded[2 * half - offset];
            let last = half + WINDOW_SAMPLES - 1;
            self.padded[last + 1 + offset] = self.padded[last - 1 - offset];
        }
    }
}

/// Slaney's mel scale: linear below 1 kHz, logarithmic above. (The spectrum display in
/// [`super::spectrum`] uses the other common definition, which this model was not trained on.)
fn hz_to_slaney_mel(hz: f64) -> f64 {
    const LINEAR_HZ_PER_MEL: f64 = 200.0 / 3.0;
    const LOG_START_HZ: f64 = 1000.0;
    if hz < LOG_START_HZ {
        hz / LINEAR_HZ_PER_MEL
    } else {
        LOG_START_HZ / LINEAR_HZ_PER_MEL + (hz / LOG_START_HZ).ln() * 27.0 / 6.4f64.ln()
    }
}

fn slaney_mel_to_hz(mel: f64) -> f64 {
    const LINEAR_HZ_PER_MEL: f64 = 200.0 / 3.0;
    const LOG_START_HZ: f64 = 1000.0;
    const LOG_START_MEL: f64 = LOG_START_HZ / LINEAR_HZ_PER_MEL;
    if mel < LOG_START_MEL {
        mel * LINEAR_HZ_PER_MEL
    } else {
        LOG_START_HZ * ((mel - LOG_START_MEL) * 6.4f64.ln() / 27.0).exp()
    }
}

/// The 80 triangular bands between 0 and 8 kHz, each scaled by 2 / (its width in Hz).
fn mel_bands() -> Vec<MelBand> {
    let nyquist = f64::from(SMART_TURN_SAMPLE_RATE) / 2.0;
    let top_mel = hz_to_slaney_mel(nyquist);
    // Band `b` rises from edge b to edge b + 1 and falls to edge b + 2.
    let edges: Vec<f64> = (0..MEL_BANDS + 2)
        .map(|index| slaney_mel_to_hz(top_mel * index as f64 / (MEL_BANDS + 1) as f64))
        .collect();

    (0..MEL_BANDS)
        .map(|band| {
            let (low, centre, high) = (edges[band], edges[band + 1], edges[band + 2]);
            let scale = 2.0 / (high - low);
            let mut first_bin = None;
            let mut weights = Vec::new();
            for bin in 0..BINS {
                let hz = nyquist * bin as f64 / (BINS - 1) as f64;
                let rising = (hz - low) / (centre - low);
                let falling = (high - hz) / (high - centre);
                let weight = rising.min(falling).max(0.0) * scale;
                if weight > 0.0 {
                    first_bin.get_or_insert(bin);
                    weights.push(weight);
                } else if first_bin.is_some() {
                    break;
                }
            }
            MelBand {
                first_bin: first_bin.unwrap_or(0),
                weights,
            }
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Complex {
    re: f64,
    im: f64,
}

/// A discrete Fourier transform for lengths with small prime factors (400 = 2^4 x 5^2).
/// Split the input into `p` interleaved parts, transform each, and combine; the combining
/// step costs `p` multiplications per output, which is cheap while the factors are small.
struct Fft {
    len: usize,
    factors: Vec<usize>,
    /// `twiddles[k]` is `exp(-2 pi i k / len)`.
    twiddles: Vec<Complex>,
}

impl Fft {
    fn new(len: usize) -> Self {
        let mut factors = Vec::new();
        let mut rest = len;
        let mut factor = 2;
        while rest > 1 {
            while rest.is_multiple_of(factor) {
                factors.push(factor);
                rest /= factor;
            }
            factor += 1;
        }
        // Largest factors first keeps the deep, frequently run levels at radix 2.
        factors.reverse();
        let twiddles = (0..len)
            .map(|index| {
                let angle = -2.0 * PI * index as f64 / len as f64;
                Complex {
                    re: angle.cos(),
                    im: angle.sin(),
                }
            })
            .collect();
        Self {
            len,
            factors,
            twiddles,
        }
    }

    /// `output[k]` is the sum over `n` of `input[n] * exp(-2 pi i k n / len)`. Both slices are
    /// `len` long.
    fn forward(&self, input: &[Complex], output: &mut [Complex]) {
        debug_assert_eq!(input.len(), self.len);
        debug_assert_eq!(output.len(), self.len);
        self.step(input, 0, 1, output, &self.factors);
    }

    /// Transform the `output.len()` samples `input[offset + n * stride]`.
    fn step(
        &self,
        input: &[Complex],
        offset: usize,
        stride: usize,
        output: &mut [Complex],
        factors: &[usize],
    ) {
        let Some((&radix, deeper)) = factors.split_first() else {
            output[0] = input[offset];
            return;
        };
        let len = output.len();
        let part = len / radix;
        for (index, chunk) in output.chunks_exact_mut(part).enumerate() {
            self.step(
                input,
                offset + index * stride,
                stride * radix,
                chunk,
                deeper,
            );
        }

        // Combine: X[k] is the sum over q of exp(-2 pi i q k / len) * part_q[k mod part].
        let twiddle_step = self.len / len;
        let mut column = [Complex::default(); MAX_RADIX];
        for low in 0..part {
            for (q, slot) in column[..radix].iter_mut().enumerate() {
                *slot = output[q * part + low];
            }
            for high in 0..radix {
                let k = high * part + low;
                let mut sum = column[0];
                for (q, value) in column[..radix].iter().enumerate().skip(1) {
                    let twiddle = self.twiddles[(q * k % len) * twiddle_step];
                    sum.re += value.re * twiddle.re - value.im * twiddle.im;
                    sum.im += value.re * twiddle.im + value.im * twiddle.re;
                }
                output[k] = sum;
            }
        }
    }
}

/// Largest prime factor [`Fft`] supports; 400 needs 5.
const MAX_RADIX: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic test audio that Python reproduces bit for bit: uniform noise from a
    /// 32-bit linear congruential generator, loud and quiet in alternating quarter seconds.
    /// Every operation is exact in f32, so both sides start from identical samples.
    fn reference_noise(samples: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..samples)
            .map(|index| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let uniform = (state >> 8) as f32 / 8_388_608.0 - 1.0;
                let gain = if (index / 4_000) % 2 == 0 {
                    0.25
                } else {
                    0.03125
                };
                uniform * gain
            })
            .collect()
    }

    /// A buzzy voice-like sound: twelve harmonics of a pitch gliding from `start_hz` to
    /// `end_hz`, pulsing `syllable_hz` times a second, then `silence` samples of nothing.
    fn voiced(
        samples: usize,
        start_hz: f64,
        end_hz: f64,
        syllable_hz: f64,
        silence: usize,
    ) -> Vec<f32> {
        let mut phase = 0.0f64;
        let mut audio: Vec<f32> = (0..samples)
            .map(|index| {
                let position = index as f64 / samples as f64;
                phase += 2.0 * PI * (start_hz + (end_hz - start_hz) * position) / 16_000.0;
                let envelope = 0.5 - 0.5 * (2.0 * PI * syllable_hz * index as f64 / 16_000.0).cos();
                let buzz: f64 = (1..=12)
                    .map(|harmonic| (phase * f64::from(harmonic)).sin() / f64::from(harmonic))
                    .sum();
                (0.2 * envelope * buzz) as f32
            })
            .collect();
        audio.resize(samples + silence, 0.0);
        audio
    }

    fn features(audio: &[f32]) -> Vec<f32> {
        LogMel::new().features(audio).to_vec()
    }

    fn largest_difference(a: &[f32], b: &[f32]) -> f32 {
        assert_eq!(a.len(), b.len());
        a.iter()
            .zip(b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_transform_matches_the_definition_at_length_400() {
        let fft = Fft::new(FFT_LEN);
        assert_eq!(fft.factors.iter().product::<usize>(), FFT_LEN);
        assert!(fft.factors.iter().all(|factor| *factor <= MAX_RADIX));

        let input: Vec<Complex> = reference_noise(FFT_LEN * 2, 7)
            .chunks(2)
            .map(|pair| Complex {
                re: f64::from(pair[0]),
                im: f64::from(pair[1]),
            })
            .collect();
        let mut output = vec![Complex::default(); FFT_LEN];
        fft.forward(&input, &mut output);

        for (k, got) in output.iter().enumerate() {
            let mut want = Complex::default();
            for (n, value) in input.iter().enumerate() {
                let angle = -2.0 * PI * (k * n % FFT_LEN) as f64 / FFT_LEN as f64;
                want.re += value.re * angle.cos() - value.im * angle.sin();
                want.im += value.re * angle.sin() + value.im * angle.cos();
            }
            assert!(
                (got.re - want.re).abs() < 1e-9 && (got.im - want.im).abs() < 1e-9,
                "bin {k}: {got:?} is not {want:?}"
            );
        }
    }

    #[test]
    fn mel_bands_are_slaney_triangles_scaled_by_their_width() {
        let bands = mel_bands();
        assert_eq!(bands.len(), MEL_BANDS);
        // Below 1 kHz the scale is linear: edges every 8000-Hz-in-mels / 81 * (200 / 3) Hz.
        assert!((hz_to_slaney_mel(1000.0) - 15.0).abs() < 1e-12);
        assert!((slaney_mel_to_hz(hz_to_slaney_mel(4321.0)) - 4321.0).abs() < 1e-9);
        // Values printed by transformers' mel_filter_bank(201, 80, 0, 8000, 16000,
        // norm="slaney", mel_scale="slaney"): [bin, band] of the reference matrix.
        let weight = |bin: usize, band: usize| {
            let band = &bands[band];
            bin.checked_sub(band.first_bin)
                .and_then(|index| band.weights.get(index).copied())
                .unwrap_or(0.0)
        };
        for (bin, band, expected) in REFERENCE_MEL_WEIGHTS {
            let got = weight(bin, band);
            assert!(
                (got - expected).abs() < 1e-9,
                "weight at bin {bin}, band {band}: {got} is not {expected}"
            );
        }
        // No band is empty, and the bands move up in frequency.
        assert!(bands.iter().all(|band| !band.weights.is_empty()));
        assert!(bands
            .windows(2)
            .all(|pair| pair[0].first_bin <= pair[1].first_bin));
    }

    #[test]
    fn features_match_the_reference_feature_extractor() {
        // 3.5 s, so the test also covers the silence put in front of short audio.
        let got = features(&reference_noise(56_000, 20_261_122));
        assert_eq!(got.len(), MEL_BANDS * FRAMES);
        for (band, frame, expected) in REFERENCE_FEATURES {
            let value = got[band * FRAMES + frame];
            assert!(
                (value - expected).abs() < 2e-4,
                "feature at band {band}, frame {frame}: {value} is not {expected}"
            );
        }
    }

    #[test]
    fn only_the_last_eight_seconds_are_used() {
        let long = reference_noise(WINDOW_SAMPLES + 50_000, 3);
        let tail = &long[long.len() - WINDOW_SAMPLES..];
        assert_eq!(features(&long), features(tail));
    }

    #[test]
    fn short_audio_is_treated_as_preceded_by_silence() {
        let speech = reference_noise(24_000, 5);
        let mut padded = vec![0.0; WINDOW_SAMPLES - speech.len()];
        padded.extend_from_slice(&speech);
        assert_eq!(features(&speech), features(&padded));

        // The same sound placed at the start instead is a different picture.
        let mut at_start = speech.clone();
        at_start.resize(WINDOW_SAMPLES, 0.0);
        assert!(largest_difference(&features(&speech), &features(&at_start)) > 0.5);
    }

    #[test]
    fn loudness_does_not_change_the_picture() {
        let audio = reference_noise(40_000, 11);
        let quieter: Vec<f32> = audio.iter().map(|sample| sample * 0.125).collect();
        assert!(largest_difference(&features(&audio), &features(&quieter)) < 1e-3);
    }

    #[test]
    fn silence_and_bad_samples_still_give_a_finite_picture() {
        for audio in [
            vec![0.0; 16_000],
            vec![f32::NAN; 16_000],
            vec![f32::INFINITY; 3],
            vec![0.5],
        ] {
            let picture = features(&audio);
            assert!(picture.iter().all(|value| value.is_finite()));
        }
        // Digital silence is the floor everywhere: (log10(1e-10) + 4) / 4.
        assert!(features(&[0.0; 100])
            .iter()
            .all(|value| (*value + 1.5).abs() < 1e-6));
    }

    #[test]
    fn a_reused_front_end_gives_the_same_answer_as_a_fresh_one() {
        let first = reference_noise(30_000, 1);
        let second = reference_noise(90_000, 2);
        let mut front_end = LogMel::new();
        front_end.features(&first);
        assert_eq!(front_end.features(&second), features(&second).as_slice());
    }

    #[test]
    fn a_missing_model_is_reported_with_the_script_that_installs_it() {
        let error = SmartTurn::load(Path::new("no-such-model.onnx"))
            .err()
            .unwrap();
        assert!(error.contains("Get-SmartTurnModel.ps1"));
        assert!(!error.contains("no-such-model"));

        let (model, reason) = best_turn_model(Some(Path::new("no-such-model.onnx")));
        assert!(model.is_none());
        assert!(reason.unwrap().contains("Get-SmartTurnModel.ps1"));

        let (model, reason) = best_turn_model(None);
        assert!(model.is_none());
        assert!(reason.is_some());
    }

    #[test]
    fn a_file_that_is_not_a_model_is_refused_without_naming_the_path() {
        let path =
            std::env::temp_dir().join(format!("localflow-not-a-model-{}.onnx", std::process::id()));
        std::fs::write(&path, b"this is not a model").unwrap();
        let error = SmartTurn::load(&path).err();
        std::fs::remove_file(&path).ok();

        let error = error.expect("a text file is not a model");
        assert!(!error.contains("localflow-not-a-model"));
    }

    fn installed_model() -> SmartTurn {
        let model = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".localflow-runtime")
            .join("models")
            .join(SMART_TURN_MODEL_FILE);
        SmartTurn::load(&model).expect("model should load")
    }

    /// Needs the real model: `scripts\Get-SmartTurnModel.ps1`, then
    /// `cargo test --release --lib real_smart_turn -- --ignored --nocapture`.
    ///
    /// The expected numbers are what the project's own `inference.py` path returns for the
    /// same sounds (transformers 5.24.0 features, ONNX Runtime 1.32.0, this model file).
    #[test]
    #[ignore = "needs the Smart Turn model in .localflow-runtime/models"]
    fn real_smart_turn_agrees_with_the_reference_runtime() {
        let mut model = installed_model();
        assert_eq!(model.id(), "smart-turn-v3.2");

        for (samples, seed, expected) in REFERENCE_NOISE_PROBABILITIES {
            let got = model
                .probability_complete(&reference_noise(samples, seed))
                .unwrap();
            assert!(
                (got - expected).abs() < 0.02,
                "{samples} samples, seed {seed}: {got} is not {expected}"
            );
        }
        for (samples, start_hz, end_hz, syllable_hz, silence, expected) in
            REFERENCE_VOICED_PROBABILITIES
        {
            let got = model
                .probability_complete(&voiced(samples, start_hz, end_hz, syllable_hz, silence))
                .unwrap();
            assert!(
                (got - expected).abs() < 0.02,
                "voiced {samples}, {start_hz} to {end_hz} Hz: {got} is not {expected}"
            );
        }
    }

    /// The model hears a rising pitch that stops dead as unfinished and a falling pitch
    /// followed by a short silence as finished, as a listener would.
    #[test]
    #[ignore = "needs the Smart Turn model in .localflow-runtime/models"]
    fn real_smart_turn_tells_a_trailing_off_from_a_finish() {
        let mut model = installed_model();
        let unfinished = model
            .probability_complete(&voiced(24_000, 110.0, 180.0, 3.0, 0))
            .unwrap();
        let finished = model
            .probability_complete(&voiced(24_000, 220.0, 110.0, 5.0, 3_200))
            .unwrap();
        assert!(unfinished < 0.2, "rising and cut off scored {unfinished}");
        assert!(finished > 0.8, "falling then silent scored {finished}");
    }

    #[test]
    #[ignore = "needs the Smart Turn model in .localflow-runtime/models"]
    fn real_smart_turn_refuses_empty_audio_and_survives_odd_audio() {
        let mut model = installed_model();
        assert!(model.probability_complete(&[]).is_err());
        for audio in [vec![0.0; 16_000], vec![f32::NAN; 100], vec![0.3; 200_000]] {
            let probability = model.probability_complete(&audio).unwrap();
            assert!((0.0..=1.0).contains(&probability));
        }
    }

    /// `[bin, band, weight]` from the reference filter bank.
    const REFERENCE_MEL_WEIGHTS: [(usize, usize, f64); 12] = [
        (1, 0, 0.024862593984176087),
        (2, 0, 0.0),
        (2, 1, 0.022871772096078023),
        (10, 8, 0.0),
        (25, 22, 0.0),
        (40, 38, 0.01485429994066739),
        (63, 50, 0.009462301431073022),
        (100, 62, 0.006172718498657561),
        (150, 73, 0.00214694392952649),
        (199, 79, 0.00044875902758905584),
        (200, 79, 0.0),
        (5, 40, 0.0),
    ];

    /// `[band, frame, value]` from the reference for `reference_noise(56_000, 20_261_122)`.
    const REFERENCE_FEATURES: [(usize, usize, f32); 16] = [
        (0, 0, 0.385_618_7),
        (79, 0, -0.472_395_54),
        (0, 449, 0.607_216_24),
        (5, 450, 1.102_394_6),
        (40, 451, 1.095_162),
        (79, 452, 1.258_286_2),
        (0, 500, 1.200_793_7),
        (17, 475, 1.085_033_9),
        (33, 501, 1.225_875_1),
        (62, 623, 1.330_762),
        (79, 700, 1.247_423_8),
        (10, 799, 0.743_325_05),
        (45, 799, 0.857_893_94),
        (79, 799, 0.847_254_34),
        (25, 449, 0.098_141_67),
        (25, 200, -0.472_395_54),
    ];

    /// `[samples, seed, probability]` from the reference for `reference_noise(samples, seed)`.
    const REFERENCE_NOISE_PROBABILITIES: [(usize, u32, f32); 4] = [
        (56_000, 20_261_122, 0.9860),
        (16_000, 1, 0.9821),
        (128_000, 2, 0.9808),
        (200_000, 3, 0.9770),
    ];

    /// `[samples, start Hz, end Hz, syllables per second, silent samples, probability]` from
    /// the reference for `voiced(..)`.
    const REFERENCE_VOICED_PROBABILITIES: [(usize, f64, f64, f64, usize, f32); 6] = [
        (24_000, 110.0, 180.0, 3.0, 0, 0.0080),
        (48_000, 120.0, 120.0, 3.0, 3_200, 0.0160),
        (24_000, 180.0, 90.0, 3.0, 3_200, 0.0544),
        (80_000, 180.0, 90.0, 3.0, 0, 0.2720),
        (80_000, 180.0, 90.0, 5.0, 3_200, 0.8506),
        (24_000, 220.0, 110.0, 5.0, 3_200, 0.9443),
    ];
}
