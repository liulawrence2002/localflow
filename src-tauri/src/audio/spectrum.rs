//! Real-time spectrum analysis for the voice overlay.
//!
//! Turns the most recent microphone samples into [`BAND_COUNT`] perceptually spaced band
//! magnitudes in `[0, 1]`, plus pitch, voicing, and brightness, so the overlay can draw what
//! the microphone actually hears instead of a synthetic wave.
//!
//! Method: a Hann-windowed radix-2 FFT of roughly the last 40 ms, pooled into mel-spaced
//! triangular bands, converted to decibels, and mapped to `[0, 1]` against a slowly adapting
//! per-band noise floor and a global ceiling. Pitch comes from the autocorrelation obtained
//! by transforming the power spectrum a second time; brightness is the spectral centroid.
//!
//! The analyser is pure and deterministic: it uses no clock (time is counted in samples), no
//! randomness, and allocates nothing after construction. It never logs, and the band values
//! it produces cannot be turned back into speech.

/// Number of spectrum bands sent to the overlay.
pub const BAND_COUNT: usize = 32;
/// Lower edge of the lowest band.
pub const BAND_MIN_HZ: f32 = 70.0;
/// Upper edge of the highest band.
pub const BAND_MAX_HZ: f32 = 8_000.0;

/// Frames quieter than this RMS are treated as silence: all bands zero, trackers frozen.
const SILENCE_RMS: f32 = 1e-4;
/// Voice pitch search range.
const PITCH_MIN_HZ: f32 = 70.0;
const PITCH_MAX_HZ: f32 = 420.0;
/// Range used to normalise pitch to `[0, 1]`; unchanged from the previous estimator so the
/// overlay's meaning of "pitch" stays the same.
const PITCH_NORM_MIN_HZ: f32 = 75.0;
const PITCH_NORM_MAX_HZ: f32 = 420.0;
/// Minimum normalised autocorrelation for a frame to count as voiced.
const VOICED_THRESHOLD: f32 = 0.45;
/// Brightness reported for silence, matching the overlay's resting value.
const RESTING_BRIGHTNESS: f32 = 0.35;

// Noise floor tracker, in dB re full scale.
const FLOOR_START_DB: f32 = -72.0;
const FLOOR_MIN_DB: f32 = -90.0;
const FLOOR_MAX_DB: f32 = -54.0;
/// How fast the floor follows a quieter reading (fraction of the gap per frame).
const FLOOR_FALL_RATE: f32 = 0.25;
/// How fast the floor creeps up under sustained sound, in dB per second.
const FLOOR_RISE_DB_PER_SEC: f32 = 1.0;
/// A band must exceed its floor by this much before it shows at all.
const FLOOR_MARGIN_DB: f32 = 6.0;

// Ceiling (automatic gain) tracker, in dB re full scale.
const CEILING_START_DB: f32 = -24.0;
const CEILING_MIN_DB: f32 = -36.0;
const CEILING_MAX_DB: f32 = -6.0;
const CEILING_RELEASE_DB_PER_SEC: f32 = 2.0;
/// The displayed range never shrinks below this, which prevents gain pumping.
const MIN_RANGE_DB: f32 = 30.0;

/// Spectral tilt compensation: speech energy falls with frequency, so higher bands are
/// lifted by this many dB per octave above the pivot, up to the cap.
const TILT_DB_PER_OCTAVE: f32 = 3.0;
const TILT_PIVOT_HZ: f32 = 500.0;
const TILT_MAX_DB: f32 = 12.0;

/// One analysed frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectrumFrame {
    /// Band magnitudes in `[0, 1]`, low to high frequency.
    pub bands: [f32; BAND_COUNT],
    /// Fundamental frequency when the frame is voiced.
    pub pitch_hz: Option<f32>,
    /// Pitch normalised to `[0, 1]` over the voice range; `0.5` when unvoiced.
    pub pitch: f32,
    /// How periodic the frame is, `[0, 1]`.
    pub voicing: f32,
    /// Spectral centroid mapped to `[0, 1]`.
    pub brightness: f32,
    /// RMS of the analysed frame after DC removal.
    pub rms: f32,
}

impl SpectrumFrame {
    fn silent() -> Self {
        Self {
            bands: [0.0; BAND_COUNT],
            pitch_hz: None,
            pitch: 0.5,
            voicing: 0.0,
            brightness: RESTING_BRIGHTNESS,
            rms: 0.0,
        }
    }
}

/// HTK mel scale.
pub fn hz_to_mel(hz: f32) -> f32 {
    2595.0 * (1.0 + hz.max(0.0) / 700.0).log10()
}

pub fn mel_to_hz(mel: f32) -> f32 {
    700.0 * (10f32.powf(mel / 2595.0) - 1.0)
}

/// FFT length for a device rate: the smallest power of two covering about 40 ms, kept
/// between 256 and 4096. That is 2048 at 44.1 and 48 kHz (about 23 Hz per bin).
pub fn frame_len_for_rate(sample_rate: u32) -> usize {
    let target = (sample_rate as f32 * 0.040).ceil().max(1.0) as usize;
    target.next_power_of_two().clamp(256, 4096)
}

