//! Conversation mode: a hands-free, spoken back-and-forth with the local language model.
//!
//! A second mode beside dictation. A hotkey switches it on; from then the microphone stays
//! open, each thing the user says is transcribed and answered aloud, and the same hotkey
//! interrupts a reply or ends the conversation. Everything runs on this computer.
//!
//! - `state`: the turn-taking state machine (pure).
//! - `engine`: the threads that run a conversation, written against traits.
//! - `router`: fixed phrases ("goodbye", "repeat that") handled without the model.
//! - `chunker`: cuts the model's streamed reply into speakable pieces.
//! - `memory`: the bounded, in-memory history of one conversation.
//!
//! This file supplies the real devices and models and connects the engine to the tray, the
//! hotkey and the overlay. Nothing that is said is logged, stored, or sent in an event.

pub mod assembler;
pub mod chunker;
pub mod echo_guard;
pub mod engine;
pub mod memory;
pub mod router;
pub mod speaking;
pub mod state;
pub mod turn_completion;

use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use self::{
    engine::{
        ConversationCore, ConversationHost, Limits, NoPageSource, Notice, Resources, TurnTimings,
    },
    state::{Event, Phase, StopReason},
};
use crate::{
    app_state::{DiagnosticMetric, LocalFlowRuntime},
    asr::engine_select::{asr_language, resolve_asr},
    audio::{
        devices::{MicStream, SpeakerStream},
        speech_detector::{best_detector, SILERO_MODEL_FILE},
    },
    llm::{chat::OllamaChat, ollama::OLLAMA_MODEL},
    overlay::{emit_audio_features, emit_status, overlay_audio_features},
    runtime_paths::runtime_candidates,
    tts::phonon::{PhononConfig, PhononProvider, PHONON_MODEL_DIR},
};

/// Event channel for conversation phase changes. Payloads carry a phase, an optional reason
/// and a fixed message, never anything that was said.
pub const CONVERSATION_EVENT: &str = "localflow://conversation";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConversationEvent {
    phase: Phase,
    reason: Option<StopReason>,
    message: String,
}

/// What the settings window shows about conversation mode.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStatus {
    pub phase: Phase,
    /// Whether everything needed is installed.
    pub available: bool,
    /// What is missing, or a short description of what will be used.
    pub detail: String,
    /// The hotkey that toggles conversation mode, when one could be registered.
    pub hotkey: Option<String>,
}

pub struct ConversationRuntime {
    core: ConversationCore,
    shared: Arc<Shared>,
}

/// State the host writes and the commands read.
#[derive(Default)]
struct Shared {
    last_problem: Mutex<Option<String>>,
    last_turn: Mutex<Option<TurnTimings>>,
    hotkey: Mutex<Option<String>>,
}

impl ConversationRuntime {
    pub fn new(app: AppHandle) -> Self {
        let shared = Arc::new(Shared::default());
        let host = Arc::new(TauriHost {
            app,
            shared: shared.clone(),
        });
        Self {
            core: ConversationCore::start(host, Limits::default()),
            shared,
        }
    }

    pub fn phase(&self) -> Phase {
        self.core.phase()
    }

    pub fn is_active(&self) -> bool {
        self.phase().is_active()
    }
}

/// Whether a conversation is on. Dictation checks this so the two never share the microphone.
pub fn is_active(app: &AppHandle) -> bool {
    app.try_state::<ConversationRuntime>()
        .is_some_and(|runtime| runtime.is_active())
}

/// The conversation hotkey or tray item: start, interrupt, or end, depending on the phase.
pub fn toggle(app: &AppHandle) {
    if let Some(runtime) = app.try_state::<ConversationRuntime>() {
        runtime.core.send(Event::Toggle);
    }
}

/// End the conversation now, whatever it is doing. Called on app exit.
pub fn end(app: &AppHandle) {
    if let Some(runtime) = app.try_state::<ConversationRuntime>() {
        runtime.core.send(Event::EndRequested);
    }
}

