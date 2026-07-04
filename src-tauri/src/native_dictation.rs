use std::{
    collections::VecDeque,
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use arboard::Clipboard;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub struct NativeDictationRuntime {
    command_tx: Mutex<mpsc::Sender<RecorderCommand>>,
    overlay_epoch: AtomicU64,
    sessions: Mutex<SessionRegistry>,
    last_transcript: Mutex<Option<String>>,
    latency_snapshots: Mutex<VecDeque<NativeLatencySnapshot>>,
}

/// Tracks which dictation session is currently authorized to insert text.
///
/// Every recording is assigned a monotonically increasing `id`. `active` holds the
/// id that is allowed to reach insertion. Starting a new recording or cancelling
/// bumps the sequence so any in-flight worker for an older id becomes non-current
/// and aborts before pasting. This is the native-path equivalent of the spec's
/// session-identity + stale-event rejection requirement (§4.4): a cancelled or
/// superseded session must never insert text.
#[derive(Debug, Default, Clone, Copy)]
struct SessionRegistry {
    seq: u64,
    active: u64,
}

impl SessionRegistry {
    /// Assign and activate a fresh session id.
    fn begin(&mut self) -> u64 {
        self.seq += 1;
        self.active = self.seq;
        self.seq
    }

    /// Invalidate the current session without starting a new recording (cancel).
    /// The active id becomes one that no worker holds, so any in-flight worker aborts.
    fn invalidate(&mut self) {
        self.seq += 1;
        self.active = self.seq;
    }

    /// Whether `id` is still the session authorized to insert.
    fn is_current(&self, id: u64) -> bool {
        id != 0 && self.active == id
    }
}

/// The focused window captured when dictation started, used to verify the insertion
/// target has not changed before pasting. `hwnd` is stored as an `isize` (the raw handle
/// value) so the token is `Send` and can cross to the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TargetWindow {
    hwnd: isize,
    pid: u32,
}

/// Read the current foreground window as an insertion target. Returns `None` if no window
/// is focused or its owning process cannot be determined.
#[cfg(windows)]
fn foreground_target() -> Option<TargetWindow> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        Some(TargetWindow {
            hwnd: hwnd.0 as isize,
            pid,
        })
    }
}

#[cfg(not(windows))]
fn foreground_target() -> Option<TargetWindow> {
    None
}

/// Whether it is safe to insert into the current foreground window. Fails closed: unless
/// the captured target and the current foreground window are both known and identical, the
/// caller must not paste (spec: never insert into a target that cannot be revalidated).
fn target_matches(captured: Option<TargetWindow>, current: Option<TargetWindow>) -> bool {
    match (captured, current) {
        (Some(captured), Some(current)) => {
            captured.hwnd == current.hwnd && captured.pid == current.pid
        }
        _ => false,
    }
}

struct RecordingSession {
    session_id: u64,
    target: Option<TargetWindow>,
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
    device_name: String,
    sample_format: SampleFormat,
    hotkey_received_at: Instant,
    started_at: Instant,
    last_voice_at: Arc<Mutex<Option<Instant>>>,
    level_stop_tx: mpsc::Sender<()>,
}

#[derive(Debug, Clone, Copy)]
struct AudioStats {
    duration_secs: f32,
    peak: f32,
    rms: f32,
    nonzero_ratio: f32,
}

#[derive(Debug, Clone, Copy)]
struct OverlayAudioFeatures {
    level: f32,
    pitch: f32,
    brightness: f32,
}

struct RecorderCommand {
    app: AppHandle,
    state: String,
    received_at: Instant,
}

#[derive(Debug, Clone)]
struct NativeLatencySnapshot {
    session_id: u64,
    ollama_model: String,
    low_resource_mode: bool,
    audio_duration_ms: u128,
    hotkey_to_recording_start_ms: Option<u128>,
    vad_tail_ms: Option<u128>,
    capture_stop_ms: Option<u128>,
    resample_ms: Option<u128>,
    wav_write_ms: Option<u128>,
    whisper_ms: Option<u128>,
    deterministic_ms: Option<u128>,
    paste_set_clipboard_ms: Option<u128>,
    paste_send_ms: Option<u128>,
    clipboard_restore_delay_ms: Option<u128>,
    speech_end_to_visible_ms: Option<u128>,
    recording_start_to_insert_ms: Option<u128>,
    ollama_cleanup_ms: Option<u128>,
    background_cleanup_status: BackgroundCleanupStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackgroundCleanupStatus {
    Pending,
    Completed,
    Failed,
    Skipped,
}

impl BackgroundCleanupStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct PasteTimings {
    set_clipboard_ms: u128,
    send_paste_ms: u128,
    visible_at: Instant,
}

#[derive(Debug)]
struct PasteStart {
    previous_text: Option<String>,
    timings: PasteTimings,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeDictationEvent {
    phase: String,
    message: String,
    level: Option<f32>,
    pitch: Option<f32>,
    brightness: Option<f32>,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    format: &'a str,
    keep_alive: &'a str,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[derive(Debug, Deserialize)]
struct CleanupModelResponse {
    text: String,
}

const OLLAMA_MODEL: &str = "llama3.2:3b";
const OLLAMA_GENERATE_URL: &str = "http://127.0.0.1:11434/api/generate";
const SILENCE_PEAK_THRESHOLD: f32 = 0.003;
const SILENCE_RMS_THRESHOLD: f32 = 0.0005;
const VAD_POLL_MS: u64 = 55;
const VAD_START_RMS_THRESHOLD: f32 = 0.008;
const VAD_CONTINUE_RMS_THRESHOLD: f32 = 0.0045;
const VAD_CONFIRM_SPEECH_MS: u64 = 120;
const END_OF_SPEECH_TIMEOUT_MS: u64 = 550;
const MIN_AUTO_STOP_RECORDING_MS: u64 = 350;
const QUICK_TAP_RELEASE_MS: u64 = 700;
const NO_SPEECH_TIMEOUT_MS: u64 = 2_500;
const MAX_RECORDING_SECS: u64 = 120;
const OLLAMA_KEEP_ALIVE: &str = "30m";
const OVERLAY_WIDTH: i32 = 456;
const OVERLAY_HEIGHT: i32 = 70;
const OVERLAY_BOTTOM_GAP: i32 = 20;
const MAX_LATENCY_SNAPSHOTS: usize = 5;
const TARGET_VISIBLE_TEXT_MS: u128 = 1_000;
const SLOW_VISIBLE_TEXT_MS: u128 = 2_000;
const WHISPER_BOTTLENECK_MS: u128 = 700;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

impl Default for NativeDictationRuntime {
    fn default() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        thread::spawn(move || recorder_loop(command_rx));

        Self {
            command_tx: Mutex::new(command_tx),
            overlay_epoch: AtomicU64::new(0),
            sessions: Mutex::new(SessionRegistry::default()),
            last_transcript: Mutex::new(None),
            latency_snapshots: Mutex::new(VecDeque::new()),
        }
    }
}

impl NativeDictationRuntime {
    fn record_latency_snapshot(&self, snapshot: NativeLatencySnapshot) {
        let mut snapshots = self
            .latency_snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        push_latency_snapshot(&mut snapshots, snapshot);
    }

    fn update_latency_snapshot(
        &self,
        session_id: u64,
        update: impl FnOnce(&mut NativeLatencySnapshot),
    ) {
        let mut snapshots = self
            .latency_snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(snapshot) = snapshots
            .iter_mut()
            .rev()
            .find(|snapshot| snapshot.session_id == session_id)
        {
            update(snapshot);
        }
    }

