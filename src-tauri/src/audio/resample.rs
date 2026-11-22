//! Band-limited resampling to the 16 kHz mono rate every local ASR engine expects.
//!
//! Microphones usually capture at 44.1 or 48 kHz. Dropping to 16 kHz without first removing
//! everything above the new Nyquist frequency (8 kHz) folds that energy back into the speech
//! band as aliasing. This module resamples with a Hann-windowed sinc low-pass so content above
//! the target Nyquist is attenuated before decimation. It has no dependencies and is pure, so
//! it is unit-tested directly and reused by the benchmark harness.
//!
//! The filter is polyphase: for a rational rate ratio the fractional position of every output
//! sample repeats with a short period (one phase for 48 kHz, 160 for 44.1 kHz), so the kernel
//! is computed once per phase and the inner loop is a plain dot product.

/// Sample rate expected by whisper.cpp and the other local ASR engines.
pub const ASR_SAMPLE_RATE: u32 = 16_000;

/// Zero crossings of the sinc kernel kept on each side of the centre tap.
const SINC_ZERO_CROSSINGS: f64 = 16.0;

/// Fraction of the Nyquist band kept as pass-band. Leaves a transition band so the response is
/// already strongly attenuated at the fold-over frequency.
const PASSBAND_FRACTION: f64 = 0.92;

/// Rate pairs whose reduced ratio needs more phases than this fall back to the nearest of
/// this many evenly spaced phases. Real devices (8, 11.025, 22.05, 32, 44.1, 48, 88.2, 96,
/// 192 kHz) all need at most 160.
const MAX_PHASES: u64 = 640;

/// Resample mono `input` captured at `input_rate` Hz to 16 kHz.
///
/// Returns the input unchanged when it is already 16 kHz. An empty input or a zero rate yields
/// an empty buffer rather than a panic.
pub fn resample_to_16khz(input: &[f32], input_rate: u32) -> Vec<f32> {
    resample(input, input_rate, ASR_SAMPLE_RATE)
}

/// Resample mono `input` from `input_rate` Hz to `output_rate` Hz with a windowed-sinc filter.
pub fn resample(input: &[f32], input_rate: u32, output_rate: u32) -> Vec<f32> {
    if input.is_empty() || input_rate == 0 || output_rate == 0 {
        return Vec::new();
    }
    if input_rate == output_rate {
        return input.to_vec();
    }

    let divisor = greatest_common_divisor(input_rate as u64, output_rate as u64);
    // Output sample `n` sits at input position `n * step_numerator / step_denominator`.
    let step_numerator = input_rate as u64 / divisor;
    let step_denominator = output_rate as u64 / divisor;

    let ratio = input_rate as f64 / output_rate as f64;
    let output_len = ((input.len() as f64) / ratio).round().max(1.0) as usize;

    // Cut-off as a fraction of the input sample rate. When downsampling the limit is the output
    // Nyquist; when upsampling it is the input Nyquist (nothing above it exists to keep).
    let cutoff = 0.5 * PASSBAND_FRACTION * (1.0_f64).min(1.0 / ratio);
    // Kernel half-width in input samples: the sinc has zero crossings every 1 / (2 * cutoff).
    let half_width = (SINC_ZERO_CROSSINGS / (2.0 * cutoff)).ceil();
    let half_taps = half_width as isize;
    // Taps cover input indices `floor(position) - half_taps + 1 ..= floor(position) + half_taps`.
    let taps_per_phase = (2 * half_taps) as usize;

    let phase_count = step_denominator.min(MAX_PHASES) as usize;
    let kernels =
        build_polyphase_kernels(phase_count, taps_per_phase, half_taps, cutoff, half_width);

    let last_index = input.len() as isize - 1;
    let mut output = Vec::with_capacity(output_len);
    for index in 0..output_len as u64 {
        let scaled = index * step_numerator;
        let whole = (scaled / step_denominator) as isize;
        let remainder = scaled % step_denominator;
        // Exact when `phase_count == step_denominator`; otherwise the nearest stored phase.
        let phase = if phase_count as u64 == step_denominator {
            remainder as usize
        } else {
            ((remainder * phase_count as u64) / step_denominator) as usize
        };
        let kernel = &kernels[phase * taps_per_phase..(phase + 1) * taps_per_phase];
        let first = whole - half_taps + 1;

        let value = if first >= 0 && first + taps_per_phase as isize - 1 <= last_index {
            // Interior: a straight dot product the compiler can vectorise.
            let window = &input[first as usize..first as usize + taps_per_phase];
            window
                .iter()
                .zip(kernel)
                .map(|(sample, weight)| sample * weight)
                .sum::<f32>()
        } else {
            // Edges: hold the first and last samples instead of assuming silence so the start
            // and end of the utterance are not faded.
            kernel
                .iter()
                .enumerate()
                .map(|(tap, weight)| {
                    let position = (first + tap as isize).clamp(0, last_index) as usize;
                    input[position] * weight
                })
                .sum::<f32>()
        };

        output.push(value.clamp(-1.0, 1.0));
    }

    output
}