/// Quantise bands to bytes for a compact event payload.
pub fn quantize_bands(bands: &[f32; BAND_COUNT]) -> [u8; BAND_COUNT] {
    let mut quantized = [0u8; BAND_COUNT];
    for (output, value) in quantized.iter_mut().zip(bands.iter()) {
        let unit = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        *output = (unit * 255.0).round() as u8;
    }
    quantized
}

/// Sparse triangular weights of one band over FFT bins.
struct BandFilter {
    first_bin: usize,
    weights: Vec<f32>,
    center_hz: f32,
    tilt_db: f32,
    /// False when the band lies above what this sample rate can represent.
    active: bool,
}

/// Highest rate analysed directly. Faster devices are averaged down to at most this, so the
/// frame always covers about 40 ms and the pitch search always reaches down to 70 Hz.
const MAX_ANALYSIS_RATE: u32 = 48_000;

pub struct SpectrumAnalyzer {
    /// Rate of the samples in the ring, after any decimation.
    sample_rate: f32,
    /// Rate of the samples passed to `push`.
    input_rate: f32,
    /// Input samples averaged into each stored sample (1 at 48 kHz and below).
    decimation: usize,
    decimation_sum: f32,
    decimation_count: usize,
    frame_len: usize,
    /// Circular buffer of the newest `frame_len` samples.
    ring: Vec<f32>,
    ring_pos: usize,
    /// Samples pushed since the last `analyze`, used as the frame's time step.
    pending: usize,
    window: Vec<f32>,
    window_sum: f32,
    /// Autocorrelation of the window, normalised to 1 at lag 0, to undo its taper on pitch.
    window_autocorr: Vec<f32>,
    cos_table: Vec<f32>,
    sin_table: Vec<f32>,
    bit_reverse: Vec<usize>,
    real: Vec<f32>,
    imag: Vec<f32>,
    power: Vec<f32>,
    filters: Vec<BandFilter>,
    floor_db: [f32; BAND_COUNT],
    ceiling_db: f32,
}

impl SpectrumAnalyzer {
    /// Build an analyser for a capture rate. A rate of zero gives an inert analyser that
    /// always returns silence.
    pub fn new(input_rate: u32) -> Self {
        let decimation = (input_rate.div_ceil(MAX_ANALYSIS_RATE)).max(1) as usize;
        let sample_rate = input_rate / decimation as u32;
        let frame_len = frame_len_for_rate(sample_rate);
        let window: Vec<f32> = (0..frame_len)
            .map(|index| {
                let phase = 2.0 * std::f32::consts::PI * index as f32 / frame_len as f32;
                0.5 - 0.5 * phase.cos()
            })
            .collect();
        let window_sum = window.iter().sum::<f32>().max(f32::EPSILON);

        let half = frame_len / 2;
        let (cos_table, sin_table): (Vec<f32>, Vec<f32>) = (0..half)
            .map(|index| {
                let angle = -2.0 * std::f64::consts::PI * index as f64 / frame_len as f64;
                (angle.cos() as f32, angle.sin() as f32)
            })
            .unzip();
        let bits = frame_len.trailing_zeros();
        let bit_reverse = (0..frame_len)
            .map(|index| index.reverse_bits() >> (usize::BITS - bits))
            .collect();

        let mut analyzer = Self {
            sample_rate: sample_rate as f32,
            input_rate: input_rate as f32,
            decimation,
            decimation_sum: 0.0,
            decimation_count: 0,
            frame_len,
            ring: vec![0.0; frame_len],
            ring_pos: 0,
            pending: 0,
            window,
            window_sum,
            window_autocorr: vec![1.0; frame_len],
            cos_table,
            sin_table,
            bit_reverse,
            real: vec![0.0; frame_len],
            imag: vec![0.0; frame_len],
            power: vec![0.0; half + 1],
            filters: Vec::new(),
            floor_db: [FLOOR_START_DB; BAND_COUNT],
            ceiling_db: CEILING_START_DB,
        };
        analyzer.filters = analyzer.build_filters();
        analyzer.window_autocorr = analyzer.compute_window_autocorr();
        analyzer
    }

    pub fn frame_len(&self) -> usize {
        self.frame_len
    }

    /// Centre frequency of a band, the same at every sample rate.
    pub fn band_center_hz(&self, band: usize) -> f32 {
        self.filters
            .get(band)
            .map_or(0.0, |filter| filter.center_hz)
    }

