use std::{
    fs,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    thread,
    time::Duration,
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::app_state::DiagnosticMetric;

const HEALTH_FILE_NAME: &str = "desktop-health.json";
const LAUNCH_SIGNAL_FILE_NAME: &str = "desktop-launch-signal.json";
const HEALTH_SCHEMA_VERSION: u8 = 1;
const LAUNCH_SIGNAL_POLL_MS: u64 = 550;

static HEALTH_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopHealth {
    pub schema_version: u8,
    pub updated_at: Option<String>,
    pub app_start_at: Option<String>,
    pub shortcut_launch_at: Option<String>,
    pub registered_hotkeys: Vec<String>,
    pub failed_hotkeys: Vec<HotkeyRegistrationFailure>,
    pub last_hotkey_event: Option<HotkeyEvent>,
    pub last_overlay_event: Option<OverlayEvent>,
    pub last_recording_start: Option<RecordingStart>,
    pub last_recording_error: Option<RecordingError>,
    pub microphone_device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyRegistrationFailure {
    pub shortcut: String,
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyEvent {
    pub shortcut: String,
    pub state: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlayEvent {
    pub phase: String,
    pub message: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStart {
    pub at: String,
    pub microphone_device_name: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingError {
    pub at: String,
    pub message: String,
}

pub fn record_app_start(app: &AppHandle) {
    let _ = update_health(app, |health| {
        health.app_start_at = Some(now());
    });
}

pub fn record_shortcut_launch(app: &AppHandle) {
    let _ = update_health(app, |health| {
        health.shortcut_launch_at = Some(now());
    });
}

pub fn record_hotkey_registration(
    app: &AppHandle,
    registered_hotkeys: Vec<String>,
    failed_hotkeys: Vec<HotkeyRegistrationFailure>,
) {
    let _ = update_health(app, |health| {
        health.registered_hotkeys = registered_hotkeys;
        health.failed_hotkeys = failed_hotkeys;
    });
}

pub fn record_hotkey_event(app: &AppHandle, shortcut: &str, state: &str) {
    let _ = update_health(app, |health| {
        health.last_hotkey_event = Some(HotkeyEvent {
            shortcut: shortcut.to_string(),
            state: state.to_string(),
            at: now(),
        });
    });
}

pub fn record_overlay_event(app: &AppHandle, phase: &str, message: &str) {
    let _ = update_health(app, |health| {
        health.last_overlay_event = Some(OverlayEvent {
            phase: phase.to_string(),
            message: message.to_string(),
            at: now(),
        });
    });
}

pub fn record_recording_start(
    app: &AppHandle,
    microphone_device_name: &str,
    sample_rate: u32,
    channels: u16,
) {
    let _ = update_health(app, |health| {
        health.microphone_device_name = Some(microphone_device_name.to_string());
        health.last_recording_error = None;
        health.last_recording_start = Some(RecordingStart {
            at: now(),
            microphone_device_name: microphone_device_name.to_string(),
            sample_rate,
            channels,
        });
    });
}

pub fn record_recording_error(app: &AppHandle, message: &str) {
    let _ = update_health(app, |health| {
        health.last_recording_error = Some(RecordingError {
            at: now(),
            message: message.to_string(),
        });
    });
}

pub fn start_launch_signal_watcher(app: AppHandle) {
    thread::spawn(move || {
        let signal_path = match launch_signal_path(&app) {
            Ok(path) => path,
            Err(error) => {
                tracing::warn!(error = %error, "could not resolve desktop launch signal path");
                return;
            }
        };
        let mut last_seen = signal_path
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok();

        loop {
            thread::sleep(Duration::from_millis(LAUNCH_SIGNAL_POLL_MS));
            let modified = signal_path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok();

            if modified.is_some() && modified != last_seen {
                last_seen = modified;
                record_shortcut_launch(&app);
                crate::native_dictation::show_ready_pulse(&app, "LocalFlow ready");
            }
        }
    });
}

pub fn diagnostics(app: &AppHandle) -> Vec<DiagnosticMetric> {
    let health = load_health(app).ok().flatten();
    let mut rows = Vec::new();

    if let Some(health) = health {
        rows.push(DiagnosticMetric {
            label: "Desktop launch".to_string(),
            value: format_optional_time(health.shortcut_launch_at.as_deref()),
            status: if health.shortcut_launch_at.is_some() {
                "ok"
            } else {
                "warning"
            }
            .to_string(),
        });
        rows.push(DiagnosticMetric {
            label: "Registered desktop hotkeys".to_string(),
            value: if health.registered_hotkeys.is_empty() {
                "None recorded".to_string()
            } else {
                health.registered_hotkeys.join(", ")
            },
            status: if health.registered_hotkeys.is_empty() {
                "error"
            } else {
                "ok"
            }
            .to_string(),
        });
        rows.push(DiagnosticMetric {
            label: "Failed desktop hotkeys".to_string(),
            value: if health.failed_hotkeys.is_empty() {
                "None".to_string()
            } else {
                health
                    .failed_hotkeys
                    .iter()
                    .map(|failure| format!("{}: {}", failure.shortcut, failure.error))
                    .collect::<Vec<_>>()
                    .join("; ")
            },
            status: if health.failed_hotkeys.is_empty() {
                "ok"
            } else {
                "warning"
            }
            .to_string(),
        });
        rows.push(DiagnosticMetric {
            label: "Last desktop hotkey event".to_string(),
            value: health
                .last_hotkey_event
                .as_ref()
                .map(|event| format!("{} {} at {}", event.shortcut, event.state, event.at))
                .unwrap_or_else(|| "Not observed yet".to_string()),
            status: "ok".to_string(),
        });
        rows.push(DiagnosticMetric {
            label: "Last overlay event".to_string(),
            value: health
                .last_overlay_event
                .as_ref()
                .map(|event| format!("{} at {}", event.phase, event.at))
                .unwrap_or_else(|| "Not observed yet".to_string()),
            status: "ok".to_string(),
        });
        rows.push(DiagnosticMetric {
            label: "Last recording start".to_string(),
            value: health
                .last_recording_start
                .as_ref()
                .map(|event| {
                    format!(
                        "{} at {} ({} Hz, {} ch)",
                        event.microphone_device_name, event.at, event.sample_rate, event.channels
                    )
                })
                .unwrap_or_else(|| "Not observed yet".to_string()),
            status: if health.last_recording_error.is_some() {
                "warning"
            } else {
                "ok"
            }
            .to_string(),
        });
        rows.push(DiagnosticMetric {
            label: "Last recording error".to_string(),
            value: health
                .last_recording_error
                .as_ref()
                .map(|event| format!("{} at {}", event.message, event.at))
                .unwrap_or_else(|| "None".to_string()),
            status: "ok".to_string(),
        });
    } else {
        rows.push(DiagnosticMetric {
            label: "Desktop health".to_string(),
            value: "desktop-health.json has not been written yet".to_string(),
            status: "warning".to_string(),
        });
    }

    rows
}

fn update_health(
    app: &AppHandle,
    update: impl FnOnce(&mut DesktopHealth),
) -> Result<(), Box<dyn std::error::Error>> {
    let lock = HEALTH_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = health_path(app)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut health = read_health_from_path(&path).unwrap_or_default();
    if health.schema_version == 0 {
        health.schema_version = HEALTH_SCHEMA_VERSION;
    }
    update(&mut health);
    health.updated_at = Some(now());

    fs::write(path, serde_json::to_string_pretty(&health)?)?;
    Ok(())
}

fn load_health(app: &AppHandle) -> Result<Option<DesktopHealth>, Box<dyn std::error::Error>> {
    let path = health_path(app)?;
    if !path.is_file() {
        return Ok(None);
    }

    Ok(Some(read_health_from_path(&path)?))
}

fn read_health_from_path(path: &PathBuf) -> Result<DesktopHealth, Box<dyn std::error::Error>> {
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

fn health_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(app.path().app_data_dir()?.join(HEALTH_FILE_NAME))
}

fn launch_signal_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(app.path().app_data_dir()?.join(LAUNCH_SIGNAL_FILE_NAME))
}

fn format_optional_time(value: Option<&str>) -> String {
    value.unwrap_or("Not observed yet").to_string()
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_file_round_trips_without_transcript_fields() {
        let health = DesktopHealth {
            schema_version: HEALTH_SCHEMA_VERSION,
            updated_at: Some("2026-07-05T00:00:00Z".to_string()),
            app_start_at: Some("2026-07-05T00:00:00Z".to_string()),
            shortcut_launch_at: Some("2026-07-05T00:00:01Z".to_string()),
            registered_hotkeys: vec!["Ctrl+Alt+Space".to_string()],
            failed_hotkeys: vec![],
            last_hotkey_event: Some(HotkeyEvent {
                shortcut: "Ctrl+Alt+Space".to_string(),
                state: "pressed".to_string(),
                at: "2026-07-05T00:00:02Z".to_string(),
            }),
            last_overlay_event: Some(OverlayEvent {
                phase: "ready".to_string(),
                message: "LocalFlow ready".to_string(),
                at: "2026-07-05T00:00:03Z".to_string(),
            }),
            last_recording_start: Some(RecordingStart {
                at: "2026-07-05T00:00:04Z".to_string(),
                microphone_device_name: "Default microphone".to_string(),
                sample_rate: 48_000,
                channels: 2,
            }),
            last_recording_error: None,
            microphone_device_name: Some("Default microphone".to_string()),
        };

        let json = serde_json::to_string(&health).expect("health should serialize");
        assert!(!json.contains("transcript"));
        assert!(!json.contains(".wav"));
        assert!(!json.contains("audioPath"));

        let parsed: DesktopHealth = serde_json::from_str(&json).expect("health should parse");
        assert_eq!(parsed.registered_hotkeys, vec!["Ctrl+Alt+Space"]);
    }
}
