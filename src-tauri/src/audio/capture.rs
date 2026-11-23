//! Microphone capture: the cpal input stream, mono downmix and capture statistics.

use std::sync::{Arc, Mutex};

use cpal::{traits::DeviceTrait, SampleFormat, Stream, StreamConfig};

#[derive(Debug, Clone, Copy)]
pub(crate) struct AudioStats {
    pub(crate) duration_secs: f32,
    pub(crate) peak: f32,
    pub(crate) rms: f32,
    pub(crate) nonzero_ratio: f32,
}

pub(crate) const SILENCE_PEAK_THRESHOLD: f32 = 0.003;
pub(crate) const SILENCE_RMS_THRESHOLD: f32 = 0.0005;

/// Channel energy (mean square) above which the capture channel choice is locked for the rest
/// of the recording. Roughly -50 dBFS: above room noise, below quiet speech.
pub(crate) const CHANNEL_LOCK_MEAN_SQUARE: f64 = 0.003 * 0.003;

pub(crate) fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

pub(crate) fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<Stream, String> {
    let channels = config.channels as usize;
    let error_callback = |error| tracing::warn!(error = %error, "microphone stream error");
    // Only one match arm below ever runs, so each can move the selector into its callback.
    let mut selector = ChannelSelector::default();

    match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| push_mono_samples(data, channels, &samples, &mut selector),
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build f32 microphone stream: {error}")),
        SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| *sample as f32 / i16::MAX as f32)
                        .collect();
                    push_mono_samples(&converted, channels, &samples, &mut selector);
                },
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build i16 microphone stream: {error}")),
        SampleFormat::U16 => device
            .build_input_stream(
                config,
                move |data: &[u16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| (*sample as f32 - 32768.0) / 32768.0)
                        .collect();
                    push_mono_samples(&converted, channels, &samples, &mut selector);
                },
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build u16 microphone stream: {error}")),
        other => Err(format!("Unsupported microphone sample format: {other:?}")),
    }
}

pub(crate) fn push_mono_samples(
    data: &[f32],
    channels: usize,
    samples: &Arc<Mutex<Vec<f32>>>,
    selector: &mut ChannelSelector,
) {
    if channels == 0 {
        return;
    }

    let mono = selector.downmix(data, channels);
    if let Ok(mut output) = samples.lock() {
        output.extend(mono);
    }
}

/// Picks which channel of a multi-channel microphone becomes the mono signal.
///
/// Channels are never averaged, because array microphones can deliver phase-inverted channels
/// that cancel to silence. While only room noise has been heard the loudest channel of each
/// callback is used; once a channel carries real signal the choice is locked for the rest of
/// the recording. Without the lock the choice could flip between callbacks mid-word and
/// splice two slightly different signals together.
#[derive(Debug, Default)]
pub(crate) struct ChannelSelector {
    pub(crate) locked: Option<usize>,
}

impl ChannelSelector {
    pub(crate) fn downmix(&mut self, data: &[f32], channels: usize) -> Vec<f32> {
        if channels == 0 {
            return Vec::new();
        }

        if channels == 1 {
            return data
                .iter()
                .copied()
                .map(|sample| sample.clamp(-1.0, 1.0))
                .collect();
        }

        let selected_channel = match self.locked {
            Some(channel) if channel < channels => channel,
            _ => {
                let (channel, mean_square) = loudest_channel_with_energy(data, channels);
                if mean_square >= CHANNEL_LOCK_MEAN_SQUARE {
                    self.locked = Some(channel);
                }
                channel
            }
        };

        data.chunks(channels)
            .filter_map(|frame| frame.get(selected_channel).copied())
            .map(|sample| sample.clamp(-1.0, 1.0))
            .collect()
    }
}

/// Stateless downmix of one callback: the loudest channel wins.
#[cfg(test)]
pub(crate) fn downmix_input_callback(data: &[f32], channels: usize) -> Vec<f32> {
    ChannelSelector::default().downmix(data, channels)
}

