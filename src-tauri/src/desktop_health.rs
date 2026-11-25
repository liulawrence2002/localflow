//! `desktop-health.json`: when the app started, which hotkeys registered, the last hotkey,
//! overlay and recording events, and the last panic. Read by diagnostics and by the desktop
//! boot check script.
//!
//! The record is kept in memory and written by its own thread, a moment after the last
//! change, so recording a hotkey press (on the main thread) never touches the disk. The file
//! is replaced whole (written to a temporary file, then renamed over it), so a reader never
//! sees half of it.
//!
//! Privacy: no transcript text, no microphone names, and no paths with the Windows user name
//! (messages pass through `redact_user_paths`).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::Duration,
};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::{app_state::DiagnosticMetric, storage::redact::redact_user_paths};

const HEALTH_FILE_NAME: &str = "desktop-health.json";
const LAUNCH_SIGNAL_FILE_NAME: &str = "desktop-launch-signal.json";
const HEALTH_SCHEMA_VERSION: u8 = 1;
const LAUNCH_SIGNAL_POLL_MS: u64 = 550;
/// How long the writer waits after a change for more before writing, so a key press and its
/// release become one write.
const WRITE_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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
    pub last_panic: Option<PanicRecord>,
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

/// A recording that started. The microphone's name is not kept: it can name the person
/// ("Alice's AirPods").
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingStart {
    pub at: String,
    pub sample_rate: u32,
    pub channels: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingError {
    pub at: String,
    pub message: String,
}

/// Where a panic happened: the thread's name and the source location. Never the panic's
/// message, which can quote the text being worked on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PanicRecord {
    pub at: String,
    pub thread: String,
    pub location: String,
}

/// The in-memory record and the file it is written to.
struct HealthStore {
    path: PathBuf,
    /// The record as last read or changed; `None` until first needed.
    state: Mutex<Option<DesktopHealth>>,
    /// Held while the file is written or read.
    file: Mutex<()>,
    writer: OnceLock<mpsc::Sender<()>>,
}

impl HealthStore {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            state: Mutex::new(None),
            file: Mutex::new(()),
            writer: OnceLock::new(),
        }
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, Option<DesktopHealth>> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The file's content, read under the file lock.
    fn read_file(&self) -> Option<DesktopHealth> {
        let _file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        read_health_from_path(&self.path).ok()
    }

    /// Change the record in memory; the caller arranges the write.
    fn apply(&self, update: impl FnOnce(&mut DesktopHealth)) {
        let mut state = self.lock_state();
        let health = state.get_or_insert_with(|| self.read_file().unwrap_or_default());
        if health.schema_version == 0 {
            health.schema_version = HEALTH_SCHEMA_VERSION;
        }
        update(health);
        health.updated_at = Some(now());
    }

    /// The current record, from memory when it has been loaded, otherwise from the file.
    fn snapshot(&self) -> Option<DesktopHealth> {
        if let Some(health) = self.lock_state().as_ref() {
            return Some(health.clone());
        }
        self.read_file()
    }

    /// Write the record now, replacing the file whole.
    fn flush(&self) -> std::io::Result<()> {
        let Some(health) = self.lock_state().clone() else {
            return Ok(());
        };
        let _file = self
            .file
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        write_atomically(&self.path, &health)
    }

    /// Ask the writer thread to write soon, starting it the first time.
    fn schedule_write(&'static self) {
        let sender = self.writer.get_or_init(|| {
            let (sender, receiver) = mpsc::channel::<()>();
            let spawned = thread::Builder::new()
                .name("localflow-health-writer".to_string())
                .spawn(move || {
                    while receiver.recv().is_ok() {
                        thread::sleep(WRITE_DELAY);
                        while receiver.try_recv().is_ok() {}
                        if let Err(error) = self.flush() {
                            tracing::warn!(error = %error, "could not write desktop health");
                        }
                    }
                });
            if let Err(error) = spawned {
                tracing::warn!(error = %error, "could not start the desktop health writer");
            }
            sender
        });
        let _ = sender.send(());
    }
}

/// Write `health` to a temporary file next to `path`, then move it over `path`.
fn write_atomically(path: &Path, health: &DesktopHealth) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(health).map_err(std::io::Error::other)?;
    let mut temporary = path.as_os_str().to_os_string();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, json)?;
    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // A reader that holds the file without sharing it blocks the rename; try once more.
            thread::sleep(Duration::from_millis(50));
            fs::rename(&temporary, path).inspect_err(|_| {
                let _ = fs::remove_file(&temporary);
            })
        }
    }
}

