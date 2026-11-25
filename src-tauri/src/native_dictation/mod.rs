//! Push-to-talk dictation: hotkey -> record -> transcribe -> format -> paste, with optional
//! background cleanup by the local model. Shared building blocks live in `audio`, `asr`,
//! `llm`, `insertion`, `overlay` and `runtime_paths`.
//!
//! Threads: one recorder loop owns the microphone and handles every hotkey and level-meter
//! event in order (`gesture.rs` decides what each one means); a level meter per recording
//! feeds the overlay and the end-of-speech detector; a worker per finished recording
//! transcribes, formats and pastes. The recorder loop and the workers are supervised: a panic
//! resets dictation and tells the user instead of leaving it dead until restart.

mod gesture;
mod latency;
mod messages;
mod recovery;
mod session;

use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, TryRecvError},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use arboard::Clipboard;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use tauri::{AppHandle, Manager};

use self::{
    gesture::{decide, is_auto_stop, ActiveRecording, Decision, RecorderInput},
    latency::{
        audio_duration_ms, duration_ms_between, latency_diagnostics_for, push_latency_snapshot,
        BackgroundCleanupStatus, NativeLatencySnapshot,
    },
    recovery::Recovery,
    session::{SessionRegistry, SessionStatus},
};
use crate::{
    asr::{
        engine_select::{asr_language, plan_asr_engine, prepare_asr_in_background, resolve_asr},
        model_label,
        provider::TranscriptionRequest,
        whisper_text::{build_whisper_prompt, is_blank_transcript},
    },
    audio::{
        capture::{analyze_audio, build_stream, is_near_silent, rms_level},
        resample::resample_to_16khz,
        spectrum::SpectrumAnalyzer,
        vad::{end_of_speech_timeout, EndOfSpeechDetector, StopReason},
    },
    insertion::{
        clipboard_paste::{
            paste_text, remember_own_clipboard_text, set_transient_clipboard_text, PendingRestore,
        },
        target::{foreground_target, target_matches, TargetWindow},
    },
    llm::{
        cleanup::refine_with_pinned_ollama,
        ollama::{warm_ollama_in_background, OLLAMA_MODEL},
    },
    overlay::{emit_audio_features, emit_status, overlay_audio_features},
    platform::{clipboard::FinishOutcome, keyboard, window},
};

pub struct NativeDictationRuntime {
    command_tx: Mutex<mpsc::Sender<RecorderCommand>>,
    /// True from the moment a recording starts until it is finished or cancelled. Audio
    /// feature events do not keep the overlay visible, so nothing else may hide it or
    /// replace its phase while this is set.
    recording: AtomicBool,
    /// Workers still transcribing or pasting.
    workers_in_flight: AtomicUsize,
    sessions: Mutex<SessionRegistry>,
    recovery: Mutex<Recovery>,
    latency_snapshots: Mutex<VecDeque<NativeLatencySnapshot>>,
}

struct RecordingSession {
    session_id: u64,
    target: Option<TargetWindow>,
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
    sample_format: SampleFormat,
    hotkey_received_at: Instant,
    started_at: Instant,
    last_voice_at: Arc<Mutex<Option<Instant>>>,
    /// True while the hotkey that started the recording is held (push-to-talk). Shared with
    /// the level meter, which suppresses silence stops while it is set.
    key_held: Arc<AtomicBool>,
    level_stop_tx: mpsc::Sender<()>,
}

struct RecorderCommand {
    app: AppHandle,
    input: RecorderInput,
    received_at: Instant,
}

/// A hotkey event for dictation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyInput {
    Pressed,
    Released,
    /// Escape while recording.
    Cancel,
}

/// A finished recording on its way to the worker.
struct CapturedDictation {
    session_id: u64,
    target: Option<TargetWindow>,
    samples: Vec<f32>,
    sample_rate: u32,
    channels: u16,
    timing: DictationTiming,
}

#[derive(Debug, Clone, Copy)]
struct DictationTiming {
    hotkey_received_at: Instant,
    started_at: Instant,
    speech_ended_at: Option<Instant>,
    stop_requested_at: Instant,
    capture_stop_ms: u128,
}