/// One normalised kernel per phase, stored back to back. Each kernel sums to one, which keeps
/// unity gain at DC for every phase.
fn build_polyphase_kernels(
    phase_count: usize,
    taps_per_phase: usize,
    half_taps: isize,
    cutoff: f64,
    half_width: f64,
) -> Vec<f32> {
    let mut kernels = Vec::with_capacity(phase_count * taps_per_phase);

    for phase in 0..phase_count {
        let fraction = phase as f64 / phase_count as f64;
        let weights: Vec<f64> = (0..taps_per_phase)
            .map(|tap| {
                let offset = (tap as isize - half_taps + 1) as f64 - fraction;
                windowed_sinc(offset, cutoff, half_width)
            })
            .collect();
        let sum: f64 = weights.iter().sum();
        let scale = if sum.abs() > f64::EPSILON {
            1.0 / sum
        } else {
            0.0
        };
        kernels.extend(weights.iter().map(|weight| (weight * scale) as f32));
    }

    kernels
}

fn windowed_sinc(offset: f64, cutoff: f64, half_width: f64) -> f64 {
    if offset.abs() > half_width {
        return 0.0;
    }

    let argument = 2.0 * cutoff * offset;
    let sinc = if argument.abs() < 1e-12 {
        1.0
    } else {
        let scaled = std::f64::consts::PI * argument;
        scaled.sin() / scaled
    };
    let window = 0.5 * (1.0 + (std::f64::consts::PI * offset / half_width).cos());

    2.0 * cutoff * sinc * window
}