static STORE: OnceLock<HealthStore> = OnceLock::new();

fn store(app: &AppHandle) -> Option<&'static HealthStore> {
    if let Some(store) = STORE.get() {
        return Some(store);
    }
    let path = health_path(app).ok()?;
    Some(STORE.get_or_init(|| HealthStore::new(path)))
}

fn update_health(app: &AppHandle, update: impl FnOnce(&mut DesktopHealth)) {
    if let Some(store) = store(app) {
        store.apply(update);
        store.schedule_write();
    }
}

/// Write any change not yet on disk. Called on the way out.
pub fn flush() {
    if let Some(store) = STORE.get() {
        if let Err(error) = store.flush() {
            tracing::warn!(error = %error, "could not write desktop health");
        }
    }
}

pub fn record_app_start(app: &AppHandle) {
    update_health(app, |health| {
        health.app_start_at = Some(now());
    });
}

pub fn record_shortcut_launch(app: &AppHandle) {
    update_health(app, |health| {
        health.shortcut_launch_at = Some(now());
    });
}

pub fn record_hotkey_registration(
    app: &AppHandle,
    registered_hotkeys: Vec<String>,
    failed_hotkeys: Vec<HotkeyRegistrationFailure>,
) {
    let failed_hotkeys = failed_hotkeys
        .into_iter()
        .map(|failure| HotkeyRegistrationFailure {
            error: redact_user_paths(&failure.error).into_owned(),
            ..failure
        })
        .collect();
    update_health(app, |health| {
        health.registered_hotkeys = registered_hotkeys;
        health.failed_hotkeys = failed_hotkeys;
    });
}

pub fn record_hotkey_event(app: &AppHandle, shortcut: &str, state: &str) {
    update_health(app, |health| {
        health.last_hotkey_event = Some(HotkeyEvent {
            shortcut: shortcut.to_string(),
            state: state.to_string(),
            at: now(),
        });
    });
}

pub fn record_overlay_event(app: &AppHandle, phase: &str, message: &str) {
    let message = redact_user_paths(message).into_owned();
    update_health(app, |health| {
        health.last_overlay_event = Some(OverlayEvent {
            phase: phase.to_string(),
            message,
            at: now(),
        });
    });
}

/// A recording started. The microphone's name is accepted for the callers' convenience and
/// not kept.
pub fn record_recording_start(
    app: &AppHandle,
    _microphone_device_name: &str,
    sample_rate: u32,
    channels: u16,
) {
    update_health(app, |health| {
        health.last_recording_error = None;
        health.last_recording_start = Some(RecordingStart {
            at: now(),
            sample_rate,
            channels,
        });
    });
}

pub fn record_recording_error(app: &AppHandle, message: &str) {
    let message = redact_user_paths(message).into_owned();
    update_health(app, |health| {
        health.last_recording_error = Some(RecordingError { at: now(), message });
    });
}

/// Record a panic. Called from the panic hook, so it never waits: if the record or the file
/// is busy (possibly on the panicking thread itself), the panic is left to the log.
pub fn record_panic(thread: &str, location: &str) {
    let Some(store) = STORE.get() else {
        return;
    };
    let record = PanicRecord {
        at: now(),
        thread: thread.to_string(),
        location: redact_user_paths(location).into_owned(),
    };
    let health = match store.state.try_lock() {
        Ok(mut state) => {
            let health =
                state.get_or_insert_with(|| read_health_from_path(&store.path).unwrap_or_default());
            health.last_panic = Some(record);
            health.updated_at = Some(now());
            health.clone()
        }
        // Busy or poisoned: the panic may have happened while it was held.
        Err(_) => return,
    };
    // Written at once: a panic on the main thread ends the app before the writer runs.
    if let Ok(_file) = store.file.try_lock() {
        let _ = write_atomically(&store.path, &health);
    }
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
    health_rows(store(app).and_then(HealthStore::snapshot).as_ref())
}