const VAD_POLL_MS: u64 = 55;
/// The level meter wakes this many times per VAD poll. Each wake analyses the spectrum for the
/// overlay (about 36 frames a second); every `METER_TICKS_PER_VAD_POLL`-th wake also runs
/// voice-activity detection on the audio gathered since the last one, so the detector sees
/// the same 55 ms chunks as before.
const METER_TICKS_PER_VAD_POLL: u64 = 2;
/// How long the paste waits for `Ctrl`, `Alt`, `Shift` and `Win` to come up.
const MODIFIER_RELEASE_WAIT: Duration = Duration::from_secs(1);

impl Default for NativeDictationRuntime {
    fn default() -> Self {
        Self {
            command_tx: Mutex::new(spawn_recorder()),
            recording: AtomicBool::new(false),
            workers_in_flight: AtomicUsize::new(0),
            sessions: Mutex::new(SessionRegistry::default()),
            recovery: Mutex::new(Recovery::default()),
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

fn spawn_recorder() -> mpsc::Sender<RecorderCommand> {
    let (command_tx, command_rx) = mpsc::channel();
    if let Err(error) = thread::Builder::new()
        .name("localflow-dictation".to_string())
        .spawn(move || recorder_loop(command_rx))
    {
        tracing::error!(error = %error, "could not start the dictation recorder thread");
    }
    command_tx
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

fn with_recovery<T>(app: &AppHandle, f: impl FnOnce(&mut Recovery) -> T) -> T {
    let runtime = app.state::<NativeDictationRuntime>();
    let mut guard = runtime
        .recovery
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

/// Assign and activate a fresh session id, superseding any in-flight worker.
fn begin_session(app: &AppHandle) -> u64 {
    with_sessions(app, SessionRegistry::begin)
}

/// Cancel the active session; any in-flight worker for it discards its result.
fn invalidate_session(app: &AppHandle) {
    with_sessions(app, SessionRegistry::invalidate);
}

/// Whether `session_id` is still authorized to insert text.
fn session_is_current(app: &AppHandle, session_id: u64) -> bool {
    with_sessions(app, |registry| registry.is_current(session_id))
}

fn session_status(app: &AppHandle, session_id: u64) -> SessionStatus {
    with_sessions(app, |registry| registry.status(session_id))
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

/// Remember a finalized transcript in volatile memory so the user can recover it (copy /
/// paste-last) if automatic insertion was skipped or failed. A transcript from an older
/// session never replaces a newer one. Nothing is written to disk (retention-safe).
fn store_last_transcript(app: &AppHandle, session_id: u64, text: &str) -> bool {
    with_recovery(app, |recovery| recovery.store(session_id, text))
}

/// The most recent finalized transcript, if any.
pub(crate) fn last_transcript(app: &AppHandle) -> Option<String> {
    with_recovery(app, |recovery| recovery.text())
}

/// Copy the last finalized transcript to the clipboard for manual pasting. Reachable from
/// the tray and the settings UI; needs no target focus, so it is always safe to run.
pub fn copy_last_transcript_to_clipboard(app: &AppHandle) -> Result<(), String> {
    let text = last_transcript(app)
        .ok_or_else(|| "No transcript is available to copy yet.".to_string())?;
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
    // Kept out of clipboard history (Win+V) and cloud clipboard sync, like the paste itself.
    set_transient_clipboard_text(&mut clipboard, &text)
        .map_err(|error| format!("Could not set clipboard text: {error}"))?;
    // The user asked for this text, so a later paste puts it back afterwards.
    remember_own_clipboard_text();
    Ok(())
}

/// Return the most recent finalized transcript (or `null`) for the recovery UI.
#[tauri::command]
pub fn get_last_transcript(app: AppHandle) -> Option<String> {
    last_transcript(&app)
}

/// Copy the most recent finalized transcript to the clipboard (explicit recovery action).
/// `async`: off the main thread, since opening the clipboard can wait on another program.
#[tauri::command(async)]
pub fn copy_last_transcript(app: AppHandle) -> Result<(), String> {
    copy_last_transcript_to_clipboard(&app)
}

/// Whether a recording is in progress.
pub(crate) fn is_recording(app: &AppHandle) -> bool {
    app.state::<NativeDictationRuntime>()
        .recording
        .load(Ordering::SeqCst)
}

fn set_recording(app: &AppHandle, recording: bool) {
    app.state::<NativeDictationRuntime>()
        .recording
        .store(recording, Ordering::SeqCst);
}

fn workers_in_flight(app: &AppHandle) -> usize {
    app.state::<NativeDictationRuntime>()
        .workers_in_flight
        .load(Ordering::SeqCst)
}

/// Whether a status pulse from outside the dictation flow may be shown. During a recording
/// it may not: the pulse would replace the "listening" phase and its hide timer would then
/// hide the overlay while the user is still speaking.
fn pulse_allowed(recording: bool) -> bool {
    !recording
}

pub fn show_ready_pulse(app: &AppHandle, message: &str) {
    if pulse_allowed(is_recording(app)) {
        emit_status(app, "ready", message);
    }
}

pub fn show_error_pulse(app: &AppHandle, message: &str) {
    if pulse_allowed(is_recording(app)) {
        emit_status(app, "error", message);
    }
}

/// Show an error and record it in the health file. Paths that name the user are removed
/// first; callers never put dictated text in `message`.
fn report_error(app: &AppHandle, message: &str) {
    let message = messages::scrub(message);
    tracing::warn!(error = %message, "native dictation failed");
    crate::desktop_health::record_recording_error(app, &message);
    emit_status(app, "error", &message);
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

pub fn handle_hotkey(app: AppHandle, input: HotkeyInput) -> Result<(), String> {
    let input = match input {
        HotkeyInput::Pressed => RecorderInput::Pressed,
        HotkeyInput::Released => RecorderInput::Released,
        HotkeyInput::Cancel => RecorderInput::Cancel,
    };
    send_command(&app, input)
}

/// Hand `input` to the recorder loop. If the loop has died, a new one is started and the
/// command goes to it, so dictation never stays dead until LocalFlow restarts.
fn send_command(app: &AppHandle, input: RecorderInput) -> Result<(), String> {
    let runtime = app.state::<NativeDictationRuntime>();
    let mut command_tx = runtime
        .command_tx
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let command = RecorderCommand {
        app: app.clone(),
        input,
        received_at: Instant::now(),
    };
    let Err(mpsc::SendError(command)) = command_tx.send(command) else {
        return Ok(());
    };

    tracing::error!("the dictation recorder had stopped; starting a new one");
    *command_tx = spawn_recorder();
    drop(command_tx);
    set_recording(app, false);
    // Not shown on the overlay: the restarted loop may be about to show "listening".
    crate::desktop_health::record_recording_error(
        app,
        "Dictation was restarted after an internal error.",
    );
    let resent = runtime
        .command_tx
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .send(command);
    resent.map_err(|_| "The dictation recorder could not be restarted.".to_string())
}

/// State owned by the recorder loop.
#[derive(Default)]
struct Recorder {
    session: Option<RecordingSession>,
    /// When the level meter last stopped a recording.
    last_auto_stop_at: Option<Instant>,
}

fn recorder_loop(command_rx: mpsc::Receiver<RecorderCommand>) {
    let mut recorder = Recorder::default();

    for command in command_rx {
        let app = command.app.clone();
        // A panic while handling one command must not end the loop: dictation would be dead
        // until restart and nobody would be told.
        match catch_unwind(AssertUnwindSafe(|| recorder.handle(command))) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                report_error(&app, &error);
                recorder.show_notice_if_idle(&app);
            }
            Err(_) => recorder.recover_after_panic(&app),
        }
    }
}

impl Recorder {
    fn handle(&mut self, command: RecorderCommand) -> Result<(), String> {
        let RecorderCommand {
            app,
            input,
            received_at,
        } = command;
        let active = self.session.as_ref().map(|session| ActiveRecording {
            elapsed: session.started_at.elapsed(),
            key_held: session.key_held.load(Ordering::SeqCst),
        });
        let since_auto_stop = self.last_auto_stop_at.map(|at| at.elapsed());
        let decision = decide(input, active, since_auto_stop);
        if is_auto_stop(input, decision) {
            self.last_auto_stop_at = Some(Instant::now());
        }

        match decision {
            Decision::Start => {
                self.session = Some(start_recording(&app, received_at)?);
                Ok(())
            }
            Decision::Finish => match self.session.take() {
                Some(session) => finish_recording(app, session, received_at),
                None => Ok(()),
            },
            Decision::Discard | Decision::Cancel => {
                // Stop any live recording and invalidate the current session so an
                // in-flight worker for it discards its result instead of pasting.
                if let Some(session) = self.session.take() {
                    let _ = session.level_stop_tx.send(());
                    drop(session.stream);
                }
                set_recording(&app, false);
                set_escape_cancel(&app, false);
                invalidate_session(&app);
                emit_status(&app, "cancelled", messages::CANCELLED);
                self.show_notice_if_idle(&app);
                Ok(())
            }
            Decision::HandsFree => {
                if let Some(session) = self.session.as_ref() {
                    session.key_held.store(false, Ordering::SeqCst);
                }
                tracing::info!("quick hotkey release; continuing hands-free");
                Ok(())
            }
            Decision::ShowNotice => {
                self.show_notice_if_idle(&app);
                Ok(())
            }
            Decision::Ignore(reason) => {
                tracing::debug!(reason, "dictation event ignored");
                Ok(())
            }
        }
    }

    /// Tell the user that a superseded dictation was not pasted, once nothing else owns the
    /// overlay. Runs on the recorder thread, so no recording can start meanwhile.
    fn show_notice_if_idle(&self, app: &AppHandle) {
        let busy = self.session.is_some() || workers_in_flight(app) > 0;
        if with_recovery(app, |recovery| recovery.take_notice(busy)) {
            crate::desktop_health::record_recording_error(app, messages::PREVIOUS_NOT_PASTED);
            emit_status(app, "error", messages::PREVIOUS_NOT_PASTED);
        }
    }

    fn recover_after_panic(&mut self, app: &AppHandle) {
        tracing::error!("the dictation recorder hit an internal error; resetting dictation");
        if let Some(session) = self.session.take() {
            let _ = catch_unwind(AssertUnwindSafe(move || {
                let _ = session.level_stop_tx.send(());
                drop(session);
            }));
        }
        set_recording(app, false);
        set_escape_cancel(app, false);
        invalidate_session(app);
        report_error(app, messages::INTERNAL_ERROR);
    }
}

/// Check, before the microphone opens, that a speech engine is installed, so the user learns
/// it is missing before speaking rather than after. Only files are looked at; nothing loads.
fn check_speech_engine(
    app: &AppHandle,
    settings: &crate::app_state::SettingsSnapshot,
) -> Result<(), String> {
    use crate::runtime_paths::{
        runtime_candidates, speech_model_path, whisper_cli_path, PREFERRED_SPEECH_MODELS,
    };

    let runtimes = runtime_candidates(app).len();
    let model = speech_model_path(app, &settings.models.whisper_model_path)
        .map_err(|_| messages::missing_speech_model(runtimes * PREFERRED_SPEECH_MODELS.len()))?;
    plan_asr_engine(
        model,
        settings.models.low_resource_mode,
        whisper_cli_path(app).ok(),
        None,
        None,
    )
    .map(|_| ())
    .map_err(|_| messages::missing_whisper_cli(runtimes))
}

fn start_recording(
    app: &AppHandle,
    hotkey_received_at: Instant,
) -> Result<RecordingSession, String> {
    let settings = app
        .state::<crate::app_state::LocalFlowRuntime>()
        .current_settings();
    check_speech_engine(app, &settings)?;

    // Anything LocalFlow is reading aloud would be recorded and pasted with the user's words.
    crate::tts::stop(app);
    crate::page_reader::stop(app);

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
    let key_held = Arc::new(AtomicBool::new(true));

    let stream = build_stream(&device, &stream_config, sample_format, samples.clone())?;
    stream
        .play()
        .map_err(|error| format!("Could not start microphone capture: {error}"))?;
    let recording_started_at = Instant::now();
    let command_tx = app
        .state::<NativeDictationRuntime>()
        .command_tx
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let level_stop_tx = start_level_meter(
        app.clone(),
        LevelMeterInputs {
            samples: samples.clone(),
            sample_rate,
            last_voice_at: last_voice_at.clone(),
            key_held: key_held.clone(),
            end_of_speech: end_of_speech_timeout(settings.microphone.end_of_speech_ms),
        },
        command_tx,
    );

    // Assign and activate a fresh session id; this supersedes any worker still processing a
    // previous recording, which then keeps its transcript for recovery but never pastes.
    let session_id = begin_session(app);
    set_recording(app, true);

    emit_status(app, "listening", messages::LISTENING);
    crate::desktop_health::record_recording_start(app, &device_name, sample_rate, channels);
    set_escape_cancel(app, true);
    let warm_model = if settings.models.ollama_model.trim().is_empty() {
        OLLAMA_MODEL.to_string()
    } else {
        settings.models.ollama_model.trim().to_string()
    };
    warm_ollama_in_background(warm_model);
    prepare_asr_in_background(app.clone());
    // No device name: it is often personal ("<name>'s AirPods").
    tracing::info!(
        sample_rate,
        channels,
        sample_format = ?sample_format,
        "native microphone recording started"
    );

    Ok(RecordingSession {
        session_id,
        target,
        stream,
        samples,
        sample_rate,
        channels,
        sample_format,
        hotkey_received_at,
        started_at: recording_started_at,
        last_voice_at,
        key_held,
        level_stop_tx,
    })
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
        sample_format,
        hotkey_received_at,
        started_at,
        last_voice_at,
        key_held: _,
        level_stop_tx,
    } = session;

    let _ = level_stop_tx.send(());
    drop(stream);
    set_recording(&app, false);
    set_escape_cancel(&app, false);
    emit_status(&app, "processing", messages::TRANSCRIBING);

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
            "Captured audio was silent or near-silent (peak={:.5}, rms={:.5}). Check the Windows default input device, microphone privacy access, and input gain.",
            stats.peak, stats.rms
        ));
    }

    spawn_worker(
        app,
        CapturedDictation {
            session_id,
            target,
            samples: captured,
            sample_rate,
            channels,
            timing: DictationTiming {
                hotkey_received_at,
                started_at,
                speech_ended_at,
                stop_requested_at,
                capture_stop_ms,
            },
        },
    );
    Ok(())
}

/// Counts a worker in flight for as long as it lives, panics included.
struct InFlight(AppHandle);

impl InFlight {
    fn enter(app: &AppHandle) -> Self {
        app.state::<NativeDictationRuntime>()
            .workers_in_flight
            .fetch_add(1, Ordering::SeqCst);
        Self(app.clone())
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.0
            .state::<NativeDictationRuntime>()
            .workers_in_flight
            .fetch_sub(1, Ordering::SeqCst);
    }
}

/// Run the transcribe -> quick insert -> background refine tail off the recorder thread so the
/// recorder stays responsive and a newer or cancelled session can supersede this one.
/// The worker revalidates its session id before every side effect and never pastes if it is
/// no longer the current session (spec §4.4).
fn spawn_worker(app: AppHandle, dictation: CapturedDictation) {
    let in_flight = InFlight::enter(&app);
    let spawned = thread::Builder::new()
        .name("localflow-dictation-worker".to_string())
        .spawn(move || {
            let session_id = dictation.session_id;
            let result = catch_unwind(AssertUnwindSafe(|| process_session(&app, &dictation)));
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(_) => {
                    tracing::error!("native dictation worker hit an internal error");
                    Some(messages::INTERNAL_ERROR.to_string())
                }
            };
            if let Some(error) = error {
                if session_is_current(&app, session_id) {
                    report_error(&app, &error);
                } else {
                    tracing::info!(
                        error = %messages::scrub(&error),
                        "native dictation error suppressed for superseded session"
                    );
                }
            }
            drop(in_flight);
            // A notice held back while this worker ran may be shown now.
            let _ = send_command(&app, RecorderInput::WorkerDone);
        });
    if let Err(error) = spawned {
        tracing::error!(error = %error, "could not start a dictation worker");
    }
}