    pub fn latency_diagnostics(&self) -> Vec<crate::app_state::DiagnosticMetric> {
        let snapshots = self
            .latency_snapshots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let latest = snapshots.back().cloned();
        latency_diagnostics_for(latest.as_ref())
    }
}

/// Lock the session registry, recovering (rather than propagating) a poisoned lock so
/// a panic elsewhere can never wedge dictation.
fn with_sessions<T>(app: &AppHandle, f: impl FnOnce(&mut SessionRegistry) -> T) -> T {
    let runtime = app.state::<NativeDictationRuntime>();
    let mut guard = runtime
        .sessions
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// Assign and activate a fresh session id, superseding any in-flight worker.
fn begin_session(app: &AppHandle) -> u64 {
    with_sessions(app, SessionRegistry::begin)
}

/// Cancel the active session; any in-flight worker for the old id will abort before pasting.
fn invalidate_session(app: &AppHandle) {
    with_sessions(app, SessionRegistry::invalidate);
}

/// Whether `session_id` is still authorized to insert text.
fn session_is_current(app: &AppHandle, session_id: u64) -> bool {
    with_sessions(app, |registry| registry.is_current(session_id))
}

/// Register (or unregister) the Escape-to-cancel global shortcut. It is only active while a
/// dictation is recording, so pressing Escape mid-utterance cancels without ever inserting,
/// and Escape is not suppressed system-wide the rest of the time.
fn set_escape_cancel(app: &AppHandle, enable: bool) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let shortcut = crate::hotkeys::escape_cancel_shortcut();
    let manager = app.global_shortcut();

    if enable {
        if let Err(error) = manager.register(shortcut) {
            tracing::debug!(error = %error, "escape-to-cancel shortcut unavailable");
        }
    } else {
        let _ = manager.unregister(shortcut);
    }
}

/// Remember the most recent finalized transcript in volatile memory so the user can
/// recover it (copy / paste-last) if automatic insertion was skipped or failed. This is
/// session-scoped memory only; nothing is written to disk (retention-safe by default).
fn store_last_transcript(app: &AppHandle, text: &str) {
    let runtime = app.state::<NativeDictationRuntime>();
    let mut guard = runtime
        .last_transcript
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(text.to_string());
}

/// The most recent finalized transcript, if any.
fn last_transcript(app: &AppHandle) -> Option<String> {
    let runtime = app.state::<NativeDictationRuntime>();
    let guard = runtime
        .last_transcript
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.clone()
}

/// Copy the last finalized transcript to the clipboard for manual pasting. Reachable from
/// the tray and the settings UI; needs no target focus, so it is always safe to run.
pub fn copy_last_transcript_to_clipboard(app: &AppHandle) -> Result<(), String> {
    let text = last_transcript(app)
        .ok_or_else(|| "No transcript is available to copy yet.".to_string())?;
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
    clipboard
        .set_text(text)
        .map_err(|error| format!("Could not set clipboard text: {error}"))
}

/// Return the most recent finalized transcript (or `null`) for the recovery UI.
#[tauri::command]
pub fn get_last_transcript(app: AppHandle) -> Option<String> {
    last_transcript(&app)
}

/// Copy the most recent finalized transcript to the clipboard (explicit recovery action).
#[tauri::command]
pub fn copy_last_transcript(app: AppHandle) -> Result<(), String> {
    copy_last_transcript_to_clipboard(&app)
}

fn record_latency_snapshot(app: &AppHandle, snapshot: NativeLatencySnapshot) {
    app.state::<NativeDictationRuntime>()
        .record_latency_snapshot(snapshot);
}

fn update_latency_snapshot(
    app: &AppHandle,
    session_id: u64,
    update: impl FnOnce(&mut NativeLatencySnapshot),
) {
    app.state::<NativeDictationRuntime>()
        .update_latency_snapshot(session_id, update);
}

fn push_latency_snapshot(
    snapshots: &mut VecDeque<NativeLatencySnapshot>,
    snapshot: NativeLatencySnapshot,
) {
    while snapshots.len() >= MAX_LATENCY_SNAPSHOTS {
        snapshots.pop_front();
    }
    snapshots.push_back(snapshot);
}

fn latency_diagnostics_for(
    latest: Option<&NativeLatencySnapshot>,
) -> Vec<crate::app_state::DiagnosticMetric> {
    let Some(snapshot) = latest else {
        return vec![
            diagnostic_metric(
                "Latency: speech end to visible text",
                "Not measured",
                "warning",
            ),
            diagnostic_metric(
                "Latency: recording start to insert",
                "Not measured",
                "warning",
            ),
            diagnostic_metric("Latency: Whisper sidecar", "Not measured", "warning"),
            diagnostic_metric("Latency: Ollama cleanup", "Not measured", "warning"),
        ];
    };

    vec![
        diagnostic_metric(
            "Latency: speech end to visible text",
            format_optional_ms(snapshot.speech_end_to_visible_ms),
            visible_latency_status(snapshot.speech_end_to_visible_ms),
        ),
        diagnostic_metric(
            "Latency: recording start to insert",
            format_optional_ms(snapshot.recording_start_to_insert_ms),
            optional_ms_status(snapshot.recording_start_to_insert_ms),
        ),
        diagnostic_metric(
            "Latency: VAD tail",
            format_optional_ms(snapshot.vad_tail_ms),
            optional_ms_status(snapshot.vad_tail_ms),
        ),
        diagnostic_metric(
            "Latency: capture stop",
            format_optional_ms(snapshot.capture_stop_ms),
            optional_ms_status(snapshot.capture_stop_ms),
        ),
        diagnostic_metric(
            "Latency: resample",
            format_optional_ms(snapshot.resample_ms),
            optional_ms_status(snapshot.resample_ms),
        ),
        diagnostic_metric(
            "Latency: WAV write",
            format_optional_ms(snapshot.wav_write_ms),
            optional_ms_status(snapshot.wav_write_ms),
        ),
        diagnostic_metric(
            "Latency: Whisper sidecar",
            format_optional_ms(snapshot.whisper_ms),
            whisper_latency_status(snapshot.whisper_ms),
        ),
        diagnostic_metric(
            "Latency: deterministic formatting",
            format_optional_ms(snapshot.deterministic_ms),
            optional_ms_status(snapshot.deterministic_ms),
        ),
        diagnostic_metric(
            "Latency: paste send",
            format!(
                "clipboard {} / send {}",
                format_optional_ms(snapshot.paste_set_clipboard_ms),
                format_optional_ms(snapshot.paste_send_ms)
            ),
            paste_latency_status(snapshot.paste_set_clipboard_ms, snapshot.paste_send_ms),
        ),
        diagnostic_metric(
            "Latency: clipboard restore",
            format_optional_ms(snapshot.clipboard_restore_delay_ms),
            optional_ms_status(snapshot.clipboard_restore_delay_ms),
        ),
        diagnostic_metric(
            "Latency: Ollama cleanup",
            format_background_cleanup(snapshot),
            background_cleanup_status(snapshot.background_cleanup_status),
        ),
        diagnostic_metric(
            "Latency: latest audio",
            format!(
                "{} ms audio / model {} / cleanup {}",
                snapshot.audio_duration_ms,
                snapshot.ollama_model,
                snapshot.background_cleanup_status.label()
            ),
            "ok",
        ),
        diagnostic_metric(
            "Latency: hotkey to recording",
            format_optional_ms(snapshot.hotkey_to_recording_start_ms),
            optional_ms_status(snapshot.hotkey_to_recording_start_ms),
        ),
    ]
}

fn diagnostic_metric(
    label: impl Into<String>,
    value: impl Into<String>,
    status: impl Into<String>,
) -> crate::app_state::DiagnosticMetric {
    crate::app_state::DiagnosticMetric {
        label: label.into(),
        value: value.into(),
        status: status.into(),
    }
}

fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(|value| format!("{value} ms"))
        .unwrap_or_else(|| "Not measured".to_string())
}

fn visible_latency_status(value: Option<u128>) -> &'static str {
    match value {
        Some(value) if value < TARGET_VISIBLE_TEXT_MS => "ok",
        Some(value) if value < SLOW_VISIBLE_TEXT_MS => "warning",
        Some(_) => "error",
        None => "warning",
    }
}

fn whisper_latency_status(value: Option<u128>) -> &'static str {
    match value {
        Some(value) if value <= WHISPER_BOTTLENECK_MS => "ok",
        Some(_) => "warning",
        None => "warning",
    }
}

fn optional_ms_status(value: Option<u128>) -> &'static str {
    if value.is_some() {
        "ok"
    } else {
        "warning"
    }
}

fn paste_latency_status(
    set_clipboard_ms: Option<u128>,
    send_paste_ms: Option<u128>,
) -> &'static str {
    if set_clipboard_ms.is_some() && send_paste_ms.is_some() {
        "ok"
    } else {
        "warning"
    }
}

fn background_cleanup_status(status: BackgroundCleanupStatus) -> &'static str {
    match status {
        BackgroundCleanupStatus::Pending => "warning",
        BackgroundCleanupStatus::Completed | BackgroundCleanupStatus::Skipped => "ok",
        BackgroundCleanupStatus::Failed => "error",
    }
}