fn health_rows(health: Option<&DesktopHealth>) -> Vec<DiagnosticMetric> {
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
                        "{} ({} Hz, {} ch)",
                        event.at, event.sample_rate, event.channels
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
        rows.push(DiagnosticMetric {
            label: "Last crash".to_string(),
            value: health
                .last_panic
                .as_ref()
                .map(|panic| {
                    format!(
                        "{} on thread {} at {}",
                        panic.location, panic.thread, panic.at
                    )
                })
                .unwrap_or_else(|| "None".to_string()),
            status: if health.last_panic.is_some() {
                "warning"
            } else {
                "ok"
            }
            .to_string(),
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

fn read_health_from_path(path: &Path) -> Result<DesktopHealth, Box<dyn std::error::Error>> {
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

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "localflow-health-{name}-{}-{}",
                std::process::id(),
                Utc::now().timestamp_nanos_opt().unwrap_or_default()
            ))
            .join(HEALTH_FILE_NAME)
    }

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
                sample_rate: 48_000,
                channels: 2,
            }),
            last_recording_error: None,
            last_panic: None,
        };

        let json = serde_json::to_string(&health).expect("health should serialize");
        assert!(!json.contains("transcript"));
        assert!(!json.contains(".wav"));
        assert!(!json.contains("audioPath"));
        assert!(!json.contains("microphone"));

        let parsed: DesktopHealth = serde_json::from_str(&json).expect("health should parse");
        assert_eq!(parsed.registered_hotkeys, vec!["Ctrl+Alt+Space"]);
    }

    #[test]
    fn a_file_from_an_older_build_with_device_names_still_loads_and_drops_them() {
        let older = r#"{"schemaVersion":1,"microphoneDeviceName":"Alice's AirPods",
            "lastRecordingStart":{"at":"t","microphoneDeviceName":"Alice's AirPods","sampleRate":16000,"channels":1}}"#;
        let parsed: DesktopHealth = serde_json::from_str(older).unwrap();
        assert_eq!(
            parsed.last_recording_start.as_ref().unwrap().sample_rate,
            16_000
        );
        assert!(!serde_json::to_string(&parsed).unwrap().contains("AirPods"));
    }

    #[test]
    fn changes_stay_in_memory_until_written_and_the_file_is_replaced_whole() {
        let path = temp_path("store");
        let store = HealthStore::new(path.clone());

        store.apply(|health| health.registered_hotkeys = vec!["Ctrl+Alt+Space".into()]);
        assert!(!path.exists(), "applying a change must not touch the disk");
        assert_eq!(
            store.snapshot().unwrap().registered_hotkeys,
            ["Ctrl+Alt+Space"]
        );

        store.flush().unwrap();
        let on_disk = read_health_from_path(&path).unwrap();
        assert_eq!(on_disk.registered_hotkeys, ["Ctrl+Alt+Space"]);
        assert_eq!(on_disk.schema_version, HEALTH_SCHEMA_VERSION);
        let mut temporary = path.as_os_str().to_os_string();
        temporary.push(".tmp");
        assert!(!PathBuf::from(temporary).exists());

        // A second store over the same file starts from what was written.
        let reopened = HealthStore::new(path.clone());
        reopened.apply(|health| health.app_start_at = Some("later".into()));
        let health = reopened.snapshot().unwrap();
        assert_eq!(health.registered_hotkeys, ["Ctrl+Alt+Space"]);
        assert_eq!(health.app_start_at.as_deref(), Some("later"));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_damaged_file_is_replaced_rather_than_blocking_updates() {
        let path = temp_path("damaged");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "{ half a record").unwrap();

        let store = HealthStore::new(path.clone());
        store.apply(|health| health.shortcut_launch_at = Some("now".into()));
        store.flush().unwrap();

        assert_eq!(
            read_health_from_path(&path)
                .unwrap()
                .shortcut_launch_at
                .as_deref(),
            Some("now")
        );
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn diagnostics_show_no_device_name_and_report_a_crash() {
        let mut health = DesktopHealth {
            last_recording_start: Some(RecordingStart {
                at: "t1".into(),
                sample_rate: 48_000,
                channels: 2,
            }),
            ..DesktopHealth::default()
        };
        let rows = health_rows(Some(&health));
        let start = rows
            .iter()
            .find(|row| row.label == "Last recording start")
            .unwrap();
        assert_eq!(start.value, "t1 (48000 Hz, 2 ch)");
        let crash = rows.iter().find(|row| row.label == "Last crash").unwrap();
        assert_eq!(
            (crash.value.as_str(), crash.status.as_str()),
            ("None", "ok")
        );

        health.last_panic = Some(PanicRecord {
            at: "t2".into(),
            thread: "localflow-ear".into(),
            location: r"src\conversation\engine.rs:120:9".into(),
        });
        let rows = health_rows(Some(&health));
        let crash = rows.iter().find(|row| row.label == "Last crash").unwrap();
        assert_eq!(crash.status, "warning");
        assert!(crash.value.contains("localflow-ear"));
        assert!(crash.value.contains("engine.rs:120"));
    }
}