fn greatest_common_divisor(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(frequency: f64, sample_rate: u32, seconds: f64, amplitude: f32) -> Vec<f32> {
        let count = (sample_rate as f64 * seconds) as usize;
        (0..count)
            .map(|index| {
                let phase =
                    2.0 * std::f64::consts::PI * frequency * index as f64 / sample_rate as f64;
                amplitude * phase.sin() as f32
            })
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
    }

    /// RMS of the steady-state middle of a buffer, skipping filter edge effects.
    fn steady_state_rms(samples: &[f32]) -> f32 {
        let margin = samples.len() / 8;
        rms(&samples[margin..samples.len() - margin])
    }

    #[test]
    fn passes_through_audio_that_is_already_16khz() {
        let input = sine(440.0, 16_000, 0.1, 0.5);
        assert_eq!(resample_to_16khz(&input, 16_000), input);
    }

    #[test]
    fn empty_input_and_zero_rate_do_not_panic() {
        assert!(resample_to_16khz(&[], 48_000).is_empty());
        assert!(resample_to_16khz(&[0.1, 0.2], 0).is_empty());
    }

    #[test]
    fn very_short_input_is_handled_by_the_edge_path() {
        let output = resample_to_16khz(&[0.5, 0.5, 0.5], 48_000);
        assert_eq!(output.len(), 1);
        assert!((output[0] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn output_length_tracks_the_rate_ratio() {
        for rate in [8_000, 22_050, 32_000, 44_100, 48_000, 96_000] {
            let one_second = vec![0.0_f32; rate as usize];
            assert_eq!(resample_to_16khz(&one_second, rate).len(), 16_000, "{rate}");
        }
    }

    #[test]
    fn preserves_speech_band_tones() {
        for rate in [44_100, 48_000] {
            for frequency in [200.0, 1_000.0, 3_400.0, 6_000.0] {
                let input = sine(frequency, rate, 0.5, 0.5);
                let output = resample_to_16khz(&input, rate);
                let gain = steady_state_rms(&output) / steady_state_rms(&input);
                assert!(
                    (gain - 1.0).abs() < 0.03,
                    "{frequency} Hz at {rate} Hz had gain {gain}"
                );
            }
        }
    }

    #[test]
    fn keeps_the_frequency_of_a_tone() {
        // A 1 kHz tone must still cross zero 2000 times a second after resampling; a phase
        // or indexing error in the polyphase table would shift or smear it.
        for rate in [44_100, 48_000] {
            let output = resample_to_16khz(&sine(1_000.0, rate, 1.0, 0.5), rate);
            let middle = &output[2_000..14_000];
            let crossings = middle
                .windows(2)
                .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
                .count();
            // 12,000 samples at 16 kHz is 0.75 s: 1,500 crossings, give or take the ends.
            assert!((1_498..=1_502).contains(&crossings), "{rate}: {crossings}");
        }
    }

    #[test]
    fn rejects_tones_above_the_16khz_nyquist_instead_of_aliasing_them() {
        // 10 kHz and 13 kHz would fold to 6 kHz and 3 kHz, right in the speech band, with a
        // naive decimator. They must be strongly attenuated (better than 60 dB here).
        for rate in [44_100, 48_000] {
            for frequency in [10_000.0, 13_000.0, 20_000.0] {
                let input = sine(frequency, rate, 0.5, 0.5);
                let output = resample_to_16khz(&input, rate);
                let leakage = steady_state_rms(&output) / steady_state_rms(&input);
                assert!(
                    leakage < 0.001,
                    "{frequency} Hz at {rate} Hz leaked {leakage}"
                );
            }
        }
    }

    #[test]
    fn keeps_unity_gain_for_a_constant_signal_including_the_edges() {
        for rate in [44_100, 48_000] {
            let input = vec![0.25_f32; 4_800];
            let output = resample_to_16khz(&input, rate);
            assert!(
                output.iter().all(|sample| (sample - 0.25).abs() < 1e-4),
                "{rate}"
            );
        }
    }

    #[test]
    fn upsamples_low_rate_devices_without_changing_level() {
        let input = sine(1_000.0, 8_000, 0.5, 0.5);
        let output = resample_to_16khz(&input, 8_000);
        let gain = steady_state_rms(&output) / steady_state_rms(&input);
        assert!((gain - 1.0).abs() < 0.03, "gain {gain}");
    }

    #[test]
    fn unusual_rates_fall_back_to_the_nearest_phase_without_distortion() {
        // 47,999 Hz reduces to a ratio with 16,000 phases, far past the table limit.
        let input = sine(1_000.0, 47_999, 0.5, 0.5);
        let output = resample_to_16khz(&input, 47_999);
        let gain = steady_state_rms(&output) / steady_state_rms(&input);
        assert!((gain - 1.0).abs() < 0.03, "gain {gain}");
    }

    #[test]
    fn output_never_exceeds_full_scale() {
        // A full-scale square wave overshoots when band-limited; the result must stay clamped.
        let input: Vec<f32> = (0..48_000)
            .map(|index| if (index / 40) % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        let output = resample_to_16khz(&input, 48_000);
        assert!(output.iter().all(|sample| sample.abs() <= 1.0));
    }

    /// Performance guard, run in release with `cargo test --release resampler_speed -- --ignored
    /// --nocapture`. Thirty seconds of 48 kHz audio must resample in well under 100 ms.
    #[test]
    #[ignore = "timing test; run in release"]
    fn resampler_speed() {
        for rate in [44_100_u32, 48_000] {
            let input = sine(440.0, rate, 30.0, 0.5);
            let started = std::time::Instant::now();
            let output = resample_to_16khz(&input, rate);
            let elapsed = started.elapsed();
            println!("{rate} Hz, 30 s: {} ms", elapsed.as_millis());
            assert_eq!(output.len(), 480_000);
            assert!(elapsed.as_millis() < 100, "{rate}: {elapsed:?}");
        }
    }
}
