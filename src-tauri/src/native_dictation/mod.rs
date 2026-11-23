//! Push-to-talk dictation: hotkey -> record -> transcribe -> format -> paste, with optional
//! background cleanup by the local model. Shared building blocks live in `audio`, `asr`,
//! `llm`, `insertion`, `overlay` and `runtime_paths`.

mod latency;
mod session;

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
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
use tauri::{AppHandle, Manager};

use self::{
    latency::{
        audio_duration_ms, duration_ms_between, latency_diagnostics_for, push_latency_snapshot,
        BackgroundCleanupStatus, NativeLatencySnapshot,
    },
    session::SessionRegistry,
};
use crate::{
    asr::{
        engine_select::{asr_language, prepare_asr_in_background, resolve_asr},
        model_label,
        provider::TranscriptionRequest,
        whisper_text::{build_whisper_prompt, is_blank_transcript},
    },
    audio::{
        capture::{analyze_audio, build_stream, is_near_silent, rms_level},
        resample::resample_to_16khz,
        spectrum::SpectrumAnalyzer,
        vad::EndOfSpeechDetector,
    },
    insertion::{
        clipboard_paste::{
            paste_text_for_visible_insert, restore_clipboard_after_paste,
            set_transient_clipboard_text,
        },
        target::{foreground_target, target_matches, TargetWindow},
    },
    llm::{
        cleanup::refine_with_pinned_ollama,
        ollama::{warm_ollama_in_background, OLLAMA_MODEL},
    },
    overlay::{emit_audio_features, emit_status, overlay_audio_features},
};

pub struct NativeDictationRuntime {
    command_tx: Mutex<mpsc::Sender<RecorderCommand>>,
    /// True from the moment a recording starts until it is finished or cancelled. Audio
    /// feature events do not keep the overlay visible, so nothing else may hide it or
    /// replace its phase while this is set.
    recording: AtomicBool,
    sessions: Mutex<SessionRegistry>,
    last_transcript: Mutex<Option<String>>,
    latency_snapshots: Mutex<VecDeque<NativeLatencySnapshot>>,
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

struct RecorderCommand {
    app: AppHandle,
    state: String,
    received_at: Instant,
}

const VAD_POLL_MS: u64 = 55;
/// The level meter wakes this many times per VAD poll. Each wake analyses the spectrum for the
/// overlay (about 36 frames a second); every `METER_TICKS_PER_VAD_POLL`-th wake also runs
/// voice-activity detection on the audio gathered since the last one, so the detector sees
/// the same 55 ms chunks as before.
const METER_TICKS_PER_VAD_POLL: u64 = 2;

const QUICK_TAP_RELEASE_MS: u64 = 700;

impl Default for NativeDictationRuntime {
    fn default() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        thread::spawn(move || recorder_loop(command_rx));

        Self {
            command_tx: Mutex::new(command_tx),
            recording: AtomicBool::new(false),
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
pub(crate) fn last_transcript(app: &AppHandle) -> Option<String> {
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
    // Kept out of clipboard history (Win+V) and cloud clipboard sync, like the paste itself.
    set_transient_clipboard_text(&mut clipboard, &text)
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
                set_recording(&command.app, false);
                set_escape_cancel(&command.app, false);
                invalidate_session(&command.app);
                emit_status(&command.app, "cancelled", "Cancelled");
                Ok(())
            }
            _ => Ok(()),
        };