fn format_background_cleanup(snapshot: &NativeLatencySnapshot) -> String {
    match snapshot.background_cleanup_status {
        BackgroundCleanupStatus::Pending => format!("Pending on {}", snapshot.ollama_model),
        BackgroundCleanupStatus::Skipped if snapshot.low_resource_mode => {
            "Skipped by low-resource mode".to_string()
        }
        BackgroundCleanupStatus::Skipped => "Skipped".to_string(),
        BackgroundCleanupStatus::Completed => format_optional_ms(snapshot.ollama_cleanup_ms),
        BackgroundCleanupStatus::Failed => format!(
            "Failed after {}",
            format_optional_ms(snapshot.ollama_cleanup_ms)
        ),
    }
}

fn duration_ms_between(start: Instant, end: Instant) -> u128 {
    end.saturating_duration_since(start).as_millis()
}

pub fn handle_hotkey(app: AppHandle, state: &str) -> Result<(), String> {
    let runtime = app.state::<NativeDictationRuntime>();
    let command_tx = runtime
        .command_tx
        .lock()
        .map_err(|_| "Native dictation command lock was poisoned.".to_string())?
        .clone();

    command_tx
        .send(RecorderCommand {
            app: app.clone(),
            state: state.to_string(),
            received_at: Instant::now(),
        })
        .map_err(|error| format!("Could not send native dictation command: {error}"))
}

fn recorder_loop(command_rx: mpsc::Receiver<RecorderCommand>) {
    let mut session: Option<RecordingSession> = None;

    for command in command_rx {
        let result = match command.state.as_str() {
            "pressed" => {
                if let Some(active_session) = session.as_ref() {
                    if should_toggle_stop(active_session.started_at.elapsed()) {
                        let active_session = session
                            .take()
                            .expect("active recording session should exist");
                        finish_recording(command.app.clone(), active_session, command.received_at)
                    } else {
                        Ok(())
                    }
                } else {
                    start_recording(&command.app, &mut session, command.received_at)
                }
            }
            "released" => {
                if let Some(active_session) = session.as_ref() {
                    if should_ignore_quick_release(active_session.started_at.elapsed()) {
                        tracing::info!("quick hotkey release ignored; continuing tap-to-dictate");
                        Ok(())
                    } else {
                        let active_session = session
                            .take()
                            .expect("active recording session should exist");
                        finish_recording(command.app.clone(), active_session, command.received_at)
                    }
                } else {
                    Ok(())
                }
            }
            "auto_stop" => {
                if let Some(session) = session.take() {
                    finish_recording(command.app.clone(), session, command.received_at)
                } else {
                    Ok(())
                }
            }
            "cancel" => {
                // Stop any live recording and invalidate the current session so an
                // in-flight transcription/refinement worker aborts before pasting.
                if let Some(active_session) = session.take() {
                    let _ = active_session.level_stop_tx.send(());
                    drop(active_session.stream);
                }
                set_escape_cancel(&command.app, false);
                invalidate_session(&command.app);
                emit_status(&command.app, "cancelled", "Cancelled");
                Ok(())
            }
            _ => Ok(()),
        };

        if let Err(error) = result {
            tracing::warn!(error = %error, "native dictation command failed");
            emit_status(&command.app, "error", &error);
        }
    }
}

fn should_ignore_quick_release(elapsed: Duration) -> bool {
    elapsed < Duration::from_millis(QUICK_TAP_RELEASE_MS)
}

fn should_toggle_stop(elapsed: Duration) -> bool {
    elapsed >= Duration::from_millis(QUICK_TAP_RELEASE_MS)
}

fn start_recording(
    app: &AppHandle,
    session: &mut Option<RecordingSession>,
    hotkey_received_at: Instant,
) -> Result<(), String> {
    if session.is_some() {
        return Ok(());
    }

    // Capture the window that had focus when dictation started. Insertion is revalidated
    // against this before pasting so a transcript never lands in a different window (spec
    // §6.1/§6.2). Captured before device setup to stay close to the hotkey-press instant.
    let target = foreground_target();
    if target.is_none() {
        tracing::warn!("could not capture a foreground insertion target at record start");
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default microphone is available.".to_string())?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| "Unknown input device".to_string());
    let supported_config = device
        .default_input_config()
        .map_err(|error| format!("Could not read default microphone config: {error}"))?;
    let sample_rate = supported_config.sample_rate().0;
    let channels = supported_config.channels();
    let sample_format = supported_config.sample_format();
    let stream_config: StreamConfig = supported_config.into();
    let samples = Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize * 20)));
    let last_voice_at = Arc::new(Mutex::new(None));

    let stream = build_stream(&device, &stream_config, sample_format, samples.clone())?;
    stream
        .play()
        .map_err(|error| format!("Could not start microphone capture: {error}"))?;
    let recording_started_at = Instant::now();
    let command_tx = app
        .state::<NativeDictationRuntime>()
        .command_tx
        .lock()
        .map_err(|_| "Native dictation command lock was poisoned.".to_string())?
        .clone();
    let level_stop_tx = start_level_meter(
        app.clone(),
        samples.clone(),
        sample_rate,
        last_voice_at.clone(),
        command_tx,
    );

    // Assign and activate a fresh session id; this supersedes any worker still
    // processing a previous recording so it cannot insert stale text.
    let session_id = begin_session(app);

    *session = Some(RecordingSession {
        session_id,
        target,
        stream,
        samples,
        sample_rate,
        channels,
        device_name: device_name.clone(),
        sample_format,
        hotkey_received_at,
        started_at: recording_started_at,
        last_voice_at,
        level_stop_tx,
    });

    emit_status(app, "listening", "Listening");
    set_escape_cancel(app, true);
    let warm_model = {
        let settings = app
            .state::<crate::app_state::LocalFlowRuntime>()
            .current_settings();
        if settings.models.ollama_model.trim().is_empty() {
            OLLAMA_MODEL.to_string()
        } else {
            settings.models.ollama_model.trim().to_string()
        }
    };
    warm_ollama_in_background(warm_model);
    tracing::info!(
        device = %device_name,
        sample_rate,
        channels,
        sample_format = ?sample_format,
        "native microphone recording started"
    );
    Ok(())
}

fn finish_recording(
    app: AppHandle,
    session: RecordingSession,
    stop_requested_at: Instant,
) -> Result<(), String> {
    let capture_stop_started_at = Instant::now();
    let RecordingSession {
        session_id,
        target,
        stream,
        samples,
        sample_rate,
        channels,
        device_name,
        sample_format,
        hotkey_received_at,
        started_at,
        last_voice_at,
        level_stop_tx,
    } = session;

    let _ = level_stop_tx.send(());
    drop(stream);
    set_escape_cancel(&app, false);
    emit_status(&app, "processing", "Transcribing with local whisper.cpp");

    let captured = samples
        .lock()
        .map_err(|_| "Captured audio lock was poisoned.".to_string())?
        .clone();
    let capture_stop_ms = capture_stop_started_at.elapsed().as_millis();
    let speech_ended_at = last_voice_at.lock().map(|guard| *guard).unwrap_or(None);

    if captured.len() < sample_rate as usize / 4 {
        return Err("Recording was too short to transcribe.".to_string());
    }

    let stats = analyze_audio(&captured, sample_rate);
    tracing::info!(
        device = %device_name,
        sample_rate,
        channels,
        sample_format = ?sample_format,
        duration_secs = stats.duration_secs,
        peak = stats.peak,
        rms = stats.rms,
        nonzero_ratio = stats.nonzero_ratio,
        "native microphone recording finished"
    );

    if is_near_silent(stats) {
        return Err(format!(
            "Captured audio from \"{device_name}\" was silent or near-silent. peak={:.5}, rms={:.5}. Check the Windows default input device, microphone privacy access, and input gain.",
            stats.peak, stats.rms
        ));
    }

    // Run the transcribe -> quick insert -> background refine tail off the recorder thread so the
    // recorder stays responsive and a newer or cancelled session can supersede this one.
    // The worker revalidates its session id before every side effect and, crucially, never
    // pastes if it is no longer the current session (spec §4.4).
    thread::spawn(move || {
        match process_session(
            &app,
            &captured,
            sample_rate,
            channels,
            session_id,
            target,
            hotkey_received_at,
            started_at,
            speech_ended_at,
            stop_requested_at,
            capture_stop_ms,
        ) {
            Ok(()) => {}
            Err(error) => {
                if session_is_current(&app, session_id) {
                    tracing::warn!(error = %error, "native dictation processing failed");
                    emit_status(&app, "error", &error);
                } else {
                    tracing::info!(
                        error = %error,
                        "native dictation error suppressed for superseded session"
                    );
                }
            }
        }
    });

    Ok(())
}