    /// Append newly captured mono samples. Non-finite samples are stored as silence.
    pub fn push(&mut self, samples: &[f32]) {
        for sample in samples {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            // Above 48 kHz, average groups of samples. A box average is a crude low-pass,
            // which is enough here: everything of interest lies below 8 kHz.
            self.decimation_sum += sample;
            self.decimation_count += 1;
            if self.decimation_count < self.decimation {
                continue;
            }
            self.ring[self.ring_pos] = self.decimation_sum / self.decimation as f32;
            self.ring_pos = (self.ring_pos + 1) % self.frame_len;
            self.decimation_sum = 0.0;
            self.decimation_count = 0;
        }
        self.pending = self.pending.saturating_add(samples.len());
    }

    /// Forget all audio and adaptation state, as for a new recording.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.ring_pos = 0;
        self.pending = 0;
        self.decimation_sum = 0.0;
        self.decimation_count = 0;
        self.floor_db = [FLOOR_START_DB; BAND_COUNT];
        self.ceiling_db = CEILING_START_DB;
    }

    /// Analyse the newest `frame_len` samples.
    pub fn analyze(&mut self) -> SpectrumFrame {
        let elapsed_secs = if self.input_rate > 0.0 {
            // Bounded so a long gap between frames cannot move the trackers in one jump.
            (self.pending as f32 / self.input_rate).min(0.25)
        } else {
            0.0
        };
        self.pending = 0;
        if self.sample_rate <= 0.0 {
            return SpectrumFrame::silent();
        }

        // Unroll the ring oldest-first and remove any DC offset.
        let frame_len = self.frame_len;
        for index in 0..frame_len {
            self.real[index] = self.ring[(self.ring_pos + index) % frame_len];
        }
        let mean = self.real.iter().sum::<f32>() / frame_len as f32;
        let mut energy = 0.0f32;
        for sample in self.real.iter_mut() {
            *sample -= mean;
            energy += *sample * *sample;
        }
        let rms = (energy / frame_len as f32).sqrt();
        if rms < SILENCE_RMS {
            return SpectrumFrame::silent();
        }

        for (sample, weight) in self.real.iter_mut().zip(self.window.iter()) {
            *sample *= weight;
        }
        self.imag.fill(0.0);
        self.transform();

        // One-sided power, scaled so a full-scale sine reads 0 dB.
        let scale = 4.0 / (self.window_sum * self.window_sum);
        for (bin, power) in self.power.iter_mut().enumerate() {
            *power = (self.real[bin] * self.real[bin] + self.imag[bin] * self.imag[bin]) * scale;
        }

        let bands = self.map_bands(elapsed_secs);
        let brightness = self.brightness();
        let (pitch_hz, voicing) = self.pitch(rms);

        SpectrumFrame {
            bands,
            pitch_hz,
            pitch: pitch_hz.map_or(0.5, |hz| {
                ((hz - PITCH_NORM_MIN_HZ) / (PITCH_NORM_MAX_HZ - PITCH_NORM_MIN_HZ)).clamp(0.0, 1.0)
            }),
            voicing,
            brightness,
            rms,
        }
    }

    fn bin_hz(&self) -> f32 {
        self.sample_rate / self.frame_len as f32
    }

    fn build_filters(&self) -> Vec<BandFilter> {
        let bin_hz = self.bin_hz().max(f32::EPSILON);
        let bins = self.frame_len / 2 + 1;
        let mel_min = hz_to_mel(BAND_MIN_HZ);
        let mel_max = hz_to_mel(BAND_MAX_HZ);
        let edge = |index: usize| {
            mel_to_hz(mel_min + (mel_max - mel_min) * index as f32 / (BAND_COUNT + 1) as f32)
        };
        // Slightly below Nyquist: the top of the spectrum is unreliable after anti-aliasing.
        let usable_hz = self.sample_rate * 0.47;

        (0..BAND_COUNT)
            .map(|band| {
                let (low, center, high) = (edge(band), edge(band + 1), edge(band + 2));
                let tilt_db =
                    (TILT_DB_PER_OCTAVE * (center / TILT_PIVOT_HZ).log2()).clamp(0.0, TILT_MAX_DB);
                let active = self.sample_rate > 0.0 && high <= usable_hz;

                let first_bin = ((low / bin_hz).ceil() as usize).min(bins - 1);
                let last_bin = ((high / bin_hz).floor() as usize).min(bins - 1);
                let mut weights: Vec<f32> = (first_bin..=last_bin.max(first_bin))
                    .map(|bin| {
                        let hz = bin as f32 * bin_hz;
                        if hz <= center {
                            (hz - low) / (center - low).max(f32::EPSILON)
                        } else {
                            (high - hz) / (high - center).max(f32::EPSILON)
                        }
                        .max(0.0)
                    })
                    .collect();

                let total: f32 = weights.iter().sum();
                let (first_bin, weights) = if total > f32::EPSILON {
                    // Area-normalised so a flat spectrum pools to a flat band response.
                    weights.iter_mut().for_each(|weight| *weight /= total);
                    (first_bin, weights)
                } else {
                    // The band is narrower than one bin: read the bin nearest its centre.
                    let nearest = ((center / bin_hz).round() as usize).min(bins - 1);
                    (nearest, vec![1.0])
                };

                BandFilter {
                    first_bin,
                    weights,
                    center_hz: center,
                    tilt_db,
                    active,
                }
            })
            .collect()
    }

    /// Autocorrelation of the analysis window, so the pitch estimate can divide out the
    /// taper the window imposes on every lag.
    fn compute_window_autocorr(&mut self) -> Vec<f32> {
        self.real.copy_from_slice(&self.window);
        self.imag.fill(0.0);
        self.transform();
        self.autocorrelate_from_spectrum();
        let zero_lag = self.real[0].max(f32::EPSILON);
        let normalised = self
            .real
            .iter()
            .map(|value| (value / zero_lag).max(1e-4))
            .collect();
        self.real.fill(0.0);
        self.imag.fill(0.0);
        normalised
    }

    /// Replace the spectrum in `real`/`imag` with its autocorrelation in `real`
    /// (Wiener-Khinchin: transform the power spectrum; it is real and even, so a forward
    /// transform gives the same result as an inverse one up to scale).
    fn autocorrelate_from_spectrum(&mut self) {
        for index in 0..self.frame_len {
            self.real[index] =
                self.real[index] * self.real[index] + self.imag[index] * self.imag[index];
        }
        self.imag.fill(0.0);
        self.transform();
    }

    /// In-place iterative radix-2 FFT over `real`/`imag`.
    fn transform(&mut self) {
        let len = self.frame_len;
        for index in 0..len {
            let swapped = self.bit_reverse[index];
            if swapped > index {
                self.real.swap(index, swapped);
                self.imag.swap(index, swapped);
            }
        }

        let mut size = 2;
        while size <= len {
            let half = size / 2;
            let stride = len / size;
            for start in (0..len).step_by(size) {
                for offset in 0..half {
                    let cos = self.cos_table[offset * stride];
                    let sin = self.sin_table[offset * stride];
                    let even = start + offset;
                    let odd = even + half;
                    let odd_real = self.real[odd] * cos - self.imag[odd] * sin;
                    let odd_imag = self.real[odd] * sin + self.imag[odd] * cos;
                    self.real[odd] = self.real[even] - odd_real;
                    self.imag[odd] = self.imag[even] - odd_imag;
                    self.real[even] += odd_real;
                    self.imag[even] += odd_imag;
                }
            }
            size *= 2;
        }
    }

    /// Pool power into bands, convert to tilted dB, and map to `[0, 1]`.
    fn map_bands(&mut self, elapsed_secs: f32) -> [f32; BAND_COUNT] {
        let mut level_db = [f32::NEG_INFINITY; BAND_COUNT];
        let mut loudest_db = f32::NEG_INFINITY;

        for (band, filter) in self.filters.iter().enumerate() {
            if !filter.active {
                continue;
            }
            let pooled: f32 = filter
                .weights
                .iter()
                .enumerate()
                .map(|(offset, weight)| weight * self.power[filter.first_bin + offset])
                .sum();
            let db = 10.0 * (pooled + 1e-12).log10() + filter.tilt_db;
            level_db[band] = db;
            loudest_db = loudest_db.max(db);

            // The floor drops quickly to a quieter reading and creeps up under sustained
            // sound, so steady background noise sinks out of view while speech stays.
            let floor = &mut self.floor_db[band];
            if db < *floor {
                *floor += FLOOR_FALL_RATE * (db - *floor);
            } else {
                *floor += FLOOR_RISE_DB_PER_SEC * elapsed_secs;
            }
            *floor = floor.clamp(FLOOR_MIN_DB, FLOOR_MAX_DB);
        }

        // Instant attack, slow release: loud speech sets the top of the display at once and
        // the gain recovers gently, never by more than 2 dB a second.
        self.ceiling_db = (self.ceiling_db - CEILING_RELEASE_DB_PER_SEC * elapsed_secs)
            .max(loudest_db)
            .clamp(CEILING_MIN_DB, CEILING_MAX_DB);

        let mut bands = [0.0f32; BAND_COUNT];
        for band in 0..BAND_COUNT {
            if !level_db[band].is_finite() {
                continue;
            }
            let low = self.floor_db[band] + FLOOR_MARGIN_DB;
            let range = (self.ceiling_db - low).max(MIN_RANGE_DB);
            bands[band] = ((level_db[band] - low) / range).clamp(0.0, 1.0);
        }
        bands
    }

    /// Spectral centroid over the band range, on untilted power, as `[0, 1]`.
    fn brightness(&self) -> f32 {
        let bin_hz = self.bin_hz();
        let first = (BAND_MIN_HZ / bin_hz).ceil() as usize;
        let last = ((BAND_MAX_HZ / bin_hz).floor() as usize).min(self.power.len() - 1);
        let (mut weighted, mut total) = (0.0f64, 0.0f64);
        for bin in first..=last.max(first).min(self.power.len() - 1) {
            let power = self.power[bin] as f64;
            weighted += power * (bin as f64 * bin_hz as f64);
            total += power;
        }
        if total <= 1e-12 {
            return RESTING_BRIGHTNESS;
        }
        let centroid_hz = (weighted / total) as f32;
        // 250 Hz maps to 0 and 4 kHz to 1, four octaves apart.
        ((centroid_hz / 250.0).log2() / 4.0).clamp(0.0, 1.0)
    }

    /// Fundamental frequency and voicing from the autocorrelation of the frame.
    fn pitch(&mut self, rms: f32) -> (Option<f32>, f32) {
        self.autocorrelate_from_spectrum();
        let zero_lag = self.real[0];
        if zero_lag <= f32::EPSILON {
            return (None, 0.0);
        }

        let min_lag = ((self.sample_rate / PITCH_MAX_HZ).floor() as usize).max(2);
        let max_lag = ((self.sample_rate / PITCH_MIN_HZ).ceil() as usize).min(self.frame_len / 3);
        if max_lag <= min_lag + 2 {
            return (None, 0.0);
        }

        let normalised = |analyzer: &Self, lag: usize| {
            analyzer.real[lag] / zero_lag / analyzer.window_autocorr[lag]
        };

        let mut best = f32::NEG_INFINITY;
        for lag in min_lag..=max_lag {
            best = best.max(normalised(self, lag));
        }
        let voicing = best.clamp(0.0, 1.0);
        if best < VOICED_THRESHOLD || rms < SILENCE_RMS * 20.0 {
            return (None, voicing);
        }

        // The true period and its multiples all correlate strongly. Take the shortest lag
        // that is nearly as good as the best, then climb to its local peak, so the estimate
        // does not drop an octave.
        let mut lag = (min_lag..=max_lag)
            .find(|lag| normalised(self, *lag) >= 0.85 * best)
            .unwrap_or(min_lag);
        while lag < max_lag && normalised(self, lag + 1) > normalised(self, lag) {
            lag += 1;
        }
        // A real period shows as a peak inside the search range. If the best correlation
        // sits on the boundary it is the slope of something outside the voice range (mains
        // hum below it, a whistle above it), not a pitch.
        if lag <= min_lag || lag >= max_lag {
            return (None, voicing);
        }

        // Parabolic interpolation around the peak for sub-sample resolution.
        let refined = if lag > min_lag && lag < max_lag {
            let (left, mid, right) = (
                normalised(self, lag - 1),
                normalised(self, lag),
                normalised(self, lag + 1),
            );
            let curvature = left - 2.0 * mid + right;
            if curvature.abs() > f32::EPSILON {
                lag as f32 + (0.5 * (left - right) / curvature).clamp(-0.5, 0.5)
            } else {
                lag as f32
            }
        } else {
            lag as f32
        };

        (Some(self.sample_rate / refined), voicing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    fn sine(frequency: f32, amplitude: f32, rate: u32, count: usize) -> Vec<f32> {
        (0..count)
            .map(|index| {
                let phase =
                    2.0 * std::f64::consts::PI * frequency as f64 * index as f64 / rate as f64;
                amplitude * phase.sin() as f32
            })
            .collect()
    }

    fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b.iter()).map(|(x, y)| x + y).collect()
    }

    /// Deterministic white noise in `[-amplitude, amplitude]`.
    fn noise(amplitude: f32, count: usize, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..count)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    /// Feed `signal` in hops of about 27 ms, as the meter thread does, and return the last
    /// frame.
    fn run(analyzer: &mut SpectrumAnalyzer, signal: &[f32], rate: u32) -> SpectrumFrame {
        let hop = (rate as usize * 27 / 1000).max(1);
        let mut frame = SpectrumFrame::silent();
        for chunk in signal.chunks(hop) {
            analyzer.push(chunk);
            frame = analyzer.analyze();
        }
        frame
    }

    fn argmax(bands: &[f32; BAND_COUNT]) -> usize {
        bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(index, _)| index)
            .unwrap()
    }

    #[test]
    fn frame_lengths_cover_about_forty_milliseconds() {
        assert_eq!(frame_len_for_rate(8_000), 512);
        assert_eq!(frame_len_for_rate(16_000), 1_024);
        assert_eq!(frame_len_for_rate(44_100), 2_048);
        assert_eq!(frame_len_for_rate(48_000), 2_048);
        // The analyser itself averages rates above 48 kHz down before framing.
        assert_eq!(frame_len_for_rate(96_000), 4_096);
        assert_eq!(frame_len_for_rate(0), 256);
    }

    #[test]
    fn mel_conversion_round_trips_and_band_centres_rise() {
        for hz in [70.0, 440.0, 1_000.0, 8_000.0] {
            assert!((mel_to_hz(hz_to_mel(hz)) - hz).abs() < hz * 1e-3);
        }
        let analyzer = SpectrumAnalyzer::new(RATE);
        let centres: Vec<f32> = (0..BAND_COUNT)
            .map(|band| analyzer.band_center_hz(band))
            .collect();
        assert!(centres.windows(2).all(|pair| pair[1] > pair[0]));
        assert!(centres[0] > BAND_MIN_HZ && centres[BAND_COUNT - 1] < BAND_MAX_HZ);
    }

    #[test]
    fn a_tone_at_a_band_centre_peaks_in_that_band() {
        for band in [2, 10, 20, 30] {
            let mut analyzer = SpectrumAnalyzer::new(RATE);
            let frequency = analyzer.band_center_hz(band);
            let frame = run(&mut analyzer, &sine(frequency, 0.1, RATE, 24_000), RATE);

            assert_eq!(argmax(&frame.bands), band, "{frequency} Hz");
            assert!(
                frame.bands[band] >= 0.8,
                "band {band}: {}",
                frame.bands[band]
            );
        }
    }

    #[test]
    fn two_tones_resolve_into_two_separate_peaks() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let low_band = 5;
        let high_band = 20;
        let signal = add(
            &sine(analyzer.band_center_hz(low_band), 0.1, RATE, 24_000),
            &sine(analyzer.band_center_hz(high_band), 0.1, RATE, 24_000),
        );

        let frame = run(&mut analyzer, &signal, RATE);

        let valley = frame.bands[low_band + 3..high_band - 2]
            .iter()
            .copied()
            .fold(f32::INFINITY, f32::min);
        assert!(frame.bands[low_band] >= 0.6, "{:?}", frame.bands);
        assert!(frame.bands[high_band] >= 0.6, "{:?}", frame.bands);
        assert!(frame.bands[low_band].min(frame.bands[high_band]) - valley >= 0.25);
    }

    #[test]
    fn silence_and_near_silence_give_exactly_zero() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);

        let silent = run(&mut analyzer, &vec![0.0; 24_000], RATE);
        let faint = run(&mut analyzer, &noise(1e-5, 24_000, 7), RATE);

        for frame in [silent, faint] {
            assert!(frame.bands.iter().all(|band| *band == 0.0));
            assert_eq!(frame.pitch_hz, None);
            assert_eq!(frame.voicing, 0.0);
            assert_eq!(frame.pitch, 0.5);
        }
    }

    #[test]
    fn louder_tones_never_read_lower() {
        let mut previous = 0.0;
        for amplitude in [0.001, 0.003, 0.01, 0.03, 0.1] {
            let mut analyzer = SpectrumAnalyzer::new(RATE);
            let frame = run(&mut analyzer, &sine(1_000.0, amplitude, RATE, 24_000), RATE);
            let peak = frame.bands[argmax(&frame.bands)];
            assert!(peak >= previous, "{amplitude}: {peak} < {previous}");
            previous = peak;
        }
        assert!(previous >= 0.8);
    }

    #[test]
    fn white_noise_shows_across_the_whole_range_and_rises_with_the_tilt() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let mut average = [0.0f32; BAND_COUNT];
        let signal = noise(0.1, RATE as usize * 2, 99);
        let hop = RATE as usize * 27 / 1000;
        let mut frames = 0;
        for chunk in signal.chunks(hop).skip(4) {
            analyzer.push(chunk);
            let frame = analyzer.analyze();
            for (sum, band) in average.iter_mut().zip(frame.bands.iter()) {
                *sum += band;
            }
            frames += 1;
        }
        average.iter_mut().for_each(|sum| *sum /= frames as f32);

        assert!(average.iter().all(|band| *band > 0.2), "{average:?}");
        // Flat noise plus an upward tilt: the top third reads higher than the bottom third.
        let low: f32 = average[..10].iter().sum::<f32>() / 10.0;
        let high: f32 = average[22..].iter().sum::<f32>() / 10.0;
        assert!(high > low, "low {low}, high {high}");
    }

    #[test]
    fn a_tone_lands_in_the_same_band_at_every_sample_rate() {
        let mut expected = None;
        for rate in [16_000, 44_100, 48_000, 96_000, 192_000] {
            let mut analyzer = SpectrumAnalyzer::new(rate);
            let frame = run(
                &mut analyzer,
                &sine(1_000.0, 0.1, rate, rate as usize / 2),
                rate,
            );
            let band = argmax(&frame.bands);
            assert_eq!(*expected.get_or_insert(band), band, "{rate} Hz");
        }
    }

    #[test]
    fn bands_above_the_usable_range_stay_dark_on_low_rate_devices() {
        let mut analyzer = SpectrumAnalyzer::new(8_000);
        let frame = run(&mut analyzer, &noise(0.2, 8_000, 3), 8_000);

        // 8 kHz capture cannot represent anything above about 3.7 kHz.
        let first_dark = (0..BAND_COUNT)
            .find(|band| analyzer.band_center_hz(*band) > 3_700.0)
            .unwrap();
        assert!(frame.bands[first_dark..].iter().all(|band| *band == 0.0));
        assert!(frame.bands[..first_dark - 2].iter().any(|band| *band > 0.2));
    }

    #[test]
    fn output_is_always_finite_and_within_range_even_for_bad_input() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        analyzer.push(&[]);
        analyzer.push(&[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 4.0, -4.0]);
        let mut frames = vec![analyzer.analyze()];
        frames.push(run(&mut analyzer, &noise(4.0, 24_000, 5), RATE));
        frames.push(run(&mut analyzer, &sine(300.0, 3.0, RATE, 24_000), RATE));

        for frame in frames {
            assert!(frame.bands.iter().all(|band| (0.0..=1.0).contains(band)));
            assert!((0.0..=1.0).contains(&frame.voicing));
            assert!((0.0..=1.0).contains(&frame.pitch));
            assert!((0.0..=1.0).contains(&frame.brightness));
            assert!(frame.rms.is_finite());
        }
    }

    #[test]
    fn a_zero_sample_rate_is_inert() {
        let mut analyzer = SpectrumAnalyzer::new(0);
        analyzer.push(&[0.5; 1_000]);
        assert_eq!(analyzer.analyze(), SpectrumFrame::silent());
    }

    #[test]
    fn analysis_is_deterministic_and_reset_restores_the_initial_state() {
        let signal = add(&sine(220.0, 0.2, RATE, 24_000), &noise(0.02, 24_000, 11));
        let mut first = SpectrumAnalyzer::new(RATE);
        let mut second = SpectrumAnalyzer::new(RATE);

        let a = run(&mut first, &signal, RATE);
        let b = run(&mut second, &signal, RATE);
        assert_eq!(a, b);

        first.reset();
        assert_eq!(run(&mut first, &signal, RATE), a);
    }

    #[test]
    fn a_dc_offset_does_not_change_the_picture() {
        let tone = sine(800.0, 0.1, RATE, 24_000);
        let shifted: Vec<f32> = tone.iter().map(|sample| sample + 0.2).collect();
        let mut plain = SpectrumAnalyzer::new(RATE);
        let mut offset = SpectrumAnalyzer::new(RATE);

        let a = run(&mut plain, &tone, RATE);
        let b = run(&mut offset, &shifted, RATE);

        for (x, y) in a.bands.iter().zip(b.bands.iter()) {
            assert!((x - y).abs() <= 0.02, "{x} vs {y}");
        }
    }

    /// A voice-like harmonic stack with a falling spectrum.
    fn voiced(fundamental: f32, rate: u32, count: usize, fundamental_gain: f32) -> Vec<f32> {
        let mut signal = vec![0.0f32; count];
        for harmonic in 1..=8 {
            let gain = if harmonic == 1 {
                fundamental_gain
            } else {
                0.12 / harmonic as f32
            };
            let partial = sine(fundamental * harmonic as f32, gain, rate, count);
            signal = add(&signal, &partial);
        }
        signal
    }

    #[test]
    fn pitch_follows_the_fundamental_of_a_harmonic_voice() {
        for fundamental in [110.0, 165.0, 220.0, 330.0] {
            let mut analyzer = SpectrumAnalyzer::new(RATE);
            let frame = run(
                &mut analyzer,
                &voiced(fundamental, RATE, 24_000, 0.12),
                RATE,
            );

            let pitch = frame.pitch_hz.expect("voiced frame");
            assert!(
                (pitch - fundamental).abs() <= fundamental * 0.02,
                "{fundamental} Hz read as {pitch}"
            );
            assert!(frame.voicing > 0.8);
        }
    }

    #[test]
    fn a_weak_fundamental_is_not_reported_an_octave_low() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let frame = run(&mut analyzer, &voiced(220.0, RATE, 24_000, 0.01), RATE);

        let pitch = frame.pitch_hz.expect("voiced frame");
        assert!((pitch - 220.0).abs() <= 6.0, "read as {pitch}");
    }

    #[test]
    fn noise_is_unvoiced_and_higher_voices_read_as_higher_pitch() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let hiss = run(&mut analyzer, &noise(0.1, 24_000, 21), RATE);
        assert_eq!(hiss.pitch_hz, None);
        assert!(hiss.voicing < VOICED_THRESHOLD);

        let mut low = SpectrumAnalyzer::new(RATE);
        let mut high = SpectrumAnalyzer::new(RATE);
        let low_frame = run(&mut low, &voiced(110.0, RATE, 24_000, 0.12), RATE);
        let high_frame = run(&mut high, &voiced(260.0, RATE, 24_000, 0.12), RATE);
        assert!(high_frame.pitch > low_frame.pitch + 0.2);
    }

    #[test]
    fn mains_hum_is_not_reported_as_a_voice_pitch() {
        for hum in [50.0, 60.0] {
            let mut analyzer = SpectrumAnalyzer::new(RATE);
            let frame = run(&mut analyzer, &sine(hum, 0.2, RATE, 24_000), RATE);
            assert_eq!(frame.pitch_hz, None, "{hum} Hz hum");
            assert_eq!(frame.pitch, 0.5);
        }
    }

    #[test]
    fn very_high_sample_rates_are_averaged_down_and_still_track_low_voices() {
        for rate in [96_000, 192_000, 384_000] {
            let mut analyzer = SpectrumAnalyzer::new(rate);
            // The frame still covers about 40 ms whatever the device rate.
            assert_eq!(analyzer.frame_len(), 2_048, "{rate} Hz");

            let frame = run(
                &mut analyzer,
                &voiced(110.0, rate, rate as usize / 2, 0.12),
                rate,
            );
            let pitch = frame.pitch_hz.expect("voiced frame");
            assert!((pitch - 110.0).abs() <= 3.0, "{rate} Hz read as {pitch}");
        }

        // The spectrum picture is the same as at 48 kHz.
        let mut reference = SpectrumAnalyzer::new(RATE);
        let mut fast = SpectrumAnalyzer::new(192_000);
        let a = run(&mut reference, &sine(1_000.0, 0.1, RATE, 24_000), RATE);
        let b = run(&mut fast, &sine(1_000.0, 0.1, 192_000, 96_000), 192_000);
        assert_eq!(argmax(&a.bands), argmax(&b.bands));
    }

    #[test]
    fn unusual_device_rates_never_panic() {
        for rate in [1, 4_000, 11_025, 22_050, 32_000, 88_200, 176_400, 768_000] {
            let mut analyzer = SpectrumAnalyzer::new(rate);
            let frame = run(&mut analyzer, &noise(0.3, 4_096, rate), rate);
            assert!(
                frame.bands.iter().all(|band| (0.0..=1.0).contains(band)),
                "{rate} Hz"
            );
        }
    }

    #[test]
    fn a_long_sustained_vowel_dims_a_little_but_never_disappears() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let vowel = voiced(140.0, RATE, RATE as usize * 20, 0.12);
        let early = run(&mut analyzer, &vowel[..RATE as usize / 2], RATE);
        let late = run(&mut analyzer, &vowel[RATE as usize / 2..], RATE);

        let early_peak = early.bands[argmax(&early.bands)];
        let late_peak = late.bands[argmax(&late.bands)];
        assert!(late_peak > 0.5, "faded to {late_peak}");
        assert!(early_peak - late_peak < 0.35);
    }

    #[test]
    fn brightness_rises_with_the_spectral_centroid() {
        let mut dark = SpectrumAnalyzer::new(RATE);
        let mut bright = SpectrumAnalyzer::new(RATE);

        let dark_frame = run(&mut dark, &sine(400.0, 0.1, RATE, 24_000), RATE);
        let bright_frame = run(&mut bright, &sine(4_000.0, 0.1, RATE, 24_000), RATE);

        assert!(bright_frame.brightness > dark_frame.brightness + 0.4);
        assert!((bright_frame.brightness - 1.0).abs() < 0.05);
    }

    #[test]
    fn steady_background_noise_sinks_while_new_sound_still_shows() {
        // A constant hum raises its band's floor over time, so it fades from view...
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let hum_band = 4;
        let hum_hz = analyzer.band_center_hz(hum_band);
        let early = run(
            &mut analyzer,
            &sine(hum_hz, 0.004, RATE, RATE as usize / 2),
            RATE,
        );
        let late = run(
            &mut analyzer,
            &sine(hum_hz, 0.004, RATE, RATE as usize * 12),
            RATE,
        );
        assert!(late.bands[hum_band] < early.bands[hum_band]);

        // ...while a louder sound in another band is still clearly visible afterwards.
        let voice_band = 16;
        let voice = sine(analyzer.band_center_hz(voice_band), 0.1, RATE, 24_000);
        let frame = run(&mut analyzer, &voice, RATE);
        assert!(frame.bands[voice_band] >= 0.8);
    }

    #[test]
    fn quantised_bands_span_the_byte_range_and_tolerate_bad_values() {
        let mut bands = [0.0f32; BAND_COUNT];
        bands[0] = 1.0;
        bands[1] = 0.5;
        bands[2] = f32::NAN;
        bands[3] = 7.0;
        bands[4] = -1.0;

        let quantized = quantize_bands(&bands);

        assert_eq!(&quantized[..5], &[255, 128, 0, 255, 0]);
    }

    /// Timing guard; run with `cargo test --release --lib spectrum_speed -- --ignored
    /// --nocapture`. One frame must cost far less than the 27 ms between frames.
    #[test]
    #[ignore = "timing test; run in release"]
    fn spectrum_speed() {
        let mut analyzer = SpectrumAnalyzer::new(RATE);
        let signal = voiced(180.0, RATE, RATE as usize, 0.12);
        analyzer.push(&signal);
        let started = std::time::Instant::now();
        let frames = 1_000;
        for _ in 0..frames {
            analyzer.push(&signal[..1_296]);
            std::hint::black_box(analyzer.analyze());
        }
        let per_frame_us = started.elapsed().as_micros() as f64 / frames as f64;
        println!("{per_frame_us:.1} microseconds per frame");
        assert!(per_frame_us < 2_000.0);
    }
}
