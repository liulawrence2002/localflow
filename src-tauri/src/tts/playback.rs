//! Plays synthesized speech on the default output device.
//!
//! Playback is blocking and cooperative: the caller passes a stop flag, and playback returns
//! as soon as the audio ends or the flag is set. Device-specific work (cpal) stays in this
//! module; the sample preparation is pure and unit-tested.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, StreamConfig,
};

use super::provider::SynthesizedAudio;
use crate::audio::resample::resample;

/// Headroom applied to speech before playback so a loud synthesis never clips the device.
const PLAYBACK_PEAK: f32 = 0.89;

/// Fade applied at both ends so playback never starts or stops on a click.
const EDGE_FADE: Duration = Duration::from_millis(8);

/// Mono samples at the device rate, peak-limited and faded at the edges.
pub fn prepare_for_device(audio: &SynthesizedAudio, device_rate: u32) -> Vec<f32> {
    let mut samples = resample(&audio.samples, audio.sample_rate, device_rate);

    let peak = samples
        .iter()
        .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
    if peak > PLAYBACK_PEAK {
        let gain = PLAYBACK_PEAK / peak;
        samples.iter_mut().for_each(|sample| *sample *= gain);
    }

    let fade = ((device_rate as f64 * EDGE_FADE.as_secs_f64()) as usize).min(samples.len() / 2);
    for index in 0..fade {
        let gain = index as f32 / fade as f32;
        samples[index] *= gain;
        let last = samples.len() - 1 - index;
        samples[last] *= gain;
    }

    samples
}

/// Fill one interleaved output buffer from `samples` starting at `position`, duplicating the
/// mono signal on every channel and writing silence once the audio is exhausted. Returns the
/// new position.
fn fill_frames<T: Copy>(
    output: &mut [T],
    channels: usize,
    samples: &[f32],
    position: usize,
    silence: T,
    convert: impl Fn(f32) -> T,
) -> usize {
    let mut position = position;
    for frame in output.chunks_mut(channels.max(1)) {
        let value = match samples.get(position) {
            Some(sample) => {
                position += 1;
                convert(*sample)
            }
            None => silence,
        };
        frame.fill(value);
    }
    position
}

/// Play `audio` and block until it finishes or `stop` becomes true. Returns `Ok(true)` when
/// the audio played to the end and `Ok(false)` when it was stopped early.
pub fn play_blocking(audio: &SynthesizedAudio, stop: &AtomicBool) -> Result<bool, String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "No default audio output device is available.".to_string())?;
    let supported = device
        .default_output_config()
        .map_err(|error| format!("Could not read the audio output configuration: {error}"))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = config.channels as usize;

    let samples = Arc::new(prepare_for_device(audio, config.sample_rate.0));
    if samples.is_empty() {
        return Ok(true);
    }
    let position = Arc::new(AtomicUsize::new(0));
    let on_error = |error| tracing::warn!(error = %error, "speech playback stream error");

    let stream = {
        let samples = samples.clone();
        let position = position.clone();
        match sample_format {
            SampleFormat::F32 => device.build_output_stream(
                &config,
                move |output: &mut [f32], _| {
                    let next = fill_frames(
                        output,
                        channels,
                        &samples,
                        position.load(Ordering::Relaxed),
                        0.0,
                        |sample| sample,
                    );
                    position.store(next, Ordering::Relaxed);
                },
                on_error,
                None,
            ),
            SampleFormat::I16 => device.build_output_stream(
                &config,
                move |output: &mut [i16], _| {
                    let next = fill_frames(
                        output,
                        channels,
                        &samples,
                        position.load(Ordering::Relaxed),
                        0,
                        |sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16,
                    );
                    position.store(next, Ordering::Relaxed);
                },
                on_error,
                None,
            ),
            SampleFormat::U16 => device.build_output_stream(
                &config,
                move |output: &mut [u16], _| {
                    let next = fill_frames(
                        output,
                        channels,
                        &samples,
                        position.load(Ordering::Relaxed),
                        u16::MAX / 2 + 1,
                        |sample| ((sample.clamp(-1.0, 1.0) * 0.5 + 0.5) * u16::MAX as f32) as u16,
                    );
                    position.store(next, Ordering::Relaxed);
                },
                on_error,
                None,
            ),
            other => return Err(format!("Unsupported audio output sample format: {other:?}")),
        }
        .map_err(|error| format!("Could not open the audio output: {error}"))?
    };

    stream
        .play()
        .map_err(|error| format!("Could not start audio playback: {error}"))?;

    // The device clock drives playback. A generous wall-clock bound guards against a device
    // that stops calling back, so this can never block forever.
    let expected = Duration::from_secs_f64(samples.len() as f64 / config.sample_rate.0 as f64);
    let give_up_at = Instant::now() + expected * 2 + Duration::from_secs(5);
    let finished = loop {
        if position.load(Ordering::Relaxed) >= samples.len() {
            // Let the device drain what it already buffered.
            thread::sleep(Duration::from_millis(120));
            break true;
        }
        if stop.load(Ordering::Relaxed) || Instant::now() >= give_up_at {
            break false;
        }
        thread::sleep(Duration::from_millis(20));
    };

    drop(stream);
    Ok(finished)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(samples: Vec<f32>, sample_rate: u32) -> SynthesizedAudio {
        SynthesizedAudio {
            samples,
            sample_rate,
        }
    }

    #[test]
    fn audio_is_resampled_to_the_device_rate() {
        let prepared = prepare_for_device(&audio(vec![0.1; 44_100], 44_100), 48_000);
        assert_eq!(prepared.len(), 48_000);
    }

    #[test]
    fn loud_audio_is_scaled_under_the_playback_peak_and_quiet_audio_is_left_alone() {
        let loud = prepare_for_device(&audio(vec![1.0; 48_000], 48_000), 48_000);
        let peak = loud
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!((peak - PLAYBACK_PEAK).abs() < 1e-3, "peak {peak}");

        let quiet = prepare_for_device(&audio(vec![0.2; 48_000], 48_000), 48_000);
        assert!((quiet[24_000] - 0.2).abs() < 1e-4);
    }

    #[test]
    fn both_ends_are_faded_to_avoid_clicks() {
        let prepared = prepare_for_device(&audio(vec![0.5; 48_000], 48_000), 48_000);
        assert_eq!(prepared[0], 0.0);
        assert_eq!(*prepared.last().unwrap(), 0.0);
        assert!((prepared[24_000] - 0.5).abs() < 1e-4);
    }

    #[test]
    fn very_short_audio_does_not_panic() {
        assert!(prepare_for_device(&audio(Vec::new(), 44_100), 48_000).is_empty());
        assert_eq!(
            prepare_for_device(&audio(vec![0.3], 48_000), 48_000).len(),
            1
        );
    }

    #[test]
    fn frames_duplicate_mono_across_channels_then_write_silence() {
        let samples = [0.25_f32, 0.5];
        let mut output = [9.0_f32; 8];

        let position = fill_frames(&mut output, 2, &samples, 0, 0.0, |sample| sample);

        assert_eq!(position, 2);
        assert_eq!(output, [0.25, 0.25, 0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
    }

    #[test]
    fn filling_resumes_from_the_given_position() {
        let samples = [0.1_f32, 0.2, 0.3];
        let mut output = [0.0_f32; 2];

        let position = fill_frames(&mut output, 1, &samples, 1, 0.0, |sample| sample);

        assert_eq!(position, 3);
        assert_eq!(output, [0.2, 0.3]);
    }
}