/// Transcribe -> deterministic insert -> background refine. Runs on a worker thread.
/// Revalidates the session before every side effect; a superseded or cancelled session aborts
/// without pasting.
fn process_session(
    app: &AppHandle,
    captured: &[f32],
    sample_rate: u32,
    channels: u16,
    session_id: u64,
    target: Option<TargetWindow>,
    hotkey_received_at: Instant,
    started_at: Instant,
    speech_ended_at: Option<Instant>,
    stop_requested_at: Instant,
    capture_stop_ms: u128,
) -> Result<(), String> {
    if !session_is_current(app, session_id) {
        emit_cancelled(app);
        return Ok(());
    }

    // Read the user's current settings so the native path honors their model choice,
    // dictionary (ASR biasing), replacements, and snippets instead of hardcoded defaults.
    let settings = app
        .state::<crate::app_state::LocalFlowRuntime>()
        .current_settings();
    let dictionary_terms: Vec<String> = settings
        .dictionary
        .iter()
        .map(|entry| entry.phrase.clone())
        .filter(|phrase| !phrase.trim().is_empty())
        .collect();
    let ollama_model = if settings.models.ollama_model.trim().is_empty() {
        OLLAMA_MODEL.to_string()
    } else {
        settings.models.ollama_model.trim().to_string()
    };

    let output_dir = env::temp_dir().join("localflow");
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Could not create temporary audio directory: {error}"))?;
    let stamp = chrono::Utc::now().timestamp_millis();
    let wav_path = output_dir.join(format!("dictation-{stamp}.wav"));
    let output_base = output_dir.join(format!("dictation-{stamp}"));

    let resample_started_at = Instant::now();
    let mono_16k = resample_to_16khz(captured, sample_rate);
    let resample_ms = resample_started_at.elapsed().as_millis();

    let wav_write_started_at = Instant::now();
    write_wav(&wav_path, &mono_16k)?;
    let wav_write_ms = wav_write_started_at.elapsed().as_millis();

    let whisper_started_at = Instant::now();
    let transcribe_result = run_whisper(app, &wav_path, &output_base, &dictionary_terms);
    let whisper_ms = whisper_started_at.elapsed().as_millis();
    let _ = fs::remove_file(&wav_path);
    let _ = fs::remove_file(output_base.with_extension("json"));
    let transcript = transcribe_result?;

    if transcript.trim().is_empty() || is_blank_transcript(&transcript) {
        return Err("Local whisper.cpp did not return any transcript text.".to_string());
    }

    if !session_is_current(app, session_id) {
        emit_cancelled(app);
        return Ok(());
    }

    // Deterministic formatting (spoken punctuation, self-corrections, filler/stutter
    // cleanup) is applied before the LLM. It seeds the cleanup prompt and is the safe
    // fallback when the model is unavailable. Never let it collapse a non-empty transcript
    // to nothing (e.g. an all-filler utterance) — fall back to the raw transcript then.
    let deterministic_started_at = Instant::now();
    let replacements: Vec<crate::transcript::Replacement> = settings
        .replacements
        .iter()
        .map(|rule| crate::transcript::Replacement {
            incorrect: rule.incorrect.clone(),
            correct: rule.correct.clone(),
            enabled: rule.enabled,
        })
        .collect();
    let snippets: Vec<crate::transcript::Snippet> = settings
        .snippets
        .iter()
        .map(|snippet| crate::transcript::Snippet {
            trigger: snippet.trigger.clone(),
            expansion: snippet.expansion.clone(),
            enabled: snippet.enabled,
        })
        .collect();

    let deterministic = {
        let formatted = crate::transcript::apply_deterministic_formatting_with(
            &transcript,
            &replacements,
            &snippets,
        );
        if formatted.trim().is_empty() {
            transcript.clone()
        } else {
            formatted
        }
    };
    let deterministic_ms = deterministic_started_at.elapsed().as_millis();

    // Remember the quick transcript before attempting insertion so it is recoverable
    // (copy / paste-last) even if the paste is skipped because focus changed.
    store_last_transcript(app, &deterministic);

    let background_cleanup_status = if settings.models.low_resource_mode {
        tracing::info!("low_resource_mode enabled; background LLM cleanup skipped");
        BackgroundCleanupStatus::Skipped
    } else {
        BackgroundCleanupStatus::Pending
    };

    // Final guard before the only irreversible side effect: never paste into whatever is
    // focused now if this session was superseded or cancelled while we were working.
    if !session_is_current(app, session_id) {
        emit_cancelled(app);
        return Ok(());
    }

    // Revalidate the insertion target. If focus moved to another window (or we cannot
    // confirm the original), do not paste — a Ctrl+V would land in the wrong app, possibly
    // a password or unrelated field. Fail closed and tell the user (spec §6.2). The
    // transcript is kept for recovery via "Copy last transcript".
    if !target_matches(target, foreground_target()) {
        tracing::warn!("insertion target changed since dictation started; skipping paste");
        emit_status(
            app,
            "error",
            "Focus changed before insertion. The transcript was not pasted; use \"Copy last transcript\" to recover it.",
        );
        return Ok(());
    }

    let paste = paste_text_for_visible_insert(&deterministic)?;
    let visible_at = paste.timings.visible_at;
    let speech_end_reference = speech_ended_at.unwrap_or(stop_requested_at);

    record_latency_snapshot(
        app,
        NativeLatencySnapshot {
            session_id,
            ollama_model: ollama_model.clone(),
            low_resource_mode: settings.models.low_resource_mode,
            audio_duration_ms: audio_duration_ms(captured.len(), sample_rate),
            hotkey_to_recording_start_ms: Some(duration_ms_between(hotkey_received_at, started_at)),
            vad_tail_ms: speech_ended_at
                .map(|speech_ended_at| duration_ms_between(speech_ended_at, stop_requested_at)),
            capture_stop_ms: Some(capture_stop_ms),
            resample_ms: Some(resample_ms),
            wav_write_ms: Some(wav_write_ms),
            whisper_ms: Some(whisper_ms),
            deterministic_ms: Some(deterministic_ms),
            paste_set_clipboard_ms: Some(paste.timings.set_clipboard_ms),
            paste_send_ms: Some(paste.timings.send_paste_ms),
            clipboard_restore_delay_ms: None,
            speech_end_to_visible_ms: Some(duration_ms_between(speech_end_reference, visible_at)),
            recording_start_to_insert_ms: Some(duration_ms_between(started_at, visible_at)),
            ollama_cleanup_ms: None,
            background_cleanup_status,
        },
    );

    emit_status(app, "inserted", "Inserted transcript");
    schedule_clipboard_restore(app.clone(), session_id, paste.previous_text);

    if !settings.models.low_resource_mode {
        start_background_cleanup(
            app.clone(),
            session_id,
            transcript,
            deterministic.clone(),
            ollama_model.clone(),
        );
    }

    tracing::info!(
        elapsed_ms = duration_ms_between(started_at, visible_at),
        speech_end_to_visible_ms = duration_ms_between(speech_end_reference, visible_at),
        whisper_ms,
        chars = deterministic.chars().count(),
        sample_rate,
        channels,
        model = %ollama_model,
        cleanup = %background_cleanup_status.label(),
        "native dictation inserted quick transcript"
    );

    Ok(())
}