        if let Err(error) = result {
            tracing::warn!(error = %error, "native dictation command failed");
            crate::desktop_health::record_recording_error(&command.app, &error);
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
    set_recording(app, true);

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
    crate::desktop_health::record_recording_start(app, &device_name, sample_rate, channels);
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
    prepare_asr_in_background(app.clone());
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
    set_recording(&app, false);
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
                    crate::desktop_health::record_recording_error(&app, &error);
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

    let resample_started_at = Instant::now();
    let mono_16k = resample_to_16khz(captured, sample_rate);
    let resample_ms = resample_started_at.elapsed().as_millis();

    // The engine is chosen from settings (model, low-resource mode) and reused between
    // dictations. It enforces its own deadline, so a hung sidecar can never leave the overlay
    // stuck on "processing".
    let asr = resolve_asr(app, &settings)?;
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
            asr_provider: asr_provider.clone(),
            asr_model: model_label(&asr.model_path),
            asr_fallback_used,
            audio_duration_ms: audio_duration_ms(captured.len(), sample_rate),
            hotkey_to_recording_start_ms: Some(duration_ms_between(hotkey_received_at, started_at)),
            vad_tail_ms: speech_ended_at
                .map(|speech_ended_at| duration_ms_between(speech_ended_at, stop_requested_at)),
            capture_stop_ms: Some(capture_stop_ms),
            resample_ms: Some(resample_ms),
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
    schedule_clipboard_restore(
        app.clone(),
        session_id,
        paste.previous_text,
        paste.sequence_after_set,
    );

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
        asr_provider = %asr_provider,
        asr_fallback_used,
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

/// A worker found that its session is no longer current. It deliberately emits nothing: an
/// explicit cancel has already announced "cancelled" itself, and a session superseded by a
/// newer recording must not hide the overlay that the newer recording is using.
fn emit_cancelled(_app: &AppHandle) {
    tracing::info!("native dictation session superseded or cancelled before insertion");
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
        let mut analyzer = SpectrumAnalyzer::new(sample_rate);
        // Reused buffers: nothing is allocated per tick once they have grown.
        let mut fresh: Vec<f32> = Vec::new();
        let mut vad_chunk: Vec<f32> = Vec::new();
        let mut tick = 0u64;

        loop {
            thread::sleep(Duration::from_millis(
                VAD_POLL_MS / METER_TICKS_PER_VAD_POLL,
            ));
            if stop_rx.try_recv().is_ok() {
                break;
            }
            tick += 1;

            // Copy the new samples out and release the lock before any analysis, so the
            // audio callback is never kept waiting on the meter.
            fresh.clear();
            if let Ok(output) = samples.lock() {
                let start = cursor.min(output.len());
                fresh.extend_from_slice(&output[start..]);
                cursor = output.len();
            }

            if !fresh.is_empty() {
                analyzer.push(&fresh);
                let frame = analyzer.analyze();
                emit_audio_features(&app, overlay_audio_features(&frame, rms_level(&fresh)));
                vad_chunk.extend_from_slice(&fresh);
            }

            if tick % METER_TICKS_PER_VAD_POLL != 0 {
                continue;
            }

            // A device that stops delivering audio (unplugged, disabled) produces empty
            // polls. The end-of-speech, no-speech, and maximum-length timeouts still run on
            // them so a session can never listen forever, but an empty poll is not evidence
            // of silence (the device may simply deliver buffers longer than the poll
            // interval), so it does not reset speech confirmation.
            let finished = if vad_chunk.is_empty() {
                detector.timed_out(Instant::now())
            } else {
                let rms = rms_level(&vad_chunk);
                vad_chunk.clear();
                detector.observe(Instant::now(), rms)
            };

            if finished {
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

fn schedule_clipboard_restore(
    app: AppHandle,
    session_id: u64,
    previous_text: Option<String>,
    sequence_after_set: Option<u32>,
) {
    thread::spawn(move || {
        let restore_started_at = Instant::now();
        // Give the target app time to read the pasted clipboard before we restore the prior
        // contents. This happens after visible insertion and is excluded from the main
        // speech-end-to-visible-text latency.
        thread::sleep(Duration::from_millis(400));

        restore_clipboard_after_paste(previous_text, sequence_after_set);

        update_latency_snapshot(&app, session_id, |snapshot| {
            snapshot.clipboard_restore_delay_ms = Some(restore_started_at.elapsed().as_millis());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
