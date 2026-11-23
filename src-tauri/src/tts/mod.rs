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
    /// Set to interrupt playback in progress.
    stop: Arc<AtomicBool>,
    speaking: AtomicBool,
}

impl Default for TtsRuntime {
    fn default() -> Self {
        Self {
            engine: Mutex::new(None),
            generation: AtomicU64::new(0),
            stop: Arc::new(AtomicBool::new(false)),
            speaking: AtomicBool::new(false),
        }
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

/// Resolve (building or reusing) the voice engine.
fn resolve_engine(app: &AppHandle) -> Result<Arc<dyn TextToSpeechProvider>, TtsError> {
    let install = locate_install(app).map_err(TtsError::Unavailable)?;
    let key = EngineKey {
        server_path: install.server_path.clone(),
        model_path: install.model_path.clone(),
        backend: install.backend,
    };

    let runtime = app.state::<TtsRuntime>();
    let mut slot = runtime
        .engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(existing) = slot.as_ref() {
        if existing.key == key {
            return Ok(existing.provider.clone());
        }
    }
    if let Some(previous) = slot.take() {
        previous.provider.shutdown();
    }

    let provider: Arc<dyn TextToSpeechProvider> = Arc::new(FishS2Provider::new(FishS2Config {
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
    Ok(provider)
}

/// Start reading `text` aloud. Returns once the request is accepted; synthesis and playback
/// continue on a worker thread and report through [`TTS_EVENT`]. A new request supersedes
/// one in progress.
pub fn speak(app: &AppHandle, text: &str) -> Result<(), String> {
    // A conversation's microphone would take the reading for the user and answer it.
    if crate::conversation::is_active(app) {
        return Err("Reading aloud is not available during a conversation.".to_string());
    }
    let text = prepare_text_for_speech(text).ok_or_else(|| TtsError::NothingToSay.to_string())?;
    // Fail fast, before spawning anything, when no engine is installed.
    locate_install(app)?;

    let runtime = app.state::<TtsRuntime>();
    let generation = runtime.generation.fetch_add(1, Ordering::SeqCst) + 1;
    // Interrupt playback from an earlier request; the worker below clears the flag again.
    runtime.stop.store(true, Ordering::SeqCst);

    let app = app.clone();
    thread::spawn(move || run_request(&app, generation, &text));
    Ok(())
}

fn is_current(app: &AppHandle, generation: u64) -> bool {
    app.state::<TtsRuntime>().generation.load(Ordering::SeqCst) == generation
}

fn run_request(app: &AppHandle, generation: u64, text: &str) {
    emit(app, "synthesizing", "Preparing speech");

    let outcome = resolve_engine(app).and_then(|engine| engine.synthesize(text));
    let audio = match outcome {
        Ok(audio) => audio,
        Err(error) => {
            if is_current(app, generation) {
                tracing::warn!(error = %error, "read-aloud failed");
                emit(app, "error", error.to_string());
            }
            return;
        }
    };

    // A stop or a newer request arrived while synthesising: discard the audio.
    if !is_current(app, generation) {
        return;
    }

    let runtime = app.state::<TtsRuntime>();
    let stop = runtime.stop.clone();
    stop.store(false, Ordering::SeqCst);
    // A stop that landed between the check above and clearing the flag would otherwise be
    // lost, and the whole clip would play. `stop()` changes the generation before it sets the
    // flag, so checking again here catches it; a stop after this point leaves the flag set.
    if !is_current(app, generation) {
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

    if is_current(app, generation) {
        runtime.speaking.store(false, Ordering::SeqCst);
        match played {
            Ok(_) => emit(app, "idle", "Finished"),
            Err(error) => {
                tracing::warn!(error = %error, "read-aloud playback failed");
                emit(app, "error", error);
            }
        }
        schedule_idle_unload(app.clone(), generation);
    }
}

/// Unload the engine if nothing else has been requested for [`IDLE_UNLOAD_AFTER`].
fn schedule_idle_unload(app: AppHandle, generation: u64) {
    thread::spawn(move || {
        thread::sleep(IDLE_UNLOAD_AFTER);
        if is_current(&app, generation) {
            tracing::info!("unloading the idle voice engine");
            shutdown(&app);
        }
    });
}

/// Stop reading aloud. Synthesis already in progress finishes in the sidecar, but its audio
/// is discarded.
pub fn stop(app: &AppHandle) {
    let runtime = app.state::<TtsRuntime>();
    runtime.generation.fetch_add(1, Ordering::SeqCst);
    runtime.stop.store(true, Ordering::SeqCst);
    runtime.speaking.store(false, Ordering::SeqCst);
    emit(app, "idle", "Stopped");
}

/// Stop the voice engine sidecar and free its memory. Called on idle and when the app exits.
pub fn shutdown(app: &AppHandle) {
    let slot = app
        .state::<TtsRuntime>()
        .engine
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(slot) = slot {
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