fn start_background_cleanup(
    app: AppHandle,
    session_id: u64,
    transcript: String,
    deterministic: String,
    ollama_model: String,
) {
    thread::spawn(move || {
        let cleanup_started_at = Instant::now();
        let result = refine_with_pinned_ollama(&transcript, &deterministic, &ollama_model);
        let cleanup_ms = cleanup_started_at.elapsed().as_millis();

        match result {
            Ok(refined) => {
                if session_is_current(&app, session_id) {
                    store_last_transcript(&app, &refined);
                }
                update_latency_snapshot(&app, session_id, |snapshot| {
                    snapshot.ollama_cleanup_ms = Some(cleanup_ms);
                    snapshot.background_cleanup_status = BackgroundCleanupStatus::Completed;
                });
                tracing::info!(
                    session_id,
                    cleanup_ms,
                    model = %ollama_model,
                    "native background cleanup completed"
                );
            }
            Err(error) => {
                update_latency_snapshot(&app, session_id, |snapshot| {
                    snapshot.ollama_cleanup_ms = Some(cleanup_ms);
                    snapshot.background_cleanup_status = BackgroundCleanupStatus::Failed;
                });
                tracing::warn!(
                    session_id,
                    cleanup_ms,
                    model = %ollama_model,
                    error = %error,
                    "native background cleanup failed; quick transcript remains inserted"
                );
            }
        }
    });
}

fn emit_cancelled(app: &AppHandle) {
    tracing::info!("native dictation session superseded or cancelled before insertion");
    emit_status(app, "cancelled", "Cancelled");
}

fn start_level_meter(
    app: AppHandle,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    last_voice_at: Arc<Mutex<Option<Instant>>>,
    command_tx: mpsc::Sender<RecorderCommand>,
) -> mpsc::Sender<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut cursor = 0usize;
        let mut detector = EndOfSpeechDetector::new(Instant::now());

        loop {
            thread::sleep(Duration::from_millis(VAD_POLL_MS));
            if stop_rx.try_recv().is_ok() {
                break;
            }

            let chunk = match samples.lock() {
                Ok(output) => {
                    let start = cursor.min(output.len());
                    let next = output[start..].to_vec();
                    cursor = output.len();
                    next
                }
                Err(_) => Vec::new(),
            };

            if chunk.is_empty() {
                continue;
            }

            let rms = rms_level(&chunk);
            let features = overlay_audio_features(&chunk, sample_rate, rms);
            emit_audio_features(&app, features);

            if detector.observe(Instant::now(), rms) {
                if let Ok(mut last_voice) = last_voice_at.lock() {
                    *last_voice = detector.last_voice_at;
                }
                // If speech was never detected, this is an abandoned tap: cancel silently
                // (hides the overlay) instead of running the pipeline and flashing an error.
                let state = if detector.speech_seen {
                    "auto_stop"
                } else {
                    "cancel"
                };
                let _ = command_tx.send(RecorderCommand {
                    app: app.clone(),
                    state: state.to_string(),
                    received_at: Instant::now(),
                });
                break;
            }

            if let Ok(mut last_voice) = last_voice_at.lock() {
                *last_voice = detector.last_voice_at;
            }
        }
    });

    stop_tx
}

#[derive(Debug)]
struct EndOfSpeechDetector {
    started_at: Instant,
    speech_started_at: Option<Instant>,
    last_voice_at: Option<Instant>,
    speech_seen: bool,
}

impl EndOfSpeechDetector {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            speech_started_at: None,
            last_voice_at: None,
            speech_seen: false,
        }
    }

    fn observe(&mut self, now: Instant, rms: f32) -> bool {
        let elapsed = now.duration_since(self.started_at);

        if elapsed >= Duration::from_secs(MAX_RECORDING_SECS) {
            return true;
        }

        if !self.speech_seen && elapsed >= Duration::from_millis(NO_SPEECH_TIMEOUT_MS) {
            return true;
        }

        let voice_threshold = if self.speech_seen {
            VAD_CONTINUE_RMS_THRESHOLD
        } else {
            VAD_START_RMS_THRESHOLD
        };

        if rms >= voice_threshold {
            let speech_started_at = self.speech_started_at.get_or_insert(now);
            self.last_voice_at = Some(now);

            if now.duration_since(*speech_started_at)
                >= Duration::from_millis(VAD_CONFIRM_SPEECH_MS)
                || rms >= VAD_START_RMS_THRESHOLD * 1.6
            {
                self.speech_seen = true;
            }
        } else if !self.speech_seen {
            self.speech_started_at = None;
        }

        self.speech_seen
            && now.duration_since(self.started_at)
                >= Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS)
            && self.last_voice_at.is_some_and(|last_voice_at| {
                now.duration_since(last_voice_at) >= Duration::from_millis(END_OF_SPEECH_TIMEOUT_MS)
            })
    }
}

fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn display_level_from_rms(rms: f32) -> f32 {
    (rms * 15.0).clamp(0.05, 1.0)
}

fn overlay_audio_features(samples: &[f32], sample_rate: u32, rms: f32) -> OverlayAudioFeatures {
    OverlayAudioFeatures {
        level: display_level_from_rms(rms),
        pitch: estimate_pitch_normalized(samples, sample_rate, rms).unwrap_or(0.5),
        brightness: estimate_brightness(samples, rms),
    }
}

fn estimate_pitch_normalized(samples: &[f32], sample_rate: u32, rms: f32) -> Option<f32> {
    if rms < VAD_CONTINUE_RMS_THRESHOLD || sample_rate == 0 {
        return None;
    }

    let min_hz = 75.0f32;
    let max_hz = 420.0f32;
    let min_lag = (sample_rate as f32 / max_hz).round().max(1.0) as usize;
    let max_lag = (sample_rate as f32 / min_hz).round() as usize;

    if samples.len() <= max_lag + 2 {
        return None;
    }

    let mean = samples.iter().copied().sum::<f32>() / samples.len() as f32;
    let centered: Vec<f32> = samples.iter().map(|sample| sample - mean).collect();
    let mut best_lag = 0usize;
    let mut best_score = 0.0f32;
    let mut scores = Vec::with_capacity(max_lag.saturating_sub(min_lag) + 1);

    for lag in min_lag..=max_lag {
        let mut correlation = 0.0f32;
        let mut left_energy = 0.0f32;
        let mut right_energy = 0.0f32;

        for index in lag..centered.len() {
            let left = centered[index];
            let right = centered[index - lag];
            correlation += left * right;
            left_energy += left * left;
            right_energy += right * right;
        }

        let energy = (left_energy * right_energy).sqrt();
        if energy <= f32::EPSILON {
            continue;
        }

        let score = correlation / energy;
        scores.push((lag, score));
        if score > best_score {
            best_score = score;
            best_lag = lag;
        }
    }

    if best_score < 0.34 || best_lag == 0 {
        return None;
    }

    let strong_score = best_score * 0.9;
    if let Some((lag, _)) = scores
        .iter()
        .find(|(_, score)| *score >= strong_score)
        .copied()
    {
        best_lag = lag;
    }

    let hz = sample_rate as f32 / best_lag as f32;
    Some(((hz - min_hz) / (max_hz - min_hz)).clamp(0.0, 1.0))
}

fn estimate_brightness(samples: &[f32], rms: f32) -> f32 {
    if samples.len() < 2 || rms < VAD_CONTINUE_RMS_THRESHOLD {
        return 0.35;
    }

    let crossings = samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0 && pair[1] >= 0.0) || (pair[0] >= 0.0 && pair[1] < 0.0))
        .count();

    (crossings as f32 / samples.len() as f32 * 12.0).clamp(0.12, 1.0)
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<Stream, String> {
    let channels = config.channels as usize;
    let error_callback = |error| tracing::warn!(error = %error, "microphone stream error");

    match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| push_mono_samples(data, channels, &samples),
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
                    push_mono_samples(&converted, channels, &samples);
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
                    push_mono_samples(&converted, channels, &samples);
                },
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build u16 microphone stream: {error}")),
        other => Err(format!("Unsupported microphone sample format: {other:?}")),
    }
}

fn push_mono_samples(data: &[f32], channels: usize, samples: &Arc<Mutex<Vec<f32>>>) {
    if channels == 0 {
        return;
    }

    if let Ok(mut output) = samples.lock() {
        output.extend(downmix_input_callback(data, channels));
    }
}

fn downmix_input_callback(data: &[f32], channels: usize) -> Vec<f32> {
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

    let selected_channel = loudest_channel(data, channels);
    data.chunks(channels)
        .filter_map(|frame| frame.get(selected_channel).copied())
        .map(|sample| sample.clamp(-1.0, 1.0))
        .collect()
}

fn loudest_channel(data: &[f32], channels: usize) -> usize {
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

    selected
}

