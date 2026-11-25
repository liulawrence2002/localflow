//! Read-aloud: turn text into speech with a local voice engine and play it.
//!
//! The coordinator here is deliberately small. It resolves the installed engine, runs
//! synthesis and playback on a worker thread, lets a newer request or an explicit stop
//! supersede the current one, and unloads the engine after a period without use because
//! voice models are large. It never logs or emits the text being spoken.

pub mod fish_s2;
pub mod phonon;
pub mod playback;
pub mod provider;
pub mod stream;

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::cancel::CancelFlag;
use fish_s2::{FishS2Backend, FishS2Config, FishS2Provider};
use provider::{prepare_text_for_speech, TextToSpeechProvider, TtsError};

/// Event channel for read-aloud phase changes. Payloads carry a phase and a short message,
/// never the text.
pub const TTS_EVENT: &str = "localflow://tts";

/// The engine is unloaded after this long without a read-aloud request.
const IDLE_UNLOAD_AFTER: Duration = Duration::from_secs(300);

/// File the build script writes next to the engine to record its compute backend.
const BACKEND_MARKER_FILE: &str = "backend.txt";

#[derive(Debug, Clone, PartialEq, Eq)]
struct EngineKey {
    server_path: PathBuf,
    model_path: PathBuf,
    backend: FishS2Backend,
}

struct EngineSlot {
    key: EngineKey,
    provider: Arc<dyn TextToSpeechProvider>,
}

pub struct TtsRuntime {
    engine: Mutex<Option<EngineSlot>>,
    /// Bumped by every speak and stop request; a worker only acts while its value is current.
    generation: AtomicU64,
    /// Bumped by every speak request only. The idle unload compares it, so a stop, which
    /// changes the generation, does not count as use and keep the engine loaded for ever.
    requests: AtomicU64,
    /// Cancels the synthesis of the current request, which stops the engine working on it.
    current: Mutex<Option<CancelFlag>>,
    /// Set to interrupt playback in progress.
    stop: Arc<AtomicBool>,
    speaking: AtomicBool,
}

impl Default for TtsRuntime {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            generation: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            current: Mutex::new(None),
            stop: Arc::new(AtomicBool::new(false)),
            speaking: AtomicBool::new(false),
        }
    }
}

/// One accepted read-aloud request.
#[derive(Debug, Clone)]
struct Request {
    generation: u64,
    /// The request count after this one, for the idle unload.
    ticket: u64,
    cancel: CancelFlag,
}

impl TtsRuntime {
    fn lock_current(&self) -> std::sync::MutexGuard<'_, Option<CancelFlag>> {
        self.current
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Accept a new request. It supersedes the one in progress, whose synthesis is cancelled
    /// and whose playback is interrupted.
    fn begin_request(&self) -> Request {
        let ticket = self.requests.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let cancel = CancelFlag::default();
        if let Some(previous) = self.lock_current().replace(cancel.clone()) {
            previous.cancel();
        }
        // Interrupt playback from an earlier request; the worker clears the flag again.
        self.stop.store(true, Ordering::SeqCst);
        Request {
            generation,
            ticket,
            cancel,
        }
    }

    /// Stop whatever is being read: cancel its synthesis and interrupt its playback.
    fn stop_requests(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(current) = self.lock_current().take() {
            current.cancel();
        }
        self.stop.store(true, Ordering::SeqCst);
        self.speaking.store(false, Ordering::SeqCst);
    }

    fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Whether nothing was asked for since the request with `ticket` and nothing is playing.
    fn idle_since(&self, ticket: u64) -> bool {
        self.requests.load(Ordering::SeqCst) == ticket && !self.speaking.load(Ordering::SeqCst)
    }

