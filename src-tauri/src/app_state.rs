use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Manager, State};

use crate::{
    asr::{AsrProvider, MockAsrProvider},
    coding_agents::CodingAgentSettings,
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
    /// Set at start-up when some or all settings had to fall back to their defaults.
    storage_notice: Mutex<Option<String>>,
}

impl Default for LocalFlowRuntime {
    fn default() -> Self {
        Self {
            workflow: Mutex::new(WorkflowState::default()),
            settings: Mutex::new(default_settings()),
            history: Mutex::new(Vec::new()),
            storage_notice: Mutex::new(None),
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

    /// Record why the stored settings could not be used as they were, for the settings window
    /// and diagnostics.
    pub fn set_storage_notice(&self, notice: Option<String>) {
        *self
            .storage_notice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = notice;
    }

    pub fn storage_notice(&self) -> Option<String> {
        self.storage_notice
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Change the settings under the lock: `change` edits a copy, `persist` stores it, and
    /// only if both succeed does the copy replace the current settings.
    pub fn update_settings<T>(
        &self,
        change: impl FnOnce(&mut SettingsSnapshot) -> Result<T, String>,
        persist: impl FnOnce(&SettingsSnapshot) -> Result<(), String>,
    ) -> Result<T, String> {
        let mut guard = self
            .settings
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut next = guard.clone();
        let value = change(&mut next)?;
        persist(&next)?;
        *guard = next;
        Ok(value)
    }
}

/// The settings to keep when the web UI saves `incoming`. Coding-agent projects are changed
/// only through their own commands, which check every folder, so whatever the web UI sends
/// for them is ignored and the current projects are kept.
pub fn merge_saved_settings(
    current: &SettingsSnapshot,
    mut incoming: SettingsSnapshot,
) -> SettingsSnapshot {
    incoming.coding_agents = current.coding_agents.clone();
    incoming
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub workflow: WorkflowState,
    pub settings: SettingsSnapshot,
    pub history: Vec<HistoryItem>,
    pub diagnostics: Vec<DiagnosticMetric>,
    /// Set when LocalFlow started on default settings for some or all of them (a damaged
    /// database, a stored value it could not read, or a database it cannot write). A sentence
    /// for the user; `null` otherwise.
    pub storage_notice: Option<String>,
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
    pub coding_agents: CodingAgentSettings,
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

/// Runs on a worker thread: building the status reads diagnostics, and a command that is not
/// async runs on the main thread, where it would hold up the hotkeys and the overlay.
#[tauri::command]
pub async fn get_status(app: AppHandle) -> Result<AppStatus, String> {
    run_blocking(move || {
        let runtime = app.state::<LocalFlowRuntime>();
        let native = app.state::<NativeDictationRuntime>();
        build_status(&runtime, Some(&native), Some(&app))
    })
    .await
}

/// Run `work` on the blocking thread pool and wait for it.
async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| "LocalFlow stopped the request unexpectedly.".to_string())?
}

fn build_status(
    runtime: &LocalFlowRuntime,
    native: Option<&NativeDictationRuntime>,
    app: Option<&AppHandle>,
) -> Result<AppStatus, String> {
    let mut diagnostics = default_diagnostics();
    diagnostics.push(storage_diagnostic(runtime.storage_notice().as_deref()));
    if let Some(native) = native {
        diagnostics.extend(native.latency_diagnostics());
    }
    if let Some(app) = app {
        diagnostics.extend(crate::desktop_health::diagnostics(app));
        diagnostics.extend(crate::conversation::diagnostics(app));
    }
    redact_diagnostics(&mut diagnostics);

    Ok(AppStatus {
        workflow: runtime.workflow.lock().map_err(lock_error)?.clone(),
        settings: runtime.settings.lock().map_err(lock_error)?.clone(),
        history: runtime.history.lock().map_err(lock_error)?.clone(),
        diagnostics,
        storage_notice: runtime.storage_notice(),
    })
}

/// Diagnostics are shown and exported, so no row may carry the Windows user name.
fn redact_diagnostics(rows: &mut [DiagnosticMetric]) {
    for row in rows {
        if let std::borrow::Cow::Owned(redacted) =
            crate::storage::redact::redact_user_paths(&row.value)
        {
            row.value = redacted;
        }
    }
}

fn storage_diagnostic(notice: Option<&str>) -> DiagnosticMetric {
    DiagnosticMetric {
        label: "Settings storage".to_string(),
        value: notice.unwrap_or("Saved settings loaded").to_string(),
        status: if notice.is_some() { "warning" } else { "ok" }.to_string(),
    }
}

/// Persists on a worker thread (see [`get_status`]). Saves are applied under the settings
/// lock, one at a time.
#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    settings: SettingsSnapshot,
) -> Result<AppStatus, String> {
    run_blocking(move || {
        let runtime = app.state::<LocalFlowRuntime>();
        let native = app.state::<NativeDictationRuntime>();
        runtime.update_settings(
            |current| {
                *current = merge_saved_settings(current, settings);
                Ok(())
            },
            |next| {
                crate::storage::save_settings(&app, next)
                    .map_err(|error| format!("Could not persist settings: {error}"))
            },
        )?;
        build_status(&runtime, Some(&native), Some(&app))
    })
    .await
}

/// Debug builds only: start a mock dictation.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn begin_mock_session(runtime: State<'_, LocalFlowRuntime>) -> Result<WorkflowState, String> {
    begin_mock(&runtime)
}