fn analyze_audio(samples: &[f32], sample_rate: u32) -> AudioStats {
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

fn is_near_silent(stats: AudioStats) -> bool {
    stats.peak < SILENCE_PEAK_THRESHOLD && stats.rms < SILENCE_RMS_THRESHOLD
}

fn resample_to_16khz(input: &[f32], input_rate: u32) -> Vec<f32> {
    const TARGET_RATE: u32 = 16_000;
    if input_rate == TARGET_RATE {
        return input.to_vec();
    }

    let output_len = ((input.len() as f64 / input_rate as f64) * TARGET_RATE as f64)
        .round()
        .max(1.0) as usize;
    let ratio = input_rate as f64 / TARGET_RATE as f64;

    (0..output_len)
        .map(|index| {
            let source = index as f64 * ratio;
            let lower = source.floor() as usize;
            let upper = (lower + 1).min(input.len().saturating_sub(1));
            let fraction = (source - lower as f64) as f32;
            let a = input.get(lower).copied().unwrap_or(0.0);
            let b = input.get(upper).copied().unwrap_or(a);
            a + (b - a) * fraction
        })
        .collect()
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|error| format!("WAV write failed: {error}"))?;

    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        writer
            .write_sample(pcm)
            .map_err(|error| format!("WAV sample write failed: {error}"))?;
    }

    writer
        .finalize()
        .map_err(|error| format!("WAV finalize failed: {error}"))
}

fn run_whisper(
    app: &AppHandle,
    wav_path: &Path,
    output_base: &Path,
    dictionary_terms: &[String],
) -> Result<String, String> {
    let cli = whisper_cli_path(app)?;
    let model = whisper_model_path(app)?;
    let thread_count = whisper_thread_count().to_string();
    let whisper_dir = cli
        .parent()
        .ok_or_else(|| "Could not resolve whisper.cpp runtime directory.".to_string())?;

    let mut command = hidden_sidecar_command(&cli);
    command
        .current_dir(whisper_dir)
        .stdin(Stdio::null())
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(wav_path)
        .arg("-oj")
        .arg("-of")
        .arg(output_base)
        .arg("-np")
        .arg("-nt")
        .arg("-l")
        .arg("en")
        .arg("-t")
        .arg(&thread_count);

    // Bias recognition toward the user's dictionary terms via whisper's initial prompt.
    if let Some(prompt) = build_whisper_prompt(dictionary_terms) {
        command.arg("--prompt").arg(prompt);
    }

    let output = command
        .output()
        .map_err(|error| format!("Could not start whisper.cpp: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "whisper.cpp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    parse_whisper_json(&output_base.with_extension("json"))
}

/// Build a bounded whisper initial prompt from the user's dictionary terms. Whisper's
/// prompt window is small, so we cap the joined length.
fn build_whisper_prompt(terms: &[String]) -> Option<String> {
    const MAX_PROMPT_CHARS: usize = 800;

    let mut joined = String::new();
    for term in terms
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
    {
        let addition = if joined.is_empty() {
            term.to_string()
        } else {
            format!(", {term}")
        };
        if joined.len() + addition.len() > MAX_PROMPT_CHARS {
            break;
        }
        joined.push_str(&addition);
    }

    if joined.is_empty() {
        None
    } else {
        Some(format!("Glossary: {joined}."))
    }
}

fn hidden_sidecar_command(program: &Path) -> Command {
    let mut command = Command::new(program);

    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);

    command
}

fn whisper_thread_count() -> usize {
    thread::available_parallelism()
        .map(|available| available.get().saturating_sub(1).max(2))
        .unwrap_or(4)
        .clamp(2, 8)
}

fn parse_whisper_json(path: &Path) -> Result<String, String> {
    let payload = fs::read_to_string(path)
        .map_err(|error| format!("Could not read whisper.cpp JSON output: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|error| format!("Could not parse whisper.cpp JSON output: {error}"))?;

    if let Some(text) = value.get("text").and_then(|item| item.as_str()) {
        return Ok(text.trim().to_string());
    }

    if let Some(segments) = value.get("transcription").and_then(|item| item.as_array()) {
        let text = segments
            .iter()
            .filter_map(|segment| segment.get("text").and_then(|item| item.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        return Ok(text.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    Err("whisper.cpp JSON output did not include transcript text.".to_string())
}

fn is_blank_transcript(transcript: &str) -> bool {
    let normalized = transcript
        .trim()
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '[' | ']' | '(' | ')' | '{' | '}' | '.' | '!' | '?'
                )
        })
        .to_ascii_lowercase()
        .replace(['_', '-'], " ");

    matches!(
        normalized.as_str(),
        "" | "blank audio" | "silence" | "no speech" | "inaudible"
    )
}

fn refine_with_pinned_ollama(
    raw_transcript: &str,
    deterministic_text: &str,
    model: &str,
) -> Result<String, String> {
    let first_prompt = build_cleanup_prompt(raw_transcript, deterministic_text);
    let first_payload = request_ollama_cleanup(&first_prompt, model)?;

    match parse_cleanup_text(&first_payload) {
        Ok(text) => Ok(text),
        Err(first_error) => {
            let repair_prompt = build_repair_prompt(&first_payload, &first_error);
            let repaired_payload = request_ollama_cleanup(&repair_prompt, model)?;
            parse_cleanup_text(&repaired_payload).map_err(|repair_error| {
                format!(
                    "{model} returned invalid cleanup JSON twice: {first_error}; {repair_error}"
                )
            })
        }
    }
}

fn request_ollama_cleanup(prompt: &str, model: &str) -> Result<String, String> {
    // Bounded so a slow/oversized model falls back to the deterministic transcript quickly
    // instead of freezing the overlay on "refining".
    request_ollama_generate(prompt, Duration::from_secs(20), 0.1, model)
}

fn request_ollama_generate(
    prompt: &str,
    timeout: Duration,
    temperature: f32,
    model: &str,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| format!("Could not create local Ollama client: {error}"))?;

    let response = client
        .post(OLLAMA_GENERATE_URL)
        .json(&OllamaGenerateRequest {
            model,
            prompt,
            stream: false,
            format: "json",
            keep_alive: OLLAMA_KEEP_ALIVE,
            options: OllamaOptions { temperature },
        })
        .send()
        .map_err(|error| {
            format!("Could not reach local Ollama at {OLLAMA_GENERATE_URL}: {error}")
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        if status.as_u16() == 404 {
            return Err(format!(
                "Ollama model \"{model}\" was not found locally. {body}"
            ));
        }
        return Err(format!("Ollama returned HTTP {status}: {body}"));
    }

    let payload: OllamaGenerateResponse = response
        .json()
        .map_err(|error| format!("Ollama returned invalid JSON: {error}"))?;

    Ok(payload.response)
}

fn warm_ollama_in_background(model: String) {
    thread::spawn(move || {
        let prompt = serde_json::json!({
            "task": "localflow.warm_dictation_cleanup",
            "instruction": "Return only JSON.",
            "requiredShape": {
                "text": "ready",
                "confidence": 1.0,
                "resolved_corrections": [],
                "warnings": []
            }
        })
        .to_string();

        if let Err(error) = request_ollama_generate(&prompt, Duration::from_secs(12), 0.0, &model) {
            tracing::debug!(error = %error, model = %model, "ollama warmup skipped");
        }
    });
}

fn parse_cleanup_text(payload: &str) -> Result<String, String> {
    let value = parse_json_value(payload)?;
    let cleanup: CleanupModelResponse = serde_json::from_value(value)
        .map_err(|error| format!("Cleanup payload did not match the strict contract: {error}"))?;
    let text = cleanup.text.trim();

    if text.is_empty() {
        Err("Cleanup payload returned empty text.".to_string())
    } else {
        Ok(text.to_string())
    }
}

fn parse_json_value(payload: &str) -> Result<serde_json::Value, String> {
    let trimmed = payload.trim();
    serde_json::from_str(trimmed).or_else(|first_error| {
        let start = trimmed.find('{');
        let end = trimmed.rfind('}');

        match (start, end) {
            (Some(start), Some(end)) if start < end => serde_json::from_str(&trimmed[start..=end])
                .map_err(|second_error| {
                    format!("Invalid JSON: {first_error}; extraction failed: {second_error}")
                }),
            _ => Err(format!("Invalid JSON: {first_error}")),
        }
    })
}

fn build_cleanup_prompt(raw_transcript: &str, deterministic_text: &str) -> String {
    serde_json::json!({
        "task": "localflow.dictation_cleanup",
        "contract": "Return only strict JSON with text, confidence, resolved_corrections, and warnings.",
        "rules": [
            "Start from deterministicText: it already has spoken punctuation, self-corrections, and filler cleanup applied. Preserve its wording and formatting unless it is clearly wrong.",
            "Preserve deterministicText capitalization, punctuation, line breaks, and technical casing unless rawTranscript clearly proves they are wrong.",
            "Preserve meaning, facts, names, numbers, uncertainty, and intent.",
            "Never answer the dictated content.",
            "Never add new claims.",
            "Remove filler words only when meaning is unchanged.",
            "Resolve explicit self-corrections in favor of the latest correction.",
            "Keep code identifiers, file paths, URLs, and email addresses verbatim.",
            "Add punctuation and capitalization conservatively.",
            "Return only JSON."
        ],
        "cleanupLevel": "balanced",
        "deterministicText": deterministic_text,
        "rawTranscript": raw_transcript
    })
    .to_string()
}

fn build_repair_prompt(invalid_payload: &str, error: &str) -> String {
    serde_json::json!({
        "task": "localflow.repair_cleanup_json",
        "instruction": "Convert the invalid payload to strict JSON only. Do not change the intended cleaned text.",
        "error": error,
        "invalidPayload": invalid_payload,
        "requiredShape": {
            "text": "final text",
            "confidence": 0.0,
            "resolved_corrections": [],
            "warnings": []
        }
    })
    .to_string()
}

fn paste_text_for_visible_insert(text: &str) -> Result<PasteStart, String> {
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
    let previous_text = clipboard.get_text().ok();
    let set_clipboard_started_at = Instant::now();
    clipboard
        .set_text(text.to_string())
        .map_err(|error| format!("Could not set clipboard text: {error}"))?;
    let set_clipboard_ms = set_clipboard_started_at.elapsed().as_millis();

    let send_paste_started_at = Instant::now();
    send_ctrl_v()?;
    let send_paste_ms = send_paste_started_at.elapsed().as_millis();
    let visible_at = Instant::now();

    Ok(PasteStart {
        previous_text,
        timings: PasteTimings {
            set_clipboard_ms,
            send_paste_ms,
            visible_at,
        },
    })
}

fn schedule_clipboard_restore(app: AppHandle, session_id: u64, previous_text: Option<String>) {
    thread::spawn(move || {
        let restore_started_at = Instant::now();
        // Give the target app time to read the pasted clipboard before we restore the prior
        // contents. This happens after visible insertion and is excluded from the main
        // speech-end-to-visible-text latency.
        thread::sleep(Duration::from_millis(400));

        if let Some(previous_text) = previous_text {
            if let Ok(mut clipboard) = Clipboard::new() {
                let _ = clipboard.set_text(previous_text);
            }
        }

        update_latency_snapshot(&app, session_id, |snapshot| {
            snapshot.clipboard_restore_delay_ms = Some(restore_started_at.elapsed().as_millis());
        });
    });
}

fn audio_duration_ms(samples: usize, sample_rate: u32) -> u128 {
    if sample_rate == 0 {
        return 0;
    }

    ((samples as f64 / sample_rate as f64) * 1000.0).round() as u128
}

fn send_ctrl_v() -> Result<(), String> {
    let inputs = [
        key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];

    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(format!(
            "Could only send {sent} of {} key events.",
            inputs.len()
        ))
    }
}

fn key_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn whisper_cli_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("LOCALFLOW_WHISPER_CLI") {
        return require_file(PathBuf::from(path), "whisper.cpp executable");
    }

    find_file(
        "whisper.cpp executable",
        runtime_candidates(app).into_iter().map(|runtime| {
            runtime
                .join("whisper")
                .join("Release")
                .join("whisper-cli.exe")
        }),
    )
}

fn whisper_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("LOCALFLOW_WHISPER_MODEL") {
        return require_file(PathBuf::from(path), "Whisper model");
    }

    find_file(
        "Whisper model",
        runtime_candidates(app)
            .into_iter()
            .map(|runtime| runtime.join("models").join("ggml-tiny.en-q5_1.bin")),
    )
}

fn runtime_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("localflow-runtime"));
    }

    candidates.push(workspace_runtime_dir());
    candidates
}