/// The text to show when the engine for these settings cannot be resolved: what is missing,
/// with file names only, or a general line with the details in the log.
fn engine_error_message(
    app: &AppHandle,
    settings: &crate::app_state::SettingsSnapshot,
    error: &str,
) -> String {
    tracing::warn!(error = %messages::scrub(error), "speech engine could not be resolved");
    match check_speech_engine(app, settings) {
        Err(message) => message,
        Ok(()) => messages::ENGINE_UNAVAILABLE.to_string(),
    }
}

/// Transcribe -> deterministic insert -> background refine. Runs on a worker thread.
/// Revalidates the session before every side effect. A cancelled session is discarded; a
/// superseded one keeps its transcript for recovery but never pastes.
fn process_session(app: &AppHandle, dictation: &CapturedDictation) -> Result<(), String> {
    let session_id = dictation.session_id;
    if session_status(app, session_id) == SessionStatus::Cancelled {
        tracing::info!("native dictation session cancelled before transcription");
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

    let resample_started_at = Instant::now();
    let mono_16k = resample_to_16khz(&dictation.samples, dictation.sample_rate);
    let resample_ms = resample_started_at.elapsed().as_millis();

    // The engine is chosen from settings (model, low-resource mode) and reused between
    // dictations. It enforces its own deadline, so a hung sidecar can never leave the overlay
    // stuck on "processing".
    let asr = resolve_asr(app, &settings)
        .map_err(|error| engine_error_message(app, &settings, &error))?;
    let whisper_prompt = build_whisper_prompt(&dictionary_terms);
    let language = asr_language(&settings.models.language, &asr.model_path);
    let whisper_started_at = Instant::now();
    let transcription = asr
        .provider
        .transcribe(&TranscriptionRequest {
            samples: &mono_16k,
            language: &language,
            initial_prompt: whisper_prompt.as_deref(),
        })
        .map_err(|error| format!("Local transcription failed: {error}"))?;
    let whisper_ms = whisper_started_at.elapsed().as_millis();
    let asr_provider = transcription.provider.to_string();
    let asr_fallback_used = transcription.fallback_reason.is_some();
    let transcript = transcription.text;

    if transcript.trim().is_empty() || is_blank_transcript(&transcript) {
        return Err(messages::NOTHING_RECOGNISED.to_string());
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

    // Keep the quick transcript before deciding anything else, so it is recoverable (copy /
    // paste-last) even if this dictation was superseded by a newer recording or the paste
    // is skipped. Only an explicit cancel discards it.
    match session_status(app, session_id) {
        SessionStatus::Cancelled => {
            tracing::info!("native dictation session cancelled before insertion");
            return Ok(());
        }
        SessionStatus::Superseded => {
            if store_last_transcript(app, session_id, &deterministic) {
                with_recovery(app, |recovery| recovery.mark_unpasted(session_id));
            }
            tracing::info!("native dictation superseded by a newer recording; not pasted");
            return Ok(());
        }
        SessionStatus::Current => {
            store_last_transcript(app, session_id, &deterministic);
        }
    }

    let background_cleanup_status = if settings.models.low_resource_mode {
        tracing::info!("low_resource_mode enabled; background LLM cleanup skipped");
        BackgroundCleanupStatus::Skipped
    } else {
        BackgroundCleanupStatus::Pending
    };

    // Push-to-talk ends when Space is released; Ctrl and Alt may still be down, and a
    // Ctrl+V sent now would arrive as Ctrl+Alt+V.
    if !keyboard::wait_for_modifiers_released(MODIFIER_RELEASE_WAIT) {
        if session_is_current(app, session_id) {
            tracing::warn!("modifier keys still held; skipping paste");
            report_error(app, messages::KEYS_STILL_HELD);
        }
        return Ok(());
    }

    // Final guard before the only irreversible side effect: never paste into whatever is
    // focused now if this session was superseded or cancelled while we were working.
    match session_status(app, session_id) {
        SessionStatus::Current => {}
        SessionStatus::Superseded => {
            with_recovery(app, |recovery| recovery.mark_unpasted(session_id));
            tracing::info!("native dictation superseded by a newer recording; not pasted");
            return Ok(());
        }
        SessionStatus::Cancelled => {
            tracing::info!("native dictation session cancelled before insertion");
            return Ok(());
        }
    }

    // Revalidate the insertion target, down to the focused control. If focus moved (or we
    // cannot confirm the original), do not paste — a Ctrl+V would land in the wrong place,
    // possibly a password or unrelated field. Fail closed and tell the user (spec §6.2). The
    // transcript is kept for recovery via "Copy last transcript".
    let target = dictation.target;
    if !target_matches(target, foreground_target()) {
        tracing::warn!("insertion target changed since dictation started; skipping paste");
        report_error(app, messages::FOCUS_CHANGED);
        return Ok(());
    }

    // Windows silently drops synthetic input to a window of a higher integrity level (an
    // administrator window when LocalFlow is not one), so the paste would never arrive.
    if let Some(target) = target {
        if window::input_blocked_for_process(target.pid) {
            tracing::warn!("target window runs at a higher integrity level; skipping paste");
            report_error(app, messages::ELEVATED_TARGET);
            return Ok(());
        }
    }

    let paste = paste_text(&deterministic, target.map(|target| target.pid))
        .map_err(|error| messages::paste_failed(&error))?;
    let visible_at = paste.timings.visible_at;
    let timing = dictation.timing;
    let speech_end_reference = timing.speech_ended_at.unwrap_or(timing.stop_requested_at);

    record_latency_snapshot(
        app,
        NativeLatencySnapshot {
            session_id,
            ollama_model: ollama_model.clone(),
            low_resource_mode: settings.models.low_resource_mode,
            asr_provider: asr_provider.clone(),
            asr_model: model_label(&asr.model_path),
            asr_fallback_used,
            audio_duration_ms: audio_duration_ms(dictation.samples.len(), dictation.sample_rate),
            hotkey_to_recording_start_ms: Some(duration_ms_between(
                timing.hotkey_received_at,
                timing.started_at,
            )),
            vad_tail_ms: timing.speech_ended_at.map(|speech_ended_at| {
                duration_ms_between(speech_ended_at, timing.stop_requested_at)
            }),
            capture_stop_ms: Some(timing.capture_stop_ms),
            resample_ms: Some(resample_ms),
            whisper_ms: Some(whisper_ms),
            deterministic_ms: Some(deterministic_ms),
            paste_set_clipboard_ms: Some(paste.timings.set_clipboard_ms),
            paste_send_ms: Some(paste.timings.send_paste_ms),
            clipboard_restore_delay_ms: None,
            speech_end_to_visible_ms: Some(duration_ms_between(speech_end_reference, visible_at)),
            recording_start_to_insert_ms: Some(duration_ms_between(timing.started_at, visible_at)),
            ollama_cleanup_ms: None,
            background_cleanup_status,
        },
    );

    emit_status(app, "inserted", messages::INSERTED);
    schedule_clipboard_restore(app.clone(), session_id, paste.restore);

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
        elapsed_ms = duration_ms_between(timing.started_at, visible_at),
        speech_end_to_visible_ms = duration_ms_between(speech_end_reference, visible_at),
        whisper_ms,
        asr_provider = %asr_provider,
        asr_fallback_used,
        chars = deterministic.chars().count(),
        sample_rate = dictation.sample_rate,
        channels = dictation.channels,
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
                // Only while this dictation's transcript is still the one kept for recovery.
                with_recovery(&app, |recovery| recovery.refine(session_id, &refined));
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

/// What the level meter of one recording reads and shares.
struct LevelMeterInputs {
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    last_voice_at: Arc<Mutex<Option<Instant>>>,
    key_held: Arc<AtomicBool>,
    end_of_speech: Duration,
}

fn start_level_meter(
    app: AppHandle,
    inputs: LevelMeterInputs,
    command_tx: mpsc::Sender<RecorderCommand>,
) -> mpsc::Sender<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    thread::spawn(move || {
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            run_level_meter(&app, &inputs, &stop_rx, &command_tx)
        }));
        if outcome.is_err() {
            // Without the meter a hands-free recording would never stop by itself. Ask the
            // recorder to finish it; a held key is still ended by its release.
            tracing::error!("dictation level meter hit an internal error");
            let _ = command_tx.send(RecorderCommand {
                app: app.clone(),
                input: RecorderInput::MeterStop {
                    reason: StopReason::EndOfSpeech,
                    speech_seen: true,
                },
                received_at: Instant::now(),
            });
        }
    });

    stop_tx
}