/// Record which hotkey was registered, for the status shown to the user.
pub fn set_hotkey_label(app: &AppHandle, label: Option<String>) {
    if let Some(runtime) = app.try_state::<ConversationRuntime>() {
        *lock(&runtime.shared.hotkey) = label;
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Folder holding the conversation voice, if a runtime folder has one.
pub(crate) fn voice_model_dir(app: &AppHandle) -> Option<std::path::PathBuf> {
    runtime_candidates(app)
        .into_iter()
        .map(|runtime| runtime.join("models").join(PHONON_MODEL_DIR))
        .find(|dir| {
            PhononProvider::missing_file(&PhononConfig {
                model_dir: dir.clone(),
            })
            .is_none()
        })
}

fn silero_model(app: &AppHandle) -> Option<std::path::PathBuf> {
    runtime_candidates(app)
        .into_iter()
        .map(|runtime| runtime.join("models").join(SILERO_MODEL_FILE))
        .find(|path| path.is_file())
}

/// How finished a transcript reads, for deciding how long a silence ends the turn.
pub fn default_readiness() -> assembler::ReadinessJudge {
    Arc::new(|text: &str| match turn_completion::judge_transcript(text) {
        turn_completion::Completeness::Complete => assembler::Readiness::Ready,
        turn_completion::Completeness::Incomplete => assembler::Readiness::NotReady,
        turn_completion::Completeness::Unsure => assembler::Readiness::Unsure,
    })
}

struct TauriHost {
    app: AppHandle,
    shared: Arc<Shared>,
}

impl ConversationHost for TauriHost {
    fn open(&self) -> Result<Resources, String> {
        let app = &self.app;
        if crate::native_dictation::is_recording(app) {
            return Err("Finish the dictation first, then start a conversation.".to_string());
        }
        // Read-aloud and a conversation would talk over each other.
        crate::tts::stop(app);

        let model_dir = voice_model_dir(app).ok_or_else(|| {
            "The conversation voice is not installed. Run scripts\\Get-PhononModel.ps1.".to_string()
        })?;
        let settings = app.state::<LocalFlowRuntime>().current_settings();
        let model = match settings.models.ollama_model.trim() {
            "" => OLLAMA_MODEL.to_string(),
            configured => configured.to_string(),
        };

        // Models first: if one cannot load, the microphone is never opened.
        let asr = resolve_asr(app, &settings)?;
        asr.provider
            .prepare()
            .map_err(|error| format!("Speech recognition could not start: {error}"))?;
        let language = asr_language(&settings.models.language, &asr.model_path);

        let voice = Arc::new(PhononProvider::new(PhononConfig { model_dir }));
        crate::tts::stream::StreamingTextToSpeech::prepare(voice.as_ref())
            .map_err(|error| error.to_string())?;

        let chat = Arc::new(OllamaChat::new().map_err(|error| error.to_string())?);

        let (detector, fallback_reason) = best_detector(silero_model(app).as_deref());
        if let Some(reason) = fallback_reason {
            tracing::warn!(reason = %reason, "conversation is using the loudness rule to detect speech");
        }

        let speaker = SpeakerStream::open()?;
        let microphone = MicStream::open()?;
        tracing::info!(
            microphone = microphone.device_name(),
            detector = detector.id(),
            asr = %asr.provider.describe(),
            model = %model,
            "conversation resources opened"
        );

        Ok(Resources {
            source: Arc::new(microphone),
            sink: Arc::new(speaker),
            detector,
            asr: asr.provider,
            chat,
            voice,
            // "Read this page" reads whatever window has the focus when it is said.
            page: match app.try_state::<crate::page_reader::ScreenReader>() {
                Some(reader) => Arc::new(reader.inner().clone()),
                None => Arc::new(NoPageSource),
            },
            echo: crate::audio::echo::best_echo_canceller(),
            readiness: default_readiness(),
            model,
            language,
        })
    }

    fn notify(&self, notice: Notice) {
        let app = &self.app;
        match notice {
            Notice::Microphone { frame, level } => {
                emit_audio_features(app, overlay_audio_features(&frame, level));
            }
            Notice::Problem(problem) => {
                tracing::warn!(problem = %problem, "conversation problem");
                *lock(&self.shared.last_problem) = Some(problem);
            }
            Notice::Turn(timings) => {
                tracing::info!(
                    turn = timings.turn,
                    speech_ms = timings.speech_ms,
                    endpoint_wait_ms = timings.endpoint_wait_ms,
                    transcribe_ms = timings.transcribe_ms,
                    model_first_text_ms = ?timings.model_first_text_ms,
                    model_prompt_ms = ?timings.model_prompt_ms,
                    model_load_ms = ?timings.model_load_ms,
                    model_prompt_tokens = ?timings.model_prompt_tokens,
                    voice_first_audio_ms = ?timings.voice_first_audio_ms,
                    response_ms = ?timings.response_ms,
                    transcript_chars = timings.transcript_chars,
                    reply_chars = timings.reply_chars,
                    "conversation turn answered"
                );
                *lock(&self.shared.last_turn) = Some(timings);
            }
            Notice::Phase { phase, reason } => {
                let message =
                    phase_message(phase, reason, lock(&self.shared.last_problem).as_deref());
                // The overlay knows dictation's phases; conversation borrows the closest ones.
                match phase {
                    Phase::Starting => emit_status(app, "processing", &message),
                    Phase::Listening | Phase::UserSpeaking => {
                        emit_status(app, "listening", &message)
                    }
                    Phase::Thinking => emit_status(app, "processing", &message),
                    Phase::Speaking => emit_status(app, "refining", &message),
                    Phase::Off => match reason {
                        // Ended on purpose: just put the overlay away.
                        Some(StopReason::User | StopReason::Goodbye) | None => {
                            emit_status(app, "cancelled", &message)
                        }
                        // Ended by a limit: say so briefly.
                        Some(StopReason::Idle | StopReason::MaxDuration) => {
                            emit_status(app, "ready", &message)
                        }
                        Some(StopReason::StartFailed | StopReason::DeviceLost) => {
                            emit_status(app, "error", &message)
                        }
                    },
                }
                if phase == Phase::Off {
                    *lock(&self.shared.last_problem) = None;
                }
                let _ = app.emit(
                    CONVERSATION_EVENT,
                    ConversationEvent {
                        phase,
                        reason,
                        message,
                    },
                );
            }
        }
    }
}

/// The short line shown for a phase. Fixed wording; never derived from speech.
fn phase_message(phase: Phase, reason: Option<StopReason>, last_problem: Option<&str>) -> String {
    match phase {
        Phase::Starting => "Starting conversation".to_string(),
        Phase::Listening => "Listening".to_string(),
        Phase::UserSpeaking => "Listening".to_string(),
        Phase::Thinking => "Thinking".to_string(),
        Phase::Speaking => "Speaking".to_string(),
        Phase::Off => match reason {
            Some(StopReason::Idle) => "Conversation ended: no speech".to_string(),
            Some(StopReason::MaxDuration) => "Conversation ended: time limit".to_string(),
            Some(StopReason::DeviceLost) => {
                "Conversation ended: the microphone stopped".to_string()
            }
            Some(StopReason::StartFailed) => last_problem
                .unwrap_or("The conversation could not start.")
                .to_string(),
            Some(StopReason::User | StopReason::Goodbye) | None => "Conversation ended".to_string(),
        },
    }
}

#[tauri::command]
pub fn conversation_status(app: AppHandle) -> ConversationStatus {
    let runtime = app.state::<ConversationRuntime>();
    let hotkey = lock(&runtime.shared.hotkey).clone();
    let (available, detail) = match voice_model_dir(&app) {
        None => (
            false,
            "The conversation voice is not installed. Run scripts\\Get-PhononModel.ps1."
                .to_string(),
        ),
        Some(_) => (
            true,
            if silero_model(&app).is_some() {
                "Local speech recognition, local language model, Phonon voice, Silero voice detection".to_string()
            } else {
                "Ready, using a simple loudness rule to detect speech. Run scripts\\Get-VadModel.ps1 for better detection.".to_string()
            },
        ),
    };
    ConversationStatus {
        phase: runtime.phase(),
        available,
        detail,
        hotkey,
    }
}

#[tauri::command]
pub fn toggle_conversation(app: AppHandle) {
    toggle(&app);
}

/// Diagnostics rows: the last answered turn's timings, as numbers only.
pub fn diagnostics(app: &AppHandle) -> Vec<DiagnosticMetric> {
    let Some(runtime) = app.try_state::<ConversationRuntime>() else {
        return Vec::new();
    };
    let last_turn = *lock(&runtime.shared.last_turn);
    conversation_diagnostics(last_turn.as_ref())
}

fn conversation_diagnostics(last_turn: Option<&TurnTimings>) -> Vec<DiagnosticMetric> {
    let row = |label: &str, value: String, status: &str| DiagnosticMetric {
        label: label.to_string(),
        value,
        status: status.to_string(),
    };
    let Some(turn) = last_turn else {
        return vec![row(
            "Conversation: response time",
            "Not measured".to_string(),
            "warning",
        )];
    };
    let optional = |value: Option<u64>| {
        value
            .map(|ms| format!("{ms} ms"))
            .unwrap_or_else(|| "Not measured".to_string())
    };

    vec![
        row(
            "Conversation: response time",
            format!("{} from end of speech to first audio", optional(turn.response_ms)),
            match turn.response_ms {
                Some(ms) if ms <= 1_500 => "ok",
                Some(_) => "warning",
                None => "warning",
            },
        ),
        row(
            "Conversation: stages",
            format!(
                "wait for pause {} ms / transcribe {} ms / model first text {} / voice first audio {}",
                turn.endpoint_wait_ms,
                turn.transcribe_ms,
                optional(turn.model_first_text_ms),
                optional(turn.voice_first_audio_ms)
            ),
            "ok",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_phase_has_a_fixed_message() {
        assert_eq!(phase_message(Phase::Listening, None, None), "Listening");
        assert_eq!(phase_message(Phase::Thinking, None, None), "Thinking");
        assert_eq!(phase_message(Phase::Speaking, None, None), "Speaking");
        assert_eq!(
            phase_message(Phase::Off, Some(StopReason::Idle), None),
            "Conversation ended: no speech"
        );
        assert_eq!(
            phase_message(Phase::Off, Some(StopReason::Goodbye), None),
            "Conversation ended"
        );
    }

    #[test]
    fn a_failed_start_shows_why() {
        assert_eq!(
            phase_message(
                Phase::Off,
                Some(StopReason::StartFailed),
                Some("No default microphone is available.")
            ),
            "No default microphone is available."
        );
        assert_eq!(
            phase_message(Phase::Off, Some(StopReason::StartFailed), None),
            "The conversation could not start."
        );
    }

    #[test]
    fn diagnostics_show_timings_or_say_nothing_was_measured() {
        let none = conversation_diagnostics(None);
        assert_eq!(none.len(), 1);
        assert_eq!(none[0].value, "Not measured");

        let rows = conversation_diagnostics(Some(&TurnTimings {
            turn: 3,
            speech_ms: 2_000,
            endpoint_wait_ms: 704,
            transcribe_ms: 310,
            model_first_text_ms: Some(60),
            model_prompt_ms: Some(20),
            model_load_ms: Some(0),
            model_prompt_tokens: Some(300),
            voice_first_audio_ms: Some(120),
            response_ms: Some(1_240),
            transcript_chars: 40,
            reply_chars: 80,
        }));
        assert_eq!(rows[0].value, "1240 ms from end of speech to first audio");
        assert_eq!(rows[0].status, "ok");
        assert!(rows[1].value.contains("transcribe 310 ms"));
    }
}