fn workspace_runtime_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".localflow-runtime")
}

fn require_file(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("Missing {label}: {}", path.display()))
    }
}

fn find_file(
    label: &str,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Result<PathBuf, String> {
    let mut checked = Vec::new();

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
        checked.push(candidate.display().to_string());
    }

    Err(format!("Missing {label}. Checked: {}", checked.join("; ")))
}

fn emit_status(app: &AppHandle, phase: &str, message: &str) {
    emit_native_event(app, phase, message, None);
}

fn emit_audio_features(app: &AppHandle, features: OverlayAudioFeatures) {
    emit_native_event(app, "listening", "Listening", Some(features));
}

fn emit_native_event(
    app: &AppHandle,
    phase: &str,
    message: &str,
    features: Option<OverlayAudioFeatures>,
) {
    let epoch = app
        .state::<NativeDictationRuntime>()
        .overlay_epoch
        .fetch_add(1, Ordering::SeqCst)
        + 1;

    match phase {
        "listening" | "processing" | "refining" | "inserted" | "error" => {
            show_overlay(app, features.is_none());
        }
        _ => hide_overlay(app),
    }

    let _ = app.emit(
        "localflow://native-dictation",
        NativeDictationEvent {
            phase: phase.to_string(),
            message: message.to_string(),
            level: features.map(|features| features.level),
            pitch: features.map(|features| features.pitch),
            brightness: features.map(|features| features.brightness),
        },
    );

    if matches!(phase, "inserted" | "error") && features.is_none() {
        schedule_overlay_hide(app.clone(), epoch);
    }
}

fn show_overlay(app: &AppHandle, reposition: bool) {
    if let Some(window) = app.get_webview_window("overlay") {
        if reposition {
            if let Err(error) = position_overlay(&window) {
                tracing::warn!(error = %error, "could not position voice overlay");
            }
        }
        let _ = window.show();
    }
}

fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.hide();
    }
}

fn schedule_overlay_hide(app: AppHandle, epoch: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(700));

        if app
            .state::<NativeDictationRuntime>()
            .overlay_epoch
            .load(Ordering::SeqCst)
            == epoch
        {
            hide_overlay(&app);
        }
    });
}