/// Debug builds only: finish a mock dictation with `raw_transcript`.
#[cfg(debug_assertions)]
#[tauri::command]
pub fn finish_mock_session(
    app: AppHandle,
    runtime: State<'_, LocalFlowRuntime>,
    native: State<'_, NativeDictationRuntime>,
    raw_transcript: String,
) -> Result<AppStatus, String> {
    finish_mock(&runtime, &raw_transcript)?;
    build_status(&runtime, Some(&native), Some(&app))
}

// The mock dictation behind the debug-only commands above. A release build has no command
// that reaches it and the linker drops it; it stays compiled so the mock providers and
// workflow events it drives are not reported as unused in their own modules.
#[cfg_attr(not(any(debug_assertions, test)), allow(dead_code))]
fn begin_mock(runtime: &LocalFlowRuntime) -> Result<WorkflowState, String> {
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

#[cfg_attr(not(any(debug_assertions, test)), allow(dead_code))]
fn finish_mock(runtime: &LocalFlowRuntime, raw_transcript: &str) -> Result<(), String> {
    let asr = MockAsrProvider::new(raw_transcript.to_string());
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
    Ok(())
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

/// Read stored settings JSON.
///
/// A value that does not fit its field — a number out of the field's range, a word from a
/// newer or older build, a value of the wrong kind — is corrected or left at its default
/// instead of failing the whole load, which would have reset every setting. Numbers are
/// clamped to the field's range (and rounded when a whole number is needed); anything else
/// that does not fit is dropped, so the field keeps its default. A list keeps every entry that
/// can be read. The second value counts the values corrected or dropped. Only text that is not
/// JSON at all is an error.
pub fn parse_settings(json: &str) -> Result<(SettingsSnapshot, usize), serde_json::Error> {
    let stored: Value = serde_json::from_str(json)?;
    if let Ok(settings) = SettingsSnapshot::deserialize(&stored) {
        return Ok((settings, 0));
    }

    let mut repairs = 0;
    let mut root = Value::Object(serde_json::Map::new());
    match &stored {
        Value::Object(fields) => {
            for (key, value) in fields {
                let path = [Step::Key(key.clone())];
                if !rebuild(&mut root, &path, value, &mut repairs) {
                    remove_at(&mut root, &path);
                    repairs += 1;
                }
            }
        }
        _ => repairs += 1,
    }
    // `root` only ever holds what was accepted, so this cannot fail; the fallback is the
    // defaults all the same.
    let settings = SettingsSnapshot::deserialize(&root).unwrap_or_default();
    Ok((settings, repairs))
}

#[derive(Debug, Clone)]
enum Step {
    Key(String),
    Index(usize),
}

fn settings_accept(root: &Value) -> bool {
    SettingsSnapshot::deserialize(root).is_ok()
}

fn slot_mut<'a>(root: &'a mut Value, path: &[Step]) -> Option<&'a mut Value> {
    path.iter().try_fold(root, |current, step| match step {
        Step::Key(key) => current.as_object_mut()?.get_mut(key),
        Step::Index(index) => current.as_array_mut()?.get_mut(*index),
    })
}

fn set_at(root: &mut Value, path: &[Step], value: Value) {
    let Some((last, parent)) = path.split_last() else {
        *root = value;
        return;
    };
    match (slot_mut(root, parent), last) {
        (Some(Value::Object(fields)), Step::Key(key)) => {
            fields.insert(key.clone(), value);
        }
        (Some(Value::Array(items)), Step::Index(index)) => {
            if let Some(item) = items.get_mut(*index) {
                *item = value;
            }
        }
        _ => {}
    }
}

fn remove_at(root: &mut Value, path: &[Step]) {
    let Some((last, parent)) = path.split_last() else {
        return;
    };
    match (slot_mut(root, parent), last) {
        (Some(Value::Object(fields)), Step::Key(key)) => {
            fields.remove(key);
        }
        (Some(Value::Array(items)), Step::Index(index)) if *index < items.len() => {
            items.remove(*index);
        }
        _ => {}
    }
}