    /// Take the engine out of its slot without waiting on anything but the slot itself.
    fn take_engine(&self) -> Option<EngineSlot> {
        self.engine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsStatus {
    /// Whether an engine and model are installed.
    pub available: bool,
    /// Engine description when available, otherwise what is missing.
    pub detail: String,
    pub speaking: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TtsEvent {
    phase: &'static str,
    message: String,
}

fn emit(app: &AppHandle, phase: &'static str, message: impl Into<String>) {
    let _ = app.emit(
        TTS_EVENT,
        TtsEvent {
            phase,
            message: message.into(),
        },
    );
}

/// Where the Fish S2 engine and model live under one runtime folder.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FishInstall {
    server_path: PathBuf,
    model_path: PathBuf,
    tokenizer_path: PathBuf,
    backend: FishS2Backend,
}

/// Find a complete Fish S2 install under `runtime`: `fish/s2.exe`, a `models/fish/s2-pro-*.gguf`
/// model, and its `tokenizer.json`. Returns what is missing otherwise.
fn locate_fish_install(runtime: &Path) -> Result<FishInstall, String> {
    let engine_dir = runtime.join("fish");
    let server_path = engine_dir.join("s2.exe");
    if !server_path.is_file() {
        return Err("The Fish S2 engine is not built. Run scripts\\Build-FishS2.ps1.".to_string());
    }

    let model_dir = runtime.join("models").join("fish");
    let mut models: Vec<PathBuf> = std::fs::read_dir(&model_dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name().is_some_and(|name| {
                        let name = name.to_string_lossy().to_ascii_lowercase();
                        name.starts_with("s2-pro-") && name.ends_with(".gguf")
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    // Deterministic choice when several quantisations are present.
    models.sort();
    let model_path = models.into_iter().next().ok_or_else(|| {
        "The Fish S2 model is not downloaded. Run scripts\\Get-FishModel.ps1.".to_string()
    })?;

    let tokenizer_path = model_dir.join("tokenizer.json");
    if !tokenizer_path.is_file() {
        return Err(
            "The Fish S2 tokenizer is missing. Run scripts\\Get-FishModel.ps1.".to_string(),
        );
    }

    let backend = std::fs::read_to_string(engine_dir.join(BACKEND_MARKER_FILE))
        .map(|value| FishS2Backend::parse(&value))
        .unwrap_or_default();

    Ok(FishInstall {
        server_path,
        model_path,
        tokenizer_path,
        backend,
    })
}

fn locate_install(app: &AppHandle) -> Result<FishInstall, String> {
    let mut last_reason = "No LocalFlow runtime folder was found.".to_string();
    for runtime in crate::runtime_paths::runtime_candidates(app) {
        match locate_fish_install(&runtime) {
            Ok(install) => return Ok(install),
            Err(reason) => last_reason = reason,
        }
    }
    Err(last_reason)
}

/// Resolve (building or reusing) the voice engine. An engine for an older install is shut
/// down after the slot is released, so nothing waits on the slot while it stops.
fn resolve_engine(app: &AppHandle) -> Result<Arc<dyn TextToSpeechProvider>, TtsError> {
    let install = locate_install(app).map_err(TtsError::Unavailable)?;
    let key = EngineKey {
        server_path: install.server_path.clone(),
        model_path: install.model_path.clone(),
        backend: install.backend,
    };

    let runtime = app.state::<TtsRuntime>();
    let (provider, previous) = {
        let mut slot = runtime
            .engine
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        if let Some(existing) = slot.as_ref() {
            if existing.key == key {
                return Ok(existing.provider.clone());
            }
        }
        let previous = slot.take();

        let provider: Arc<dyn TextToSpeechProvider> =
            Arc::new(FishS2Provider::new(FishS2Config {
                server_path: install.server_path,
                model_path: install.model_path,
                tokenizer_path: install.tokenizer_path,
                backend: install.backend,
                threads: crate::asr::sidecar::default_asr_thread_count(),
                // Loading several gigabytes of weights can take a while on a cold disk.
                startup_timeout: Duration::from_secs(180),
            })?);
        tracing::info!(engine = %provider.describe(), "voice engine selected");
        *slot = Some(EngineSlot {
            key,
            provider: provider.clone(),
        });
        (provider, previous)
    };
    if let Some(previous) = previous {
        previous.provider.shutdown();
    }
    Ok(provider)
}

/// Start reading `text` aloud. Returns once the request is accepted; synthesis and playback
/// continue on a worker thread and report through [`TTS_EVENT`]. A new request supersedes
/// one in progress, and the engine stops working on the old one.
pub fn speak(app: &AppHandle, text: &str) -> Result<(), String> {
    // A conversation's microphone would take the reading for the user and answer it.
    if crate::conversation::is_active(app) {
        return Err("Reading aloud is not available during a conversation.".to_string());
    }
    let text = prepare_text_for_speech(text).ok_or_else(|| TtsError::NothingToSay.to_string())?;
    // Fail fast, before spawning anything, when no engine is installed.
    locate_install(app)?;
    // Reading a window and reading a transcript would talk over each other.
    crate::page_reader::stop(app);

    let request = app.state::<TtsRuntime>().begin_request();
    let app = app.clone();
    thread::spawn(move || {
        run_request(&app, &request, &text);
        schedule_idle_unload(app, request.ticket);
    });
    Ok(())
}

fn run_request(app: &AppHandle, request: &Request, text: &str) {
    let runtime = app.state::<TtsRuntime>();
    emit(app, "synthesizing", "Preparing speech");

    let outcome = resolve_engine(app).and_then(|engine| {
        // Superseded while the engine was being found: do not start on it at all.
        if !runtime.is_current(request.generation) {
            return Err(TtsError::Cancelled);
        }
        engine.synthesize(text, &request.cancel)
    });
    let audio = match outcome {
        Ok(audio) => audio,
        Err(TtsError::Cancelled) => return,
        Err(error) => {
            if runtime.is_current(request.generation) {
                tracing::warn!(error = %error, "read-aloud failed");
                emit(app, "error", error.to_string());
            }
            return;
        }
    };

    // A stop or a newer request arrived while synthesising: discard the audio.
    if !runtime.is_current(request.generation) {
        return;
    }

    let stop = runtime.stop.clone();
    stop.store(false, Ordering::SeqCst);
    // A stop that landed between the check above and clearing the flag would otherwise be
    // lost, and the whole clip would play. `stop()` changes the generation before it sets the
    // flag, so checking again here catches it; a stop after this point leaves the flag set.
    if !runtime.is_current(request.generation) {
        return;
    }
    runtime.speaking.store(true, Ordering::SeqCst);
    emit(app, "speaking", "Reading aloud");
    tracing::info!(
        audio_ms = audio.duration().as_millis() as u64,
        chars = text.chars().count(),
        "read-aloud playback started"
    );

    let played = playback::play_blocking(&audio, &stop);

    if runtime.is_current(request.generation) {
        runtime.speaking.store(false, Ordering::SeqCst);
        match played {
            Ok(_) => emit(app, "idle", "Finished"),
            Err(error) => {
                tracing::warn!(error = %error, "read-aloud playback failed");
                emit(app, "error", error);
            }
        }
    }
}

/// Unload the engine if nothing else has been asked for [`IDLE_UNLOAD_AFTER`] after the
/// request with `ticket` ended, however it ended.
fn schedule_idle_unload(app: AppHandle, ticket: u64) {
    thread::spawn(move || {
        thread::sleep(IDLE_UNLOAD_AFTER);
        if app.state::<TtsRuntime>().idle_since(ticket) {
            tracing::info!("unloading the idle voice engine");
            shutdown(&app);
        }
    });
}

/// Stop reading aloud: the engine stops working on the request in progress and playback
/// is interrupted.
pub fn stop(app: &AppHandle) {
    app.state::<TtsRuntime>().stop_requests();
    emit(app, "idle", "Stopped");
}

/// Stop the voice engine sidecar and free its memory. Called on idle and when the app
/// exits; it does not wait for a model that is still loading (the start is called off).
pub fn shutdown(app: &AppHandle) {
    if let Some(slot) = app.state::<TtsRuntime>().take_engine() {
        slot.provider.shutdown();
    }
}

#[tauri::command]
pub fn tts_status(app: AppHandle) -> TtsStatus {
    let speaking = app.state::<TtsRuntime>().speaking.load(Ordering::SeqCst);
    match locate_install(&app) {
        Ok(install) => TtsStatus {
            available: true,
            detail: format!(
                "Fish S2 Pro ({}, {})",
                crate::asr::model_label(&install.model_path),
                install.backend.label()
            ),
            speaking,
        },
        Err(reason) => TtsStatus {
            available: false,
            detail: reason,
            speaking,
        },
    }
}

#[tauri::command]
pub fn speak_text(app: AppHandle, text: String) -> Result<(), String> {
    speak(&app, &text)
}

#[tauri::command]
pub fn speak_last_transcript(app: AppHandle) -> Result<(), String> {
    let text = crate::native_dictation::last_transcript(&app)
        .ok_or_else(|| "No transcript is available to read yet.".to_string())?;
    speak(&app, &text)
}

#[tauri::command]
pub fn stop_speaking(app: AppHandle) {
    stop(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_runtime(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("localflow-tts-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("fish")).unwrap();
        std::fs::create_dir_all(dir.join("models").join("fish")).unwrap();
        dir
    }

    #[test]
    fn a_stop_does_not_keep_the_engine_from_unloading() {
        let runtime = TtsRuntime::default();
        let request = runtime.begin_request();

        // The user stops it; before the fix the stop changed the counter the idle check
        // compared, so the engine was never unloaded.
        runtime.stop_requests();

        assert!(!runtime.is_current(request.generation));
        assert!(runtime.idle_since(request.ticket));
    }

    #[test]
    fn a_newer_request_keeps_the_engine_and_supersedes_the_old_one() {
        let runtime = TtsRuntime::default();
        let first = runtime.begin_request();
        let second = runtime.begin_request();

        assert!(!runtime.idle_since(first.ticket));
        assert!(runtime.idle_since(second.ticket));
        assert!(!runtime.is_current(first.generation));
        assert!(runtime.is_current(second.generation));
        // The engine is told to stop working on the first, not just to drop its audio.
        assert!(first.cancel.is_cancelled());
        assert!(!second.cancel.is_cancelled());
    }

    #[test]
    fn stopping_cancels_the_synthesis_in_progress() {
        let runtime = TtsRuntime::default();
        let request = runtime.begin_request();
        runtime.speaking.store(true, Ordering::SeqCst);
        assert!(
            !runtime.idle_since(request.ticket),
            "never unload while playing"
        );

        runtime.stop_requests();

        assert!(request.cancel.is_cancelled());
        assert!(runtime.stop.load(Ordering::SeqCst));
        assert!(!runtime.speaking.load(Ordering::SeqCst));
    }

    #[test]
    fn reports_each_missing_piece_with_the_script_that_provides_it() {
        let runtime = temp_runtime("missing");

        let no_engine = locate_fish_install(&runtime).unwrap_err();
        assert!(no_engine.contains("Build-FishS2.ps1"));

        std::fs::write(runtime.join("fish").join("s2.exe"), b"stub").unwrap();
        let no_model = locate_fish_install(&runtime).unwrap_err();
        assert!(no_model.contains("model is not downloaded"));

        let model_dir = runtime.join("models").join("fish");
        std::fs::write(model_dir.join("s2-pro-q4_k_m.gguf"), b"stub").unwrap();
        let no_tokenizer = locate_fish_install(&runtime).unwrap_err();
        assert!(no_tokenizer.contains("tokenizer is missing"));

        let _ = std::fs::remove_dir_all(&runtime);
    }

    #[test]
    fn finds_a_complete_install_and_reads_the_backend_marker() {
        let runtime = temp_runtime("complete");
        let model_dir = runtime.join("models").join("fish");
        std::fs::write(runtime.join("fish").join("s2.exe"), b"stub").unwrap();
        std::fs::write(runtime.join("fish").join(BACKEND_MARKER_FILE), "vulkan\n").unwrap();
        std::fs::write(model_dir.join("s2-pro-q6_k.gguf"), b"stub").unwrap();
        std::fs::write(model_dir.join("s2-pro-q4_k_m.gguf"), b"stub").unwrap();
        std::fs::write(model_dir.join("notes.txt"), b"ignored").unwrap();
        std::fs::write(model_dir.join("tokenizer.json"), b"{}").unwrap();

        let install = locate_fish_install(&runtime).unwrap();
        let _ = std::fs::remove_dir_all(&runtime);

        assert_eq!(install.backend, FishS2Backend::Vulkan);
        // Sorted by name, so the choice does not depend on directory order.
        assert!(install.model_path.ends_with("s2-pro-q4_k_m.gguf"));
        assert!(install.tokenizer_path.ends_with("tokenizer.json"));
    }

    #[test]
    fn a_missing_backend_marker_means_cpu() {
        let runtime = temp_runtime("cpu");
        let model_dir = runtime.join("models").join("fish");
        std::fs::write(runtime.join("fish").join("s2.exe"), b"stub").unwrap();
        std::fs::write(model_dir.join("s2-pro-q4_k_m.gguf"), b"stub").unwrap();
        std::fs::write(model_dir.join("tokenizer.json"), b"{}").unwrap();

        let install = locate_fish_install(&runtime).unwrap();
        let _ = std::fs::remove_dir_all(&runtime);

        assert_eq!(install.backend, FishS2Backend::Cpu);
    }
}
