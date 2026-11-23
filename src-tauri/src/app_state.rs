use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::{
    asr::{AsrProvider, MockAsrProvider},
    insertion::{MockTextInserter, TextInserter},
    native_dictation::NativeDictationRuntime,
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

impl LocalFlowRuntime {
    /// A snapshot of the current settings for the native dictation path to read. Recovers a
    /// poisoned lock rather than propagating, so a panic elsewhere cannot wedge dictation.
    pub fn current_settings(&self) -> SettingsSnapshot {
        self.settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn replace_settings(&self, settings: SettingsSnapshot) {
        let mut guard = self
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = settings;
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

/// Persisted user settings.
///
/// Every settings struct is `#[serde(default)]`: a field that is missing from stored JSON
/// takes its default instead of failing the whole load. Without that, the first release that
/// adds a field would discard the user's saved dictionary, replacements and snippets.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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
#[serde(default, rename_all = "camelCase")]
pub struct HotkeySettings {
    pub default_hotkey: String,
    pub activation_mode: ActivationMode,
    pub command_hotkey: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModelSettings {
    pub whisper_model_path: String,
    pub language: String,
    pub asr_threads: u8,
    pub ollama_model: String,
    pub low_resource_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MicrophoneSettings {
    pub selected_device_id: String,
    pub selected_device_name: String,
    pub vad_enabled: bool,
    pub end_of_speech_ms: u16,
    pub max_recording_seconds: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PrivacySettings {
    pub history_retention: String,
    pub delete_after: String,
    pub active_app_detection: bool,
    pub accessibility_context: bool,
    pub selected_text_transforms: bool,
    pub context_retention: bool,
    pub delete_audio_after_processing: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DictionaryEntry {
    pub id: String,
    pub phrase: String,
    pub pronunciation_hint: Option<String>,
    pub category: String,
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ReplacementRule {
    pub id: String,
    pub incorrect: String,
    pub correct: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub trigger: String,
    pub expansion: String,
    pub enabled: bool,
    pub allow_cleanup: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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
pub fn get_status(
    app: AppHandle,
    runtime: State<'_, LocalFlowRuntime>,
    native: State<'_, NativeDictationRuntime>,
) -> Result<AppStatus, String> {
    build_status(&runtime, Some(&native), Some(&app))
}

fn build_status(
    runtime: &LocalFlowRuntime,
    native: Option<&NativeDictationRuntime>,
    app: Option<&AppHandle>,
) -> Result<AppStatus, String> {
    let mut diagnostics = default_diagnostics();
    if let Some(native) = native {
        diagnostics.extend(native.latency_diagnostics());
    }
    if let Some(app) = app {
        diagnostics.extend(crate::desktop_health::diagnostics(app));
        diagnostics.extend(crate::conversation::diagnostics(app));
    }

    Ok(AppStatus {
        workflow: runtime.workflow.lock().map_err(lock_error)?.clone(),
        settings: runtime.settings.lock().map_err(lock_error)?.clone(),
        history: runtime.history.lock().map_err(lock_error)?.clone(),
        diagnostics,
    })
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    runtime: State<'_, LocalFlowRuntime>,
    native: State<'_, NativeDictationRuntime>,
    settings: SettingsSnapshot,
) -> Result<AppStatus, String> {
    crate::storage::save_settings(&app, &settings)
        .map_err(|error| format!("Could not persist settings: {error}"))?;
    *runtime.settings.lock().map_err(lock_error)? = settings;
    build_status(&runtime, Some(&native), Some(&app))
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
    app: AppHandle,
    runtime: State<'_, LocalFlowRuntime>,
    native: State<'_, NativeDictationRuntime>,
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
    build_status(&runtime, Some(&native), Some(&app))
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

pub fn default_settings() -> SettingsSnapshot {
    SettingsSnapshot::default()
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            default_hotkey: "Ctrl+Alt+Space".to_string(),
            activation_mode: ActivationMode::PushToTalk,
            command_hotkey: "Ctrl+Alt+Shift+Space".to_string(),
        }
    }
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            whisper_model_path: String::new(),
            language: "auto".to_string(),
            asr_threads: 4,
            ollama_model: "llama3.2:3b".to_string(),
            low_resource_mode: false,
        }
    }
}

impl Default for MicrophoneSettings {
    fn default() -> Self {
        Self {
            selected_device_id: "default".to_string(),
            selected_device_name: "Default microphone".to_string(),
            vad_enabled: true,
            end_of_speech_ms: 900,
            max_recording_seconds: 120,
        }
    }
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            history_retention: "original_and_cleaned".to_string(),
            delete_after: "7d".to_string(),
            active_app_detection: true,
            accessibility_context: false,
            selected_text_transforms: false,
            context_retention: false,
            delete_audio_after_processing: true,
        }
    }
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            hotkeys: HotkeySettings::default(),
            models: ModelSettings::default(),
            microphone: MicrophoneSettings::default(),
            privacy: PrivacySettings::default(),
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
}

fn default_diagnostics() -> Vec<DiagnosticMetric> {
    vec![
        DiagnosticMetric {
            label: "Dictation pipeline".to_string(),
            value: "Native cpal capture -> local speech engine (Parakeet in-process, Whisper fallback) -> deterministic quick insert -> background local Ollama cleanup".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "Insertion safety".to_string(),
            value: "Session guard + target-window revalidation; Escape cancels; recover with Copy last transcript".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "Global hotkey".to_string(),
            value: "Ctrl+Alt+Space preferred; Ctrl+Alt+Shift+Space fallback".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "Refinement model".to_string(),
            value: "Configurable local Ollama model (default llama3.2:3b); dictionary biases ASR".to_string(),
            status: "ok".to_string(),
        },
        DiagnosticMetric {
            label: "Streaming ASR".to_string(),
            value: "Whole-utterance transcription on a model kept loaded; streaming partials are planned".to_string(),
            status: "warning".to_string(),
        },
    ]
}

fn lock_error<T>(_: std::sync::PoisonError<T>) -> String {
    "LocalFlow runtime state lock was poisoned.".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_saved_before_a_field_existed_still_load() {
        // Stored JSON from an older build: `models` lacks `lowResourceMode`, the whole
        // `microphone` section is absent, and a replacement lacks `enabled`.
        let stored = r#"{
            "hotkeys": {"defaultHotkey": "Ctrl+Alt+Space", "commandHotkey": "Ctrl+Alt+Shift+Space"},
            "models": {"whisperModelPath": "", "language": "de", "asrThreads": 2, "ollamaModel": "custom:1b"},
            "privacy": {"historyRetention": "off"},
            "dictionary": [{"id": "d1", "phrase": "LocalFlow", "category": "product", "caseSensitive": true}],
            "replacements": [{"id": "r1", "incorrect": "local flow", "correct": "LocalFlow"}],
            "snippets": [],
            "styles": []
        }"#;

        let settings: SettingsSnapshot =
            serde_json::from_str(stored).expect("older settings should still load");

        // What the user saved is kept...
        assert_eq!(settings.models.language, "de");
        assert_eq!(settings.models.ollama_model, "custom:1b");
        assert_eq!(settings.privacy.history_retention, "off");
        assert_eq!(settings.dictionary.len(), 1);
        assert_eq!(settings.dictionary[0].phrase, "LocalFlow");
        assert_eq!(settings.replacements[0].correct, "LocalFlow");
        assert!(settings.snippets.is_empty());
        // ...and everything that was missing takes its default.
        assert!(!settings.models.low_resource_mode);
        assert_eq!(settings.privacy.delete_after, "7d");
        assert_eq!(settings.microphone.selected_device_id, "default");
    }

    #[test]
    fn unknown_fields_from_a_newer_build_are_ignored() {
        let stored = r#"{"models": {"ollamaModel": "x:1b", "fieldFromTheFuture": 3}, "conversation": {"enabled": true}}"#;

        let settings: SettingsSnapshot =
            serde_json::from_str(stored).expect("newer settings should still load");

        assert_eq!(settings.models.ollama_model, "x:1b");
    }

    #[test]
    fn defaults_round_trip_unchanged() {
        let defaults = default_settings();
        let json = serde_json::to_string(&defaults).unwrap();
        let loaded: SettingsSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(serde_json::to_string(&loaded).unwrap(), json);
    }
}
