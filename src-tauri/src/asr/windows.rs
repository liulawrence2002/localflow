//! Rolling audio window planner for streaming ASR.
//!
//! A persistent ASR runtime re-decodes overlapping windows of the captured audio so it can
//! emit partials before the user stops speaking. This plans those window boundaries.
//! Authoritative Rust port of the tested `src/domain/asrWindows.ts`.
//!
//! Streaming ASR foundation (spec Phase 3): exercised by the unit tests below; not yet on the
//! default one-shot path, so `dead_code` is allowed.
#![allow(dead_code)]

#[derive(Debug, Clone, PartialEq)]
pub struct AudioWindow {
    pub index: usize,
    pub start_sample: usize,
    pub end_sample: usize,
    pub overlap_start_sample: usize,
    pub duration_ms: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct RollingWindowConfig {
    pub total_samples: usize,
    pub sample_rate: u32,
    pub window_ms: u32,
    pub overlap_ms: u32,
}

/// Plan the sequence of rolling windows for the given audio length.
pub fn plan_rolling_windows(config: RollingWindowConfig) -> Result<Vec<AudioWindow>, String> {
    if config.total_samples == 0 {
        return Ok(Vec::new());
    }
    if config.sample_rate == 0 || config.window_ms == 0 {
        return Err("Invalid rolling window configuration.".to_string());
    }
    if config.overlap_ms >= config.window_ms {
        return Err("overlap_ms must be shorter than window_ms.".to_string());
    }

    let sample_rate = config.sample_rate as f64;
    let window_samples = ((config.window_ms as f64 / 1000.0) * sample_rate)
        .round()
        .max(1.0) as usize;
    let overlap_samples = ((config.overlap_ms as f64 / 1000.0) * sample_rate).round() as usize;
    let step_samples = window_samples.saturating_sub(overlap_samples).max(1);

    let mut windows = Vec::new();
    let mut start_sample = 0usize;
    while start_sample < config.total_samples {
        let end_sample = (start_sample + window_samples).min(config.total_samples);
        windows.push(AudioWindow {
            index: windows.len(),
            start_sample,
            end_sample,
            overlap_start_sample: start_sample.saturating_sub(overlap_samples),
            duration_ms: (end_sample - start_sample) as f64 / sample_rate * 1000.0,
        });

        if end_sample == config.total_samples {
            break;
        }
        start_sample += step_samples;
    }

    Ok(windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_audio_produces_no_windows() {
        let windows = plan_rolling_windows(RollingWindowConfig {
            total_samples: 0,
            sample_rate: 16_000,
            window_ms: 1000,
            overlap_ms: 200,
        })
        .unwrap();
        assert!(windows.is_empty());
    }

    #[test]
    fn windows_step_by_window_minus_overlap_and_cover_all_audio() {
        let windows = plan_rolling_windows(RollingWindowConfig {
            total_samples: 48_000, // 3 seconds at 16 kHz
            sample_rate: 16_000,
            window_ms: 1000, // 16000-sample windows
            overlap_ms: 250, // 4000-sample overlap -> step 12000
        })
        .unwrap();

        assert_eq!(windows[0].start_sample, 0);
        assert_eq!(windows[1].start_sample, 12_000);
        // Last window ends exactly at the end of the audio.
        assert_eq!(windows.last().unwrap().end_sample, 48_000);
        // Windows are contiguous with the configured step.
        for pair in windows.windows(2) {
            assert_eq!(pair[1].start_sample - pair[0].start_sample, 12_000);
        }
    }

    #[test]
    fn rejects_overlap_not_shorter_than_window() {
        let result = plan_rolling_windows(RollingWindowConfig {
            total_samples: 16_000,
            sample_rate: 16_000,
            window_ms: 500,
            overlap_ms: 500,
        });
        assert!(result.is_err());
    }
}
