use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{
    asr::{AsrProvider, MockAsrProvider},
    insertion::{MockTextInserter, TextInserter},
    privacy::redact_for_log,
    refinement::{MockRefinementProvider, RefinementProvider},
    workflow::{transition, ActivationMode, TargetSnapshot, WorkflowEvent, WorkflowState},
};

pub struct LocalFlowRuntime {
    workflow: Mutex<WorkflowState>,
    settings: Mutex<SettingsSnapshot>,
    history: Mutex<Vec<HistoryItem>>,
}

impl Default for LocalFlowRuntime {
    fn default() -> Self {
        Self {
            workflow: Mutex::new(WorkflowState::default()),
            settings: Mutex::new(default_settings()),
            history: Mutex::new(Vec::new()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub workflow: WorkflowState,
    pub settings: SettingsSnapshot,
    pub history: Vec<HistoryItem>,
    pub diagnostics: Vec<DiagnosticMetric>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub hotkeys: HotkeySettings,
    pub models: ModelSettings,
    pub microphone: MicrophoneSettings,
    pub privacy: PrivacySettings,
    pub dictionary: Vec<DictionaryEntry>,
    pub replacements: Vec<ReplacementRule>,
    pub snippets: Vec<Snippet>,
    pub styles: Vec<StyleProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HotkeySettings {
    pub default_hotkey: String,
    pub activation_mode: ActivationMode,
    pub command_hotkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSettings {
    pub whisper_model_path: String,
    pub language: String,
    pub asr_threads: u8,
    pub ollama_model: String,
    pub low_resource_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicrophoneSettings {
    pub selected_device_id: String,
    pub selected_device_name: String,
    pub vad_enabled: bool,
    pub end_of_speech_ms: u16,
    pub max_recording_seconds: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacySettings {
    pub history_retention: String,
    pub delete_after: String,
    pub active_app_detection: bool,
    pub accessibility_context: bool,
    pub selected_text_transforms: bool,
    pub context_retention: bool,
    pub delete_audio_after_processing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictionaryEntry {
    pub id: String,
    pub phrase: String,
    pub pronunciation_hint: Option<String>,
    pub category: String,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplacementRule {
    pub id: String,
    pub incorrect: String,
    pub correct: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub trigger: String,
    pub expansion: String,
    pub enabled: bool,
    pub allow_cleanup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StyleProfile {
    pub id: String,
    pub name: String,
    pub category: String,
    pub cleanup_level: String,
    pub conciseness: u8,
    pub formality: u8,
    pub contractions: bool,
    pub emoji: String,
    pub paragraph_length: String,
    pub bullet_preference: String,
    pub greeting_behavior: String,
    pub sign_off_behavior: String,
    pub aggressive_filler_removal: bool,
    pub allow_sentence_fragments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryItem {
    pub id: String,
    pub completed_at: String,
    pub target_application: String,
    pub raw_transcript: String,
    pub deterministic_text: Option<String>,
    pub refined_text: Option<String>,
    pub final_text: String,
    pub cleanup_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticMetric {
    pub label: String,
    pub value: String,
    pub status: String,
}

#[tauri::command]
pub fn get_status(runtime: State<'_, LocalFlowRuntime>) -> Result<AppStatus, String> {
    Ok(AppStatus {
        workflow: runtime.workflow.lock().map_err(lock_error)?.clone(),
        settings: runtime.settings.lock().map_err(lock_error)?.clone(),
        history: runtime.history.lock().map_err(lock_error)?.clone(),
        diagnostics: default_diagnostics(),
    })
}

#[tauri::command]
pub fn save_settings(
    runtime: State<'_, LocalFlowRuntime>,
    settings: SettingsSnapshot,
) -> Result<AppStatus, String> {
    *runtime.settings.lock().map_err(lock_error)? = settings;
    get_status(runtime)
}

#[tauri::command]
pub fn begin_mock_session(runtime: State<'_, LocalFlowRuntime>) -> Result<WorkflowState, String> {
    let mut workflow = runtime.workflow.lock().map_err(lock_error)?;
    let session_id = format!("native-{}", Utc::now().timestamp_millis());

    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::BeginActivation {
            session_id: session_id.clone(),
            mode: ActivationMode::PushToTalk,
            target: TargetSnapshot::mock(),
            timestamp: Utc::now().to_rfc3339(),
        },
    );
    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::CaptureStarted { session_id },
    );

    Ok(workflow.clone())
}

#[tauri::command]
pub fn finish_mock_session(
    runtime: State<'_, LocalFlowRuntime>,
    raw_transcript: String,
) -> Result<AppStatus, String> {
    let asr = MockAsrProvider::new(raw_transcript);
    let refinement = MockRefinementProvider;
    let inserter = MockTextInserter::default();

    let transcript = asr.transcribe()?;
    let refined = refinement.refine(&transcript)?;
    let inserted = inserter.insert_text(&refined.text)?;

    tracing::info!(text = %redact_for_log(&inserted), "completed mock dictation");

    let mut workflow = runtime.workflow.lock().map_err(lock_error)?;
    if workflow.active_session.is_none() {
        let session_id = format!("native-{}", Utc::now().timestamp_millis());
        *workflow = transition(
            workflow.clone(),
            WorkflowEvent::BeginActivation {
                session_id: session_id.clone(),
                mode: ActivationMode::PushToTalk,
                target: TargetSnapshot::mock(),
                timestamp: Utc::now().to_rfc3339(),
            },
        );
        *workflow = transition(
            workflow.clone(),
            WorkflowEvent::CaptureStarted { session_id },
        );
    }

    let session_id = workflow
        .active_session
        .as_ref()
        .map(|session| session.id.clone())
        .ok_or_else(|| "No active session is available to finish.".to_string())?;

    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::RecordingStopped {
            session_id: session_id.clone(),
        },
    );
    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::TranscriptReady {
            session_id: session_id.clone(),
            transcript: transcript.clone(),
        },
    );
    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::DeterministicTextReady {
            session_id: session_id.clone(),
            text: transcript.clone(),
        },
    );
    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::RefinementReady {
            session_id: session_id.clone(),
            text: inserted.clone(),
            confidence: refined.confidence,
        },
    );
    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::Inserted {
            session_id,
            timestamp: Utc::now().to_rfc3339(),
        },
    );

    if let Some(last_completed) = workflow.last_completed.clone() {
        runtime
            .history
            .lock()
            .map_err(lock_error)?
            .insert(0, last_completed.into());
    }

    drop(workflow);
    get_status(runtime)
}

#[tauri::command]
pub fn cancel_session(runtime: State<'_, LocalFlowRuntime>) -> Result<WorkflowState, String> {
    let mut workflow = runtime.workflow.lock().map_err(lock_error)?;
    *workflow = transition(
        workflow.clone(),
        WorkflowEvent::Cancel {
            reason: "Cancelled by user.".to_string(),
        },
    );
    Ok(workflow.clone())
}

impl From<crate::workflow::SessionHistoryItem> for HistoryItem {
    fn from(value: crate::workflow::SessionHistoryItem) -> Self {
        Self {
            id: value.id,
            completed_at: value.completed_at,
            target_application: value.target_application,
            raw_transcript: value.raw_transcript,
            deterministic_text: value.deterministic_text,
            refined_text: value.refined_text,
            final_text: value.final_text,
            cleanup_level: "balanced".to_string(),
        }
    }
}

fn default_settings() -> SettingsSnapshot {
    SettingsSnapshot {
        hotkeys: HotkeySettings {
            default_hotkey: "Ctrl+Alt+Space".to_string(),
            activation_mode: ActivationMode::PushToTalk,
            command_hotkey: "Ctrl+Alt+Shift+Space".to_string(),
        },
        models: ModelSettings {
            whisper_model_path: String::new(),
            language: "auto".to_string(),
            asr_threads: 4,
            ollama_model: "gemma4:12b-it-qat".to_string(),
            low_resource_mode: false,
        },
        microphone: MicrophoneSettings {
            selected_device_id: "default".to_string(),
            selected_device_name: "Default microphone".to_string(),
            vad_enabled: true,
            end_of_speech_ms: 900,
            max_recording_seconds: 120,
        },
        privacy: PrivacySettings {
            history_retention: "original_and_cleaned".to_string(),
            delete_after: "7d".to_string(),
            active_app_detection: true,
            accessibility_context: false,
            selected_text_transforms: false,
            context_retention: false,
            delete_audio_after_processing: true,
        },
        dictionary: vec![DictionaryEntry {
            id: "dict-pytorch".to_string(),
            phrase: "PyTorch".to_string(),
            pronunciation_hint: Some("pie torch".to_string()),
            category: "technical".to_string(),
            case_sensitive: false,
        }],
        replacements: vec![ReplacementRule {
            id: "replace-pytorch".to_string(),
            incorrect: "pie torch".to_string(),
            correct: "PyTorch".to_string(),
            enabled: true,
        }],
        snippets: vec![Snippet {
            id: "snippet-signature".to_string(),
            trigger: "insert my signature".to_string(),
            expansion: "Best,\nLocalFlow".to_string(),
            enabled: true,
            allow_cleanup: false,
        }],
        styles: vec![StyleProfile {
            id: "style-work".to_string(),
            name: "Work messages".to_string(),
            category: "work_messaging".to_string(),
            cleanup_level: "balanced".to_string(),
            conciseness: 6,
            formality: 6,
            contractions: true,
            emoji: "preserve".to_string(),
            paragraph_length: "short".to_string(),
            bullet_preference: "preserve".to_string(),
            greeting_behavior: "preserve".to_string(),
            sign_off_behavior: "preserve".to_string(),
            aggressive_filler_removal: false,
            allow_sentence_fragments: true,
        }],
    }
}

fn default_diagnostics() -> Vec<DiagnosticMetric> {
    vec![
        DiagnosticMetric {
            label: "Tray".to_string(),
            value: "Configured at startup".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "Global hotkey".to_string(),
            value: "Ctrl+Alt+Space preferred; Ctrl+Alt+Shift+Space fallback".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "Refinement model".to_string(),
            value: "gemma4:12b-it-qat through local Ollama for native dictation".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "ASR provider".to_string(),
            value: "Native whisper.cpp path plus mock UI controls".to_string(),
            status: "warning".to_string(),
        },
    ]
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "LocalFlow runtime state lock was poisoned.".to_string()
}