/// Numbers to try, widest range first, for a stored number its field refused: the value
/// rounded, then clamped to the unsigned ranges the settings use.
fn number_candidates(number: &serde_json::Number) -> Vec<Value> {
    let Some(value) = number.as_f64().filter(|value| value.is_finite()) else {
        return Vec::new();
    };
    let rounded = value.round();
    let mut candidates = Vec::new();
    if rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
        candidates.push(Value::from(rounded as i64));
    }
    for max in [u32::MAX as f64, u16::MAX as f64, u8::MAX as f64] {
        candidates.push(Value::from(rounded.clamp(0.0, max) as u64));
    }
    candidates
}

/// Put `stored` at `path` in `root`, keeping only what still reads as settings. Returns
/// whether anything was kept there; the caller removes the slot otherwise.
fn rebuild(root: &mut Value, path: &[Step], stored: &Value, repairs: &mut usize) -> bool {
    set_at(root, path, stored.clone());
    if settings_accept(root) {
        return true;
    }
    match stored {
        Value::Object(fields) => {
            set_at(root, path, Value::Object(serde_json::Map::new()));
            if !settings_accept(root) {
                return false;
            }
            for (key, value) in fields {
                let mut child = path.to_vec();
                child.push(Step::Key(key.clone()));
                if !rebuild(root, &child, value, repairs) {
                    remove_at(root, &child);
                    *repairs += 1;
                }
            }
            true
        }
        Value::Array(items) => {
            set_at(root, path, Value::Array(Vec::new()));
            if !settings_accept(root) {
                return false;
            }
            for item in items {
                let index = match slot_mut(root, path) {
                    Some(Value::Array(kept)) => {
                        kept.push(Value::Null);
                        kept.len() - 1
                    }
                    _ => return true,
                };
                let mut child = path.to_vec();
                child.push(Step::Index(index));
                if !rebuild(root, &child, item, repairs) {
                    remove_at(root, &child);
                    *repairs += 1;
                }
            }
            true
        }
        Value::Number(number) => {
            for candidate in number_candidates(number) {
                set_at(root, path, candidate);
                if settings_accept(root) {
                    *repairs += 1;
                    return true;
                }
            }
            false
        }
        _ => false,
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
            coding_agents: CodingAgentSettings::default(),
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
    fn coding_projects_load_and_settings_without_them_still_load() {
        let stored = r#"{"codingAgents": {"projects": [{"id": "p1", "path": "C:\\work\\app", "spokenNames": ["app"]}]}}"#;
        let settings: SettingsSnapshot = serde_json::from_str(stored).unwrap();
        assert_eq!(settings.coding_agents.projects[0].spoken_names, ["app"]);

        let older: SettingsSnapshot = serde_json::from_str(r#"{"snippets": []}"#).unwrap();
        assert!(older.coding_agents.projects.is_empty());
    }

    #[test]
    fn a_general_save_cannot_change_coding_projects() {
        let mut current = default_settings();
        current
            .coding_agents
            .projects
            .push(crate::coding_agents::CodingProject {
                id: "p1".into(),
                path: r"C:\work\app".into(),
                spoken_names: vec!["app".into()],
            });
        let mut incoming = default_settings();
        incoming.models.language = "de".into();
        incoming
            .coding_agents
            .projects
            .push(crate::coding_agents::CodingProject {
                id: "evil".into(),
                path: r"C:\work\a;calc".into(),
                spoken_names: vec!["x".into()],
            });

        let merged = merge_saved_settings(&current, incoming);

        assert_eq!(merged.models.language, "de");
        assert_eq!(merged.coding_agents, current.coding_agents);
    }

    #[test]
    fn a_failed_persist_leaves_the_settings_unchanged() {
        let runtime = LocalFlowRuntime::default();
        let result = runtime.update_settings(
            |settings| {
                settings.models.language = "fr".into();
                Ok(())
            },
            |_| Err("disk full".into()),
        );
        assert_eq!(result, Err("disk full".to_string()));
        assert_eq!(runtime.current_settings().models.language, "auto");
    }

    #[test]
    fn defaults_round_trip_unchanged() {
        let defaults = default_settings();
        let json = serde_json::to_string(&defaults).unwrap();
        let loaded: SettingsSnapshot = serde_json::from_str(&json).unwrap();

        assert_eq!(serde_json::to_string(&loaded).unwrap(), json);
    }

    #[test]
    fn well_formed_settings_need_no_repair() {
        let json = serde_json::to_string(&default_settings()).unwrap();
        let (settings, repaired) = parse_settings(&json).unwrap();
        assert_eq!(repaired, 0);
        assert_eq!(serde_json::to_string(&settings).unwrap(), json);
    }

    #[test]
    fn an_unknown_activation_mode_falls_back_without_losing_the_rest() {
        let stored = r#"{
            "hotkeys": {"defaultHotkey": "Ctrl+Alt+D", "activationMode": "hold_and_wiggle"},
            "models": {"language": "de"},
            "snippets": [{"id": "s1", "trigger": "sig", "expansion": "Bye", "enabled": true}]
        }"#;
        assert!(serde_json::from_str::<SettingsSnapshot>(stored).is_err());

        let (settings, repaired) = parse_settings(stored).unwrap();

        assert_eq!(repaired, 1);
        assert_eq!(settings.hotkeys.default_hotkey, "Ctrl+Alt+D");
        assert!(matches!(
            settings.hotkeys.activation_mode,
            ActivationMode::PushToTalk
        ));
        assert_eq!(settings.models.language, "de");
        assert_eq!(settings.snippets[0].expansion, "Bye");
    }

    #[test]
    fn out_of_range_numbers_are_clamped_to_their_field() {
        let stored = r#"{
            "models": {"asrThreads": 900, "ollamaModel": "kept:1b"},
            "microphone": {"endOfSpeechMs": 100000, "maxRecordingSeconds": -3},
            "styles": [{"id": "st", "name": "Mine", "conciseness": 6.6, "formality": 300}]
        }"#;

        let (settings, repaired) = parse_settings(stored).unwrap();

        assert_eq!(repaired, 5);
        assert_eq!(settings.models.asr_threads, u8::MAX);
        assert_eq!(settings.models.ollama_model, "kept:1b");
        assert_eq!(settings.microphone.end_of_speech_ms, u16::MAX);
        assert_eq!(settings.microphone.max_recording_seconds, 0);
        assert_eq!(settings.styles[0].name, "Mine");
        assert_eq!(settings.styles[0].conciseness, 7);
        assert_eq!(settings.styles[0].formality, u8::MAX);
    }

    #[test]
    fn a_bad_list_entry_keeps_the_entry_and_the_other_entries() {
        let stored = r#"{"dictionary": [
            {"id": "d1", "phrase": "Tauri", "caseSensitive": "very"},
            {"id": "d2", "phrase": "cpal", "caseSensitive": true},
            "not an entry"
        ]}"#;

        let (settings, repaired) = parse_settings(stored).unwrap();

        assert_eq!(repaired, 2);
        let phrases: Vec<&str> = settings
            .dictionary
            .iter()
            .map(|entry| entry.phrase.as_str())
            .collect();
        assert_eq!(phrases, ["Tauri", "cpal"]);
        assert!(!settings.dictionary[0].case_sensitive);
        assert!(settings.dictionary[1].case_sensitive);
    }

    #[test]
    fn a_section_of_the_wrong_kind_takes_its_defaults() {
        let (settings, repaired) =
            parse_settings(r#"{"models": 5, "privacy": {"historyRetention": "off"}}"#).unwrap();
        assert_eq!(repaired, 1);
        assert_eq!(settings.models.ollama_model, "llama3.2:3b");
        assert_eq!(settings.privacy.history_retention, "off");

        let (settings, repaired) = parse_settings("[1, 2]").unwrap();
        assert_eq!(repaired, 1);
        assert_eq!(settings.models.language, "auto");
    }

    #[test]
    fn text_that_is_not_json_is_still_an_error() {
        assert!(parse_settings("{not json").is_err());
    }

    #[test]
    fn the_storage_notice_is_shown_as_a_warning_row() {
        let ok = storage_diagnostic(None);
        assert_eq!(ok.status, "ok");
        let warning = storage_diagnostic(Some("The settings database was damaged."));
        assert_eq!(warning.status, "warning");
        assert_eq!(warning.value, "The settings database was damaged.");

        let runtime = LocalFlowRuntime::default();
        assert_eq!(runtime.storage_notice(), None);
        runtime.set_storage_notice(Some("reset".into()));
        let status = build_status(&runtime, None, None).unwrap();
        assert_eq!(status.storage_notice.as_deref(), Some("reset"));
        assert!(status
            .diagnostics
            .iter()
            .any(|row| row.label == "Settings storage" && row.value == "reset"));
    }

    #[test]
    fn diagnostics_never_carry_the_user_folder() {
        let mut rows = vec![DiagnosticMetric {
            label: "ASR engine".into(),
            value: r"model C:\Users\someone\models\x.gguf".into(),
            status: "ok".into(),
        }];
        redact_diagnostics(&mut rows);
        assert_eq!(rows[0].value, r"model <user folder>\models\x.gguf");
    }

    #[test]
    fn the_mock_dictation_records_one_history_item() {
        let runtime = LocalFlowRuntime::default();
        begin_mock(&runtime).unwrap();
        finish_mock(&runtime, "hello there").unwrap();
        let history = runtime.history.lock().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].final_text, "Hello there.");
    }
}