/// The channel with the highest mean-square energy in `data`, and that energy.
pub(crate) fn loudest_channel_with_energy(data: &[f32], channels: usize) -> (usize, f64) {
    let mut energy = vec![0.0f64; channels];
    let mut counts = vec![0usize; channels];

    for frame in data.chunks(channels) {
        for (channel, sample) in frame.iter().enumerate() {
            energy[channel] += (*sample as f64) * (*sample as f64);
            counts[channel] += 1;
        }
    }

    let mut selected = 0usize;
    let mut selected_energy = 0.0f64;

    for channel in 0..channels {
        let channel_energy = if counts[channel] == 0 {
            0.0
        } else {
            energy[channel] / counts[channel] as f64
        };

        if channel_energy > selected_energy {
            selected = channel;
            selected_energy = channel_energy;
        }
    }

    (selected, selected_energy)
}

pub(crate) fn analyze_audio(samples: &[f32], sample_rate: u32) -> AudioStats {
    if samples.is_empty() || sample_rate == 0 {
        return AudioStats {
            duration_secs: 0.0,
            peak: 0.0,
            rms: 0.0,
            nonzero_ratio: 0.0,
        };
    }

    let mut peak = 0.0f32;
    let mut sum_squares = 0.0f64;
    let mut nonzero = 0usize;

    for sample in samples {
        let abs = sample.abs();
        peak = peak.max(abs);
        sum_squares += (*sample as f64) * (*sample as f64);
        if abs > 0.000_01 {
            nonzero += 1;
        }
    }

    AudioStats {
        duration_secs: samples.len() as f32 / sample_rate as f32,
        peak,
        rms: (sum_squares / samples.len() as f64).sqrt() as f32,
        nonzero_ratio: nonzero as f32 / samples.len() as f32,
    }
}

pub(crate) fn is_near_silent(stats: AudioStats) -> bool {
    stats.peak < SILENCE_PEAK_THRESHOLD && stats.rms < SILENCE_RMS_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_uses_loudest_channel_to_avoid_phase_cancellation() {
        let interleaved = vec![0.8, -0.8, 0.7, -0.7, 0.6, -0.6, 0.5, -0.5];

        let mono = downmix_input_callback(&interleaved, 2);

        assert_eq!(mono, vec![0.8, 0.7, 0.6, 0.5]);
    }

    #[test]
    fn downmix_prefers_nonzero_channel_for_sparse_multichannel_input() {
        let interleaved = vec![0.0, 0.25, 0.0, 0.5, 0.0, 0.75];

        let mono = downmix_input_callback(&interleaved, 2);

        assert_eq!(mono, vec![0.25, 0.5, 0.75]);
    }

    #[test]
    fn audio_stats_detect_near_silence() {
        let silent = vec![0.0; 16_000];
        let speech = vec![0.02; 16_000];

        assert!(is_near_silent(analyze_audio(&silent, 16_000)));
        assert!(!is_near_silent(analyze_audio(&speech, 16_000)));
    }

    #[test]
    fn channel_choice_locks_once_a_channel_carries_signal() {
        let mut selector = ChannelSelector::default();

        // Speech arrives on channel 1; the choice locks there.
        assert_eq!(selector.downmix(&[0.0, 0.25, 0.0, 0.5], 2), vec![0.25, 0.5]);
        // A later callback where channel 0 is briefly louder must not flip the channel.
        assert_eq!(selector.downmix(&[0.9, 0.1, 0.9, 0.2], 2), vec![0.1, 0.2]);
    }

    #[test]
    fn channel_choice_stays_open_while_only_noise_is_heard() {
        let mut selector = ChannelSelector::default();

        // Both callbacks are far below the lock threshold, so each picks its own loudest.
        assert_eq!(
            selector.downmix(&[0.001, 0.0, 0.001, 0.0], 2),
            vec![0.001, 0.001]
        );
        assert_eq!(
            selector.downmix(&[0.0, 0.001, 0.0, 0.001], 2),
            vec![0.001, 0.001]
        );
        assert_eq!(selector.locked, None);
    }
}
