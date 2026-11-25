//! The floating voice overlay: phase and audio-feature events, show/hide, and placement.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};

use crate::audio::spectrum::{quantize_bands, SpectrumFrame, BAND_COUNT};

/// Counts phase changes so a pending auto-hide only fires if nothing newer was shown.
#[derive(Default)]
pub struct OverlayRuntime {
    epoch: AtomicU64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct OverlayAudioFeatures {
    pub(crate) level: f32,
    pub(crate) pitch: f32,
    pub(crate) brightness: f32,
    /// How periodic (voiced) the sound is, 0..1.
    pub(crate) voicing: f32,
    /// Spectrum magnitudes, low to high frequency, quantised to bytes for a small payload.
    pub(crate) bands: [u8; BAND_COUNT],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NativeDictationEvent {
    pub(crate) phase: String,
    pub(crate) message: String,
    pub(crate) level: Option<f32>,
    pub(crate) pitch: Option<f32>,
    pub(crate) brightness: Option<f32>,
    /// Present only on audio-feature events. Older consumers ignore both fields.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) voicing: Option<f32>,
    /// Spectrum bands as bytes (0..=255); divide by 255 for magnitudes in 0..1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) bands: Option<Vec<u8>>,
}

pub(crate) const OVERLAY_WIDTH: i32 = 593;
pub(crate) const OVERLAY_HEIGHT: i32 = 91;
pub(crate) const OVERLAY_BOTTOM_GAP: i32 = 20;

pub(crate) fn display_level_from_rms(rms: f32) -> f32 {
    (rms * 15.0).clamp(0.05, 1.0)
}

/// Overlay features for one analysed frame. `chunk_rms` is the level of the newest samples
/// only, so the level stays as responsive as it was before spectrum analysis was added.
pub(crate) fn overlay_audio_features(
    frame: &SpectrumFrame,
    chunk_rms: f32,
) -> OverlayAudioFeatures {
    OverlayAudioFeatures {
        level: display_level_from_rms(chunk_rms),
        pitch: frame.pitch,
        brightness: frame.brightness,
        voicing: frame.voicing,
        bands: quantize_bands(&frame.bands),
    }
}

pub(crate) fn emit_status(app: &AppHandle, phase: &str, message: &str) {
    emit_native_event(app, phase, message, None);
}

pub(crate) fn emit_audio_features(app: &AppHandle, features: OverlayAudioFeatures) {
    emit_native_event(app, "listening", "Listening", Some(features));
}

pub(crate) fn emit_native_event(
    app: &AppHandle,
    phase: &str,
    message: &str,
    features: Option<OverlayAudioFeatures>,
) {
    // Audio-feature events arrive about 36 times a second while listening. They carry data
    // for the picture only: the overlay is already visible, so they do not touch the window,
    // the hide timer, or the health file.
    let Some(features) = features else {
        emit_phase_event(app, phase, message);
        return;
    };

    let _ = app.emit(
        "localflow://native-dictation",
        NativeDictationEvent {
            phase: phase.to_string(),
            message: message.to_string(),
            level: Some(features.level),
            pitch: Some(features.pitch),
            brightness: Some(features.brightness),
            voicing: Some(features.voicing),
            bands: Some(features.bands.to_vec()),
        },
    );
}

impl OverlayRuntime {
    /// Number a new phase change.
    fn advance(&self) -> u64 {
        self.epoch.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Number a new phase change only if `shown` is still the latest one.
    fn advance_from(&self, shown: u64) -> Option<u64> {
        self.epoch
            .compare_exchange(shown, shown + 1, Ordering::SeqCst, Ordering::SeqCst)
            .ok()
            .map(|_| shown + 1)
    }
}

/// Emit a real phase change: show or hide the overlay, record it, and arm the auto-hide.
pub(crate) fn emit_phase_event(app: &AppHandle, phase: &str, message: &str) {
    let epoch = app.state::<OverlayRuntime>().advance();
    show_phase(app, epoch, phase, message);
}

/// [`emit_status`], returning the change's number for [`emit_status_if_current`].
pub(crate) fn emit_status_tracked(app: &AppHandle, phase: &str, message: &str) -> u64 {
    let epoch = app.state::<OverlayRuntime>().advance();
    show_phase(app, epoch, phase, message);
    epoch
}

/// Emit a phase change only if the overlay still shows the change numbered `shown`, so a
/// feature finishing late cannot replace (or hide) what a newer one put up. Returns the new
/// change's number when it was shown.
pub(crate) fn emit_status_if_current(
    app: &AppHandle,
    shown: u64,
    phase: &str,
    message: &str,
) -> Option<u64> {
    let epoch = app.state::<OverlayRuntime>().advance_from(shown)?;
    show_phase(app, epoch, phase, message);
    Some(epoch)
}

fn show_phase(app: &AppHandle, epoch: u64, phase: &str, message: &str) {
    match phase {
        "ready" | "listening" | "processing" | "refining" | "inserted" | "error" => {
            show_overlay(app, true);
        }
        _ => hide_overlay(app),
    }

    crate::desktop_health::record_overlay_event(app, phase, message);

    let _ = app.emit(
        "localflow://native-dictation",
        NativeDictationEvent {
            phase: phase.to_string(),
            message: message.to_string(),
            level: None,
            pitch: None,
            brightness: None,
            voicing: None,
            bands: None,
        },
    );

    if matches!(phase, "ready" | "inserted" | "error") {
        schedule_overlay_hide(app.clone(), epoch);
    }
}

pub(crate) fn show_overlay(app: &AppHandle, reposition: bool) {
    if let Some(window) = app.get_webview_window("overlay") {
        if reposition {
            if let Err(error) = position_overlay(&window) {
                tracing::warn!(error = %error, "could not position voice overlay");
            }
        }
        let _ = window.show();
    }
}

pub(crate) fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.hide();
    }
}

pub(crate) fn schedule_overlay_hide(app: AppHandle, epoch: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(700));