fn run_level_meter(
    app: &AppHandle,
    inputs: &LevelMeterInputs,
    stop_rx: &mpsc::Receiver<()>,
    command_tx: &mpsc::Sender<RecorderCommand>,
) {
    let mut cursor = 0usize;
    let mut detector = EndOfSpeechDetector::new(Instant::now(), inputs.end_of_speech);
    let mut analyzer = SpectrumAnalyzer::new(inputs.sample_rate);
    // Reused buffers: nothing is allocated per tick once they have grown.
    let mut fresh: Vec<f32> = Vec::new();
    let mut vad_chunk: Vec<f32> = Vec::new();
    let mut tick = 0u64;

    loop {
        thread::sleep(Duration::from_millis(
            VAD_POLL_MS / METER_TICKS_PER_VAD_POLL,
        ));
        // Stop when asked, or when the recording that owns the meter has gone.
        if !matches!(stop_rx.try_recv(), Err(TryRecvError::Empty)) {
            break;
        }
        tick += 1;

        // Copy the new samples out and release the lock before any analysis, so the
        // audio callback is never kept waiting on the meter.
        fresh.clear();
        if let Ok(output) = inputs.samples.lock() {
            let start = cursor.min(output.len());
            fresh.extend_from_slice(&output[start..]);
            cursor = output.len();
        }

        if !fresh.is_empty() {
            analyzer.push(&fresh);
            let frame = analyzer.analyze();
            emit_audio_features(app, overlay_audio_features(&frame, rms_level(&fresh)));
            vad_chunk.extend_from_slice(&fresh);
        }

        if !tick.is_multiple_of(METER_TICKS_PER_VAD_POLL) {
            continue;
        }

        // A device that stops delivering audio (unplugged, disabled) produces empty
        // polls. The end-of-speech, no-speech, and maximum-length timeouts still run on
        // them so a session can never listen forever, but an empty poll is not evidence
        // of silence (the device may simply deliver buffers longer than the poll
        // interval), so it does not reset speech confirmation.
        let now = Instant::now();
        if !vad_chunk.is_empty() {
            detector.observe(now, rms_level(&vad_chunk));
            vad_chunk.clear();
        }
        if let Ok(mut last_voice) = inputs.last_voice_at.lock() {
            *last_voice = detector.last_voice_at;
        }

        // While the hotkey is held (push-to-talk) only the maximum length ends the recording.
        let hands_free = !inputs.key_held.load(Ordering::SeqCst);
        if let Some(reason) = detector.stop_reason(now, hands_free) {
            // If speech was never detected, this is an abandoned tap: the recorder cancels
            // silently (hides the overlay) instead of running the pipeline.
            let _ = command_tx.send(RecorderCommand {
                app: app.clone(),
                input: RecorderInput::MeterStop {
                    reason,
                    speech_seen: detector.speech_seen,
                },
                received_at: now,
            });
            break;
        }
    }
}