fn position_overlay(window: &WebviewWindow) -> tauri::Result<()> {
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
fn overlay_work_area(window: &WebviewWindow) -> Option<(i32, i32)> {
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
fn overlay_work_area(_: &WebviewWindow) -> Option<(i32, i32)> {
    None
}

fn overlay_position_for_work_area(
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
    fn latency_duration_helpers_compute_milliseconds() {
        let started_at = Instant::now();
        let ended_at = started_at + Duration::from_millis(42);

        assert_eq!(duration_ms_between(started_at, ended_at), 42);
        assert_eq!(audio_duration_ms(48_000, 48_000), 1_000);
    }

    #[test]
    fn latency_diagnostics_show_not_measured_before_first_dictation() {
        let rows = latency_diagnostics_for(None);

        let visible = diagnostic_row(&rows, "Latency: speech end to visible text");
        assert_eq!(visible.value, "Not measured");
        assert_eq!(visible.status, "warning");
    }

    #[test]
    fn latency_diagnostics_apply_visible_and_whisper_thresholds() {
        let mut snapshot = latency_snapshot();
        snapshot.speech_end_to_visible_ms = Some(2_100);
        snapshot.whisper_ms = Some(900);

        let rows = latency_diagnostics_for(Some(&snapshot));

        assert_eq!(
            diagnostic_row(&rows, "Latency: speech end to visible text").status,
            "error"
        );
        assert_eq!(
            diagnostic_row(&rows, "Latency: Whisper sidecar").status,
            "warning"
        );
    }

    #[test]
    fn latency_snapshots_keep_only_the_last_five() {
        let mut snapshots = VecDeque::new();

        for session_id in 1..=6 {
            let mut snapshot = latency_snapshot();
            snapshot.session_id = session_id;
            push_latency_snapshot(&mut snapshots, snapshot);
        }

        assert_eq!(snapshots.len(), MAX_LATENCY_SNAPSHOTS);
        assert_eq!(
            snapshots.front().map(|snapshot| snapshot.session_id),
            Some(2)
        );
        assert_eq!(
            snapshots.back().map(|snapshot| snapshot.session_id),
            Some(6)
        );
    }

    #[test]
    fn background_cleanup_failure_does_not_change_visible_latency() {
        let mut snapshot = latency_snapshot();
        snapshot.speech_end_to_visible_ms = Some(850);
        snapshot.ollama_cleanup_ms = Some(2_400);
        snapshot.background_cleanup_status = BackgroundCleanupStatus::Failed;

        let rows = latency_diagnostics_for(Some(&snapshot));

        assert_eq!(
            diagnostic_row(&rows, "Latency: speech end to visible text").value,
            "850 ms"
        );
        assert_eq!(
            diagnostic_row(&rows, "Latency: Ollama cleanup").status,
            "error"
        );
    }

    #[test]
    fn end_of_speech_detector_waits_for_actual_voice() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(started_at + Duration::from_millis(900), 0.0));
        assert!(!detector.observe(started_at + Duration::from_millis(1_800), 0.001));
    }

    #[test]
    fn quick_hotkey_release_is_treated_as_tap_to_start() {
        assert!(should_ignore_quick_release(Duration::from_millis(
            QUICK_TAP_RELEASE_MS - 1,
        )));
        assert!(!should_ignore_quick_release(Duration::from_millis(
            QUICK_TAP_RELEASE_MS,
        )));
        assert!(should_toggle_stop(Duration::from_millis(
            QUICK_TAP_RELEASE_MS,
        )));
    }

    #[test]
    fn pitch_estimator_distinguishes_lower_and_higher_voice_tones() {
        let sample_rate = 48_000;
        let low = sine_wave(120.0, sample_rate, 0.08);
        let high = sine_wave(260.0, sample_rate, 0.08);
        let low_pitch = estimate_pitch_normalized(&low, sample_rate, rms_level(&low)).unwrap();
        let high_pitch = estimate_pitch_normalized(&high, sample_rate, rms_level(&high)).unwrap();

        assert!(high_pitch > low_pitch);
        assert!(low_pitch < 0.3);
        assert!(high_pitch > 0.45);
    }

    #[test]
    fn end_of_speech_detector_times_out_when_no_voice_arrives() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS - 100),
            0.0,
        ));
        assert!(detector.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS),
            0.0,
        ));
    }

    #[test]
    fn end_of_speech_detector_stops_after_post_speech_silence() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(started_at + Duration::from_millis(80), 0.02));
        assert!(!detector.observe(started_at + Duration::from_millis(220), 0.012));
        assert!(!detector.observe(
            started_at + Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS + 120),
            0.0,
        ));
        assert!(detector.observe(
            started_at
                + Duration::from_millis(
                    MIN_AUTO_STOP_RECORDING_MS + END_OF_SPEECH_TIMEOUT_MS + 120,
                ),
            0.0,
        ));
    }

    #[test]
    fn no_speech_timeout_is_distinguishable_from_end_of_speech() {
        let started_at = Instant::now();

        // Abandoned tap: never any voice -> stops via NO_SPEECH_TIMEOUT with speech_seen false,
        // so the recorder cancels silently instead of flashing an error.
        let mut idle = EndOfSpeechDetector::new(started_at);
        assert!(idle.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS),
            0.0,
        ));
        assert!(!idle.speech_seen, "abandoned tap must not report speech");

        // Real speech then silence -> stops via END_OF_SPEECH with speech_seen true.
        let mut spoken = EndOfSpeechDetector::new(started_at);
        assert!(!spoken.observe(started_at + Duration::from_millis(80), 0.02));
        assert!(!spoken.observe(started_at + Duration::from_millis(220), 0.012));
        assert!(spoken.observe(
            started_at
                + Duration::from_millis(
                    MIN_AUTO_STOP_RECORDING_MS + END_OF_SPEECH_TIMEOUT_MS + 120,
                ),
            0.0,
        ));
        assert!(spoken.speech_seen, "spoken dictation must report speech");
    }

    #[test]
    fn end_of_speech_detector_caps_long_recordings() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(detector.observe(
            started_at + Duration::from_secs(MAX_RECORDING_SECS + 1),
            0.0
        ));
    }

    #[test]
    fn detects_blank_whisper_markers() {
        assert!(is_blank_transcript("[BLANK_AUDIO]"));
        assert!(is_blank_transcript("(silence)"));
        assert!(!is_blank_transcript("hello local flow"));
    }

    #[test]
    fn cleanup_prompt_preserves_seeded_casing_and_punctuation_contract() {
        let deterministic_text = "Email Sarah about PyTorch at 3.14 p.m.\n- Ship LocalFlow.";
        let prompt = build_cleanup_prompt(
            "email sarah about pie torch at three point one four p m bullet ship local flow",
            deterministic_text,
        );
        let value: serde_json::Value =
            serde_json::from_str(&prompt).expect("cleanup prompt should be valid JSON");
        let rules = value
            .get("rules")
            .and_then(|rules| rules.as_array())
            .expect("cleanup prompt should include rules");

        assert_eq!(
            value
                .get("deterministicText")
                .and_then(|text| text.as_str()),
            Some(deterministic_text)
        );
        assert!(rules.iter().filter_map(|rule| rule.as_str()).any(|rule| {
            rule.contains("Preserve deterministicText capitalization, punctuation")
                && rule.contains("technical casing")
        }));
    }

    #[test]
    fn session_registry_begin_activates_new_id() {
        let mut registry = SessionRegistry::default();

        // id 0 (never started) is never authorized to insert.
        assert!(!registry.is_current(0));

        let first = registry.begin();
        assert_eq!(first, 1);
        assert!(registry.is_current(first));
    }

    #[test]
    fn new_session_supersedes_previous_in_flight_worker() {
        let mut registry = SessionRegistry::default();
        let first = registry.begin();

        // A second recording starts while the first is still being processed.
        let second = registry.begin();

        assert_ne!(first, second);
        assert!(!registry.is_current(first)); // stale worker must abort before pasting
        assert!(registry.is_current(second));
    }

    #[test]
    fn target_matches_only_identical_foreground_window() {
        let a = TargetWindow {
            hwnd: 0x1234,
            pid: 42,
        };
        let same = TargetWindow {
            hwnd: 0x1234,
            pid: 42,
        };
        let other_window = TargetWindow {
            hwnd: 0x9999,
            pid: 42,
        };
        let other_process = TargetWindow {
            hwnd: 0x1234,
            pid: 7,
        };

        assert!(target_matches(Some(a), Some(same)));
        assert!(!target_matches(Some(a), Some(other_window)));
        assert!(!target_matches(Some(a), Some(other_process)));
    }

    #[test]
    fn target_matches_fails_closed_when_target_unknown() {
        let a = TargetWindow {
            hwnd: 0x1234,
            pid: 42,
        };

        // Cannot revalidate -> must not paste.
        assert!(!target_matches(None, Some(a)));
        assert!(!target_matches(Some(a), None));
        assert!(!target_matches(None, None));
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

    #[test]
    fn cancel_invalidates_the_current_session() {
        let mut registry = SessionRegistry::default();
        let session = registry.begin();
        assert!(registry.is_current(session));

        registry.invalidate();

        // The cancelled session can no longer insert, and no worker holds the new id.
        assert!(!registry.is_current(session));
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

    fn latency_snapshot() -> NativeLatencySnapshot {
        NativeLatencySnapshot {
            session_id: 1,
            ollama_model: "llama3.2:3b".to_string(),
            low_resource_mode: false,
            audio_duration_ms: 1_200,
            hotkey_to_recording_start_ms: Some(30),
            vad_tail_ms: Some(550),
            capture_stop_ms: Some(5),
            resample_ms: Some(2),
            wav_write_ms: Some(3),
            whisper_ms: Some(600),
            deterministic_ms: Some(1),
            paste_set_clipboard_ms: Some(2),
            paste_send_ms: Some(4),
            clipboard_restore_delay_ms: Some(400),
            speech_end_to_visible_ms: Some(900),
            recording_start_to_insert_ms: Some(1_500),
            ollama_cleanup_ms: Some(1_100),
            background_cleanup_status: BackgroundCleanupStatus::Completed,
        }
    }

    fn diagnostic_row<'a>(
        rows: &'a [crate::app_state::DiagnosticMetric],
        label: &str,
    ) -> &'a crate::app_state::DiagnosticMetric {
        rows.iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("missing diagnostic row: {label}"))
    }
}