        if app.state::<OverlayRuntime>().epoch.load(Ordering::SeqCst) == epoch {
            hide_overlay(&app);
        }
    });
}

pub(crate) fn position_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some((x, y)) = overlay_work_area(window) {
        window.set_position(PhysicalPosition::new(x, y))?;
        return Ok(());
    }

    let monitor = window
        .current_monitor()?
        .or_else(|| window.primary_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let size = monitor.size();
        let position = monitor.position();
        let (x, y) = overlay_position_for_work_area(
            position.x,
            position.y,
            size.width as i32,
            size.height as i32,
        );
        window.set_position(PhysicalPosition::new(x, y))?;
    }

    Ok(())
}

#[cfg(windows)]
pub(crate) fn overlay_work_area(window: &WebviewWindow) -> Option<(i32, i32)> {
    let hwnd = window.hwnd().ok()?;
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }

    let mut monitor_info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    if unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
        let work = monitor_info.rcWork;
        Some(overlay_position_for_work_area(
            work.left,
            work.top,
            work.right - work.left,
            work.bottom - work.top,
        ))
    } else {
        None
    }
}

#[cfg(not(windows))]
pub(crate) fn overlay_work_area(_: &WebviewWindow) -> Option<(i32, i32)> {
    None
}

pub(crate) fn overlay_position_for_work_area(
    work_left: i32,
    work_top: i32,
    work_width: i32,
    work_height: i32,
) -> (i32, i32) {
    let available_width = work_width.max(0);
    let available_height = work_height.max(0);
    let x = work_left + ((available_width - OVERLAY_WIDTH) / 2).max(16);
    let y =
        work_top + (available_height - OVERLAY_HEIGHT - OVERLAY_BOTTOM_GAP).max(OVERLAY_BOTTOM_GAP);

    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{capture::rms_level, spectrum::SpectrumAnalyzer};

    #[test]
    fn a_late_status_is_shown_only_if_nothing_newer_was() {
        let runtime = OverlayRuntime::default();
        let reading = runtime.advance();
        // Nothing else was shown: the reading may put up its next status.
        let next = runtime.advance_from(reading).expect("still current");
        assert_eq!(next, reading + 1);

        // A dictation shows "listening"; the reading's late "stopped" must not replace it.
        let dictation = runtime.advance();
        assert_eq!(runtime.advance_from(next), None);
        assert_eq!(runtime.epoch.load(Ordering::SeqCst), dictation);
    }

    #[test]
    fn overlay_features_carry_the_spectrum_pitch_and_a_responsive_level() {
        let sample_rate = 48_000;
        let mut analyzer = SpectrumAnalyzer::new(sample_rate);
        let low = sine_wave(120.0, sample_rate, 0.5);
        analyzer.push(&low);
        let low_features = overlay_audio_features(&analyzer.analyze(), rms_level(&low));

        analyzer.reset();
        let high = sine_wave(260.0, sample_rate, 0.5);
        analyzer.push(&high);
        let high_features = overlay_audio_features(&analyzer.analyze(), rms_level(&high));

        // Higher voices read as higher pitch, as with the previous estimator.
        assert!(high_features.pitch > low_features.pitch);
        assert!(low_features.pitch < 0.3);
        assert!(high_features.pitch > 0.45);
        assert!(low_features.voicing > 0.8);
        // The spectrum is present and the level follows the newest samples.
        assert!(low_features.bands.iter().any(|band| *band > 200));
        assert_eq!(low_features.level, display_level_from_rms(rms_level(&low)));
    }

    #[test]
    fn feature_events_serialise_bands_and_phase_events_omit_them() {
        let with_features = serde_json::to_value(NativeDictationEvent {
            phase: "listening".to_string(),
            message: "Listening".to_string(),
            level: Some(0.5),
            pitch: Some(0.4),
            brightness: Some(0.3),
            voicing: Some(0.9),
            bands: Some(vec![0, 128, 255]),
        })
        .unwrap();
        assert_eq!(with_features["bands"], serde_json::json!([0, 128, 255]));
        assert!((with_features["voicing"].as_f64().unwrap() - 0.9).abs() < 1e-6);

        let phase_only = serde_json::to_value(NativeDictationEvent {
            phase: "processing".to_string(),
            message: "Transcribing".to_string(),
            level: None,
            pitch: None,
            brightness: None,
            voicing: None,
            bands: None,
        })
        .unwrap();
        // Exactly the shape older consumers already handle.
        assert!(phase_only.get("bands").is_none());
        assert!(phase_only.get("voicing").is_none());
        assert!(phase_only["level"].is_null());
    }

    #[test]
    fn overlay_position_centers_inside_work_area() {
        let (x, y) = overlay_position_for_work_area(0, 0, 1920, 1040);

        assert_eq!(x, (1920 - OVERLAY_WIDTH) / 2);
        assert_eq!(y, 1040 - OVERLAY_HEIGHT - OVERLAY_BOTTOM_GAP);
    }

    #[test]
    fn overlay_position_respects_offset_work_area() {
        let (x, y) = overlay_position_for_work_area(-1280, 40, 1280, 984);

        assert_eq!(x, -1280 + ((1280 - OVERLAY_WIDTH) / 2));
        assert_eq!(y, 40 + 984 - OVERLAY_HEIGHT - OVERLAY_BOTTOM_GAP);
    }

    #[test]
    fn overlay_position_clamps_small_work_area() {
        let (x, y) = overlay_position_for_work_area(10, 20, 320, 96);

        assert_eq!(x, 26);
        assert_eq!(y, 40);
    }

    fn sine_wave(frequency_hz: f32, sample_rate: u32, duration_secs: f32) -> Vec<f32> {
        let sample_count = (sample_rate as f32 * duration_secs).round() as usize;

        (0..sample_count)
            .map(|index| {
                let t = index as f32 / sample_rate as f32;
                (std::f32::consts::TAU * frequency_hz * t).sin() * 0.35
            })
            .collect()
    }
}