/// Put the earlier clipboard back once the target has read the pasted text. Runs after the
/// visible insertion and is excluded from the speech-end-to-visible-text latency.
fn schedule_clipboard_restore(app: AppHandle, session_id: u64, restore: PendingRestore) {
    thread::spawn(move || {
        let report = restore.complete(window::is_remote_session());
        if report.outcome == FinishOutcome::Failed {
            tracing::warn!("the clipboard could not be tidied after the paste");
        }
        tracing::debug!(
            outcome = ?report.outcome,
            target_read = report.target_read,
            waited_ms = report.waited.as_millis() as u64,
            "clipboard tidied after the paste"
        );
        update_latency_snapshot(&app, session_id, |snapshot| {
            snapshot.clipboard_restore_delay_ms = Some(report.waited.as_millis());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_pulses_are_suppressed_only_while_recording() {
        assert!(pulse_allowed(false));
        assert!(!pulse_allowed(true));
    }

    #[test]
    fn detects_blank_whisper_markers() {
        assert!(is_blank_transcript("[BLANK_AUDIO]"));
        assert!(is_blank_transcript("(silence)"));
        assert!(!is_blank_transcript("hello local flow"));
    }

    #[test]
    fn a_panic_inside_catch_unwind_leaves_the_loop_usable() {
        // The recorder and worker rely on this: the closure's panic is contained and the
        // state borrowed by it is usable afterwards.
        let mut handled = 0;
        let first = catch_unwind(AssertUnwindSafe(|| {
            handled += 1;
            panic!("simulated failure while handling a command");
        }));
        let second = catch_unwind(AssertUnwindSafe(|| {
            handled += 1;
        }));

        assert!(first.is_err());
        assert!(second.is_ok());
        assert_eq!(handled, 2);
    }
}
