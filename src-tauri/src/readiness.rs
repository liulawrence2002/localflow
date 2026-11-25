//! What LocalFlow needs installed, and what is missing: the checklist the settings window
//! shows.
//!
//! Every check is local: files in the runtime folders ([`crate::runtime_paths`]) and one
//! request to Ollama on loopback, with a short time limit. Labels and fix instructions name
//! a script or a command, never a local path, because paths carry the user's name.

use std::{
    env,
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    app_state::LocalFlowRuntime,
    audio::{smart_turn::SMART_TURN_MODEL_FILE, speech_detector::SILERO_MODEL_FILE},
    llm::ollama::{is_cloud_model_tag, list_local_models, OLLAMA_MODEL},
    runtime_paths::{
        preferred_model_candidates, runtime_candidates, whisper_program_dir,
        FALLBACK_WHISPER_MODELS, PREFERRED_SPEECH_MODELS, WHISPER_CLI_FILE, WHISPER_SERVER_FILE,
    },
    tts::phonon::{PhononConfig, PhononProvider, PHONON_MODEL_DIR},
};

/// How long the checklist waits for Ollama to list its models. Ollama on loopback answers
/// in milliseconds; a refused connection fails at once.
const OLLAMA_CHECK_LIMIT: Duration = Duration::from_millis(2500);

/// One line of the checklist. Serialized camelCase for the settings window:
/// `{ id, label, ready, required, fix }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessItem {
    /// Stable identifier: `speech-model`, `conversation-voice`, `silero-vad`, `smart-turn`,
    /// `ollama`, `whisper-fallback` or `fish-s2`.
    pub id: String,
    /// What the item is, for people.
    pub label: String,
    /// Whether it is installed and usable.
    pub ready: bool,
    /// Whether a main feature (dictation or conversation) needs it. Optional items only
    /// improve something or add an extra feature.
    pub required: bool,
    /// A short instruction to fix it when it is not ready. Never contains a path.
    pub fix: Option<String>,
}

impl ReadinessItem {
    fn new(id: &str, label: impl Into<String>, required: bool, fix: Option<String>) -> Self {
        Self {
            id: id.to_string(),
            label: label.into(),
            ready: fix.is_none(),
            required,
            fix,
        }
    }
}

/// What asking Ollama for its models gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OllamaProbe {
    /// Ollama answered; these local models are installed (cloud-hosted tags already removed).
    Models(Vec<String>),
    /// Nothing answered on the loopback port, or it answered with an error.
    NotAnswering,
    /// No answer within [`OLLAMA_CHECK_LIMIT`].
    TimedOut,
}

/// The checklist, for the settings window. Runs off the main thread: the file checks are
/// quick, and the Ollama request is bounded by [`OLLAMA_CHECK_LIMIT`].
#[tauri::command]
pub async fn readiness_status(app: AppHandle) -> Result<Vec<ReadinessItem>, String> {
    tauri::async_runtime::spawn_blocking(move || current_checklist(&app))
        .await
        .map_err(|_| "The readiness check stopped unexpectedly.".to_string())
}

fn current_checklist(app: &AppHandle) -> Vec<ReadinessItem> {
    let ollama = probe_ollama_in_background();
    let settings = app.state::<LocalFlowRuntime>().current_settings();
    // The same override the speech engine honours (see runtime_paths::speech_model_path).
    let configured_model = env::var("LOCALFLOW_WHISPER_MODEL")
        .unwrap_or_else(|_| settings.models.whisper_model_path.clone());
    let runtimes = runtime_candidates(app);
    let probe = match ollama.recv_timeout(OLLAMA_CHECK_LIMIT) {
        Ok(probe) => probe,
        Err(mpsc::RecvTimeoutError::Timeout) => OllamaProbe::TimedOut,
        Err(mpsc::RecvTimeoutError::Disconnected) => OllamaProbe::NotAnswering,
    };

    let items = checklist(
        &runtimes,
        &configured_model,
        &settings.models.ollama_model,
        &probe,
    );
    tracing::info!(
        ready = items.iter().filter(|item| item.ready).count(),
        missing = items.iter().filter(|item| !item.ready).count(),
        "readiness checked"
    );
    items
}

/// Ask Ollama for its models on a thread of its own, so a slow answer cannot hold up the
/// checklist beyond the time limit. The request has its own timeouts, so the thread ends by
/// itself.
fn probe_ollama_in_background() -> mpsc::Receiver<OllamaProbe> {
    let (sender, receiver) = mpsc::channel();
    let spawned = thread::Builder::new()
        .name("localflow-readiness-ollama".to_string())
        .spawn(move || {
            let probe = match list_local_models() {
                Ok(models) => OllamaProbe::Models(models),
                Err(_) => OllamaProbe::NotAnswering,
            };
            let _ = sender.send(probe);
        });
    if spawned.is_err() {
        tracing::warn!("could not start the Ollama readiness check");
    }
    receiver
}

/// The whole checklist, in display order. `runtimes` are the runtime folders in search
/// order, `configured_model` is the speech model path from settings (empty for the default)
/// and `ollama_model` the Ollama model from settings.
pub(crate) fn checklist(
    runtimes: &[PathBuf],
    configured_model: &str,
    ollama_model: &str,
    ollama: &OllamaProbe,
) -> Vec<ReadinessItem> {
    vec![
        speech_model_item(runtimes, configured_model),
        conversation_voice_item(runtimes),
        model_file_item(
            "silero-vad",
            "Voice detection (Silero VAD)",
            runtimes,
            SILERO_MODEL_FILE,
            "Run scripts\\Get-VadModel.ps1",
        ),
        model_file_item(
            "smart-turn",
            "End-of-turn model (Smart Turn)",
            runtimes,
            SMART_TURN_MODEL_FILE,
            "Run scripts\\Get-SmartTurnModel.ps1",
        ),
        ollama_item(ollama_model, ollama),
        whisper_fallback_item(runtimes),
        fish_item(runtimes),
    ]
}

fn any_file(candidates: impl IntoIterator<Item = PathBuf>) -> bool {
    candidates.into_iter().any(|path| path.is_file())
}

fn speech_model_item(runtimes: &[PathBuf], configured_model: &str) -> ReadinessItem {
    let configured = configured_model.trim();
    if !configured.is_empty() {
        let fix = (!Path::new(configured).is_file()).then(|| {
            "Clear the model path in Settings > Models, or choose a file that exists".to_string()
        });
        return ReadinessItem::new(
            "speech-model",
            "Speech recognition (the model set in Settings)",
            true,
            fix,
        );
    }

    let parakeet = preferred_model_candidates(runtimes, &PREFERRED_SPEECH_MODELS[..1]);
    let fix = (!any_file(parakeet)).then(|| "Run scripts\\Get-AsrModel.ps1".to_string());
    ReadinessItem::new("speech-model", "Speech recognition (Parakeet)", true, fix)
}

fn conversation_voice_item(runtimes: &[PathBuf]) -> ReadinessItem {
    let installed = runtimes.iter().any(|runtime| {
        PhononProvider::missing_file(&PhononConfig {
            model_dir: runtime.join("models").join(PHONON_MODEL_DIR),
        })
        .is_none()
    });
    let fix = (!installed).then(|| "Run scripts\\Get-PhononModel.ps1".to_string());
    ReadinessItem::new(
        "conversation-voice",
        "Conversation voice (Phonon)",
        true,
        fix,
    )
}

/// An optional model that is one file in the `models` folder.
fn model_file_item(
    id: &str,
    label: &str,
    runtimes: &[PathBuf],
    file: &str,
    fix: &str,
) -> ReadinessItem {
    let installed = any_file(preferred_model_candidates(runtimes, &[file]));
    ReadinessItem::new(id, label, false, (!installed).then(|| fix.to_string()))
}

fn ollama_item(configured_model: &str, probe: &OllamaProbe) -> ReadinessItem {
    let wanted = match configured_model.trim() {
        "" => OLLAMA_MODEL,
        name => name,
    };
    let shown = speakable_model_name(wanted);
    let label = match shown {
        Some(name) => format!("Language model (Ollama, {name})"),
        None => "Language model (Ollama)".to_string(),
    };
    let pull = match shown {
        Some(name) => format!("ollama pull {name}"),
        None => "ollama pull <the model chosen in Settings > Models>".to_string(),
    };

    let fix = if is_cloud_model_tag(wanted) {
        Some("Choose a local model in Settings > Models; cloud models are refused".to_string())
    } else {
        match probe {
            OllamaProbe::Models(models) if models.iter().any(|m| same_model(m, wanted)) => None,
            OllamaProbe::Models(_) => Some(format!("Run: {pull}")),
            OllamaProbe::NotAnswering => Some(format!("Install Ollama and run: {pull}")),
            OllamaProbe::TimedOut => Some("Restart Ollama, then check again".to_string()),
        }
    };
    ReadinessItem::new("ollama", label, true, fix)
}

/// The model name if it is safe to show and to put in a command: Ollama's own alphabet,
/// bounded in length. Anything else is described instead of echoed.
fn speakable_model_name(name: &str) -> Option<&str> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || "._:/-+".contains(c);
    (!name.is_empty() && name.len() <= 100 && name.chars().all(allowed)).then_some(name)
}

/// Whether an installed Ollama model is the wanted one. A name without a tag means
/// `:latest`, as in Ollama itself; case does not matter.
fn same_model(installed: &str, wanted: &str) -> bool {
    let normalise = |name: &str| {
        let name = name.trim().to_ascii_lowercase();
        if name.contains(':') {
            name
        } else {
            format!("{name}:latest")
        }
    };
    normalise(installed) == normalise(wanted)
}

fn whisper_fallback_item(runtimes: &[PathBuf]) -> ReadinessItem {
    let programs = runtimes.iter().any(|runtime| {
        let dir = whisper_program_dir(runtime);
        dir.join(WHISPER_CLI_FILE).is_file() && dir.join(WHISPER_SERVER_FILE).is_file()
    });
    let model = any_file(preferred_model_candidates(
        runtimes,
        FALLBACK_WHISPER_MODELS,
    ));
    let fix = match (programs, model) {
        (true, true) => None,
        (false, true) => Some("Reinstall LocalFlow, or run scripts\\Get-WhisperRuntime.ps1"),
        (true, false) => Some("Run scripts\\Get-AsrModel.ps1 -Model tiny.en"),
        (false, false) => Some(
            "Reinstall LocalFlow, or run scripts\\Get-WhisperRuntime.ps1 and \
             scripts\\Get-AsrModel.ps1 -Model tiny.en",
        ),
    };
    ReadinessItem::new(
        "whisper-fallback",
        "Whisper fallback (whisper.cpp and tiny.en)",
        true,
        fix.map(str::to_string),
    )
}

/// What the Fish S2 read-aloud voice has under one runtime folder: `fish\s2.exe`, and an
/// `s2-pro-*.gguf` model with its `tokenizer.json` in `models\fish`. Mirrors
/// `tts::locate_fish_install`, which needs all three in the same folder.
fn fish_parts(runtime: &Path) -> (bool, bool) {
    let engine = runtime.join("fish").join("s2.exe").is_file();
    let model_dir = runtime.join("models").join("fish");
    let has_weights = std::fs::read_dir(&model_dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                name.starts_with("s2-pro-") && name.ends_with(".gguf") && entry.path().is_file()
            })
        })
        .unwrap_or(false);
    let model = has_weights && model_dir.join("tokenizer.json").is_file();
    (engine, model)
}

fn fish_item(runtimes: &[PathBuf]) -> ReadinessItem {
    let parts: Vec<(bool, bool)> = runtimes.iter().map(|runtime| fish_parts(runtime)).collect();
    let complete = parts.iter().any(|&(engine, model)| engine && model);
    let engine = parts.iter().any(|&(engine, _)| engine);
    let model = parts.iter().any(|&(_, model)| model);
    let fix = match (complete, engine, model) {
        (true, _, _) => None,
        (false, true, true) => Some(
            "Optional: run scripts\\Get-FishModel.ps1 and scripts\\Build-FishS2.ps1 with the \
             same -Destination",
        ),
        (false, true, false) => {
            Some("Optional: run scripts\\Get-FishModel.ps1 (non-commercial use only)")
        }
        (false, false, true) => Some("Optional: run scripts\\Build-FishS2.ps1"),
        (false, false, false) => Some(
            "Optional: run scripts\\Get-FishModel.ps1 (non-commercial use only) and \
             scripts\\Build-FishS2.ps1",
        ),
    };
    ReadinessItem::new(
        "fish-s2",
        "Read-aloud voice (Fish S2 Pro)",
        false,
        fix.map(str::to_string),
    )
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;
    use crate::tts::phonon::PHONON_REQUIRED_FILES;

    /// A runtime folder in the temp directory, removed when dropped.
    struct TempRuntime {
        root: PathBuf,
    }

    impl TempRuntime {
        fn new(name: &str) -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let root = env::temp_dir().join(format!(
                "localflow-readiness-{name}-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn add(&self, relative: &str) -> &Self {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"x").unwrap();
            self
        }

        fn add_everything(&self) -> &Self {
            self.add("models/parakeet-tdt-0.6b-v3-Q5_K_M.gguf");
            for file in PHONON_REQUIRED_FILES {
                self.add(&format!("models/phonon/{file}"));
            }
            self.add(&format!("models/{SILERO_MODEL_FILE}"))
                .add(&format!("models/{SMART_TURN_MODEL_FILE}"))
                .add("whisper/Release/whisper-cli.exe")
                .add("whisper/Release/whisper-server.exe")
                .add("models/ggml-tiny.en-q5_1.bin")
                .add("fish/s2.exe")
                .add("models/fish/s2-pro-q4_k_m.gguf")
                .add("models/fish/tokenizer.json")
        }

        fn path(&self) -> PathBuf {
            self.root.clone()
        }
    }

    impl Drop for TempRuntime {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn ollama_with(models: &[&str]) -> OllamaProbe {
        OllamaProbe::Models(models.iter().map(|m| m.to_string()).collect())
    }

    fn item<'a>(items: &'a [ReadinessItem], id: &str) -> &'a ReadinessItem {
        items.iter().find(|item| item.id == id).unwrap()
    }

    #[test]
    fn the_checklist_has_every_item_in_order() {
        let runtime = TempRuntime::new("order");
        let items = checklist(&[runtime.path()], "", "", &OllamaProbe::NotAnswering);

        let ids: Vec<&str> = items.iter().map(|item| item.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "speech-model",
                "conversation-voice",
                "silero-vad",
                "smart-turn",
                "ollama",
                "whisper-fallback",
                "fish-s2"
            ]
        );
        let required: Vec<bool> = items.iter().map(|item| item.required).collect();
        assert_eq!(required, [true, true, false, false, true, true, false]);
    }

    #[test]
    fn an_empty_install_says_which_script_to_run_for_each_item() {
        let runtime = TempRuntime::new("empty");
        let items = checklist(&[runtime.path()], "", "", &OllamaProbe::NotAnswering);

        assert!(items.iter().all(|item| !item.ready));
        let fix = |id: &str| item(&items, id).fix.clone().unwrap();
        assert_eq!(fix("speech-model"), "Run scripts\\Get-AsrModel.ps1");
        assert_eq!(
            fix("conversation-voice"),
            "Run scripts\\Get-PhononModel.ps1"
        );
        assert_eq!(fix("silero-vad"), "Run scripts\\Get-VadModel.ps1");
        assert_eq!(fix("smart-turn"), "Run scripts\\Get-SmartTurnModel.ps1");
        assert_eq!(
            fix("ollama"),
            "Install Ollama and run: ollama pull llama3.2:3b"
        );
        assert_eq!(
            fix("whisper-fallback"),
            "Reinstall LocalFlow, or run scripts\\Get-WhisperRuntime.ps1 and \
             scripts\\Get-AsrModel.ps1 -Model tiny.en"
        );
        assert_eq!(
            fix("fish-s2"),
            "Optional: run scripts\\Get-FishModel.ps1 (non-commercial use only) and \
             scripts\\Build-FishS2.ps1"
        );
    }

    #[test]
    fn a_full_install_is_ready_with_no_fixes() {
        let runtime = TempRuntime::new("full");
        runtime.add_everything();

        let items = checklist(
            &[runtime.path()],
            "",
            "llama3.2:3b",
            &ollama_with(&["llama3.2:3b"]),
        );

        for item in &items {
            assert!(item.ready, "{} should be ready", item.id);
            assert_eq!(item.fix, None);
        }
    }

    #[test]
    fn models_are_found_in_any_runtime_folder() {
        let bundled = TempRuntime::new("bundled");
        let per_user = TempRuntime::new("per-user");
        bundled
            .add("whisper/Release/whisper-cli.exe")
            .add("whisper/Release/whisper-server.exe")
            .add("models/ggml-tiny.en-q5_1.bin");
        per_user.add("models/parakeet-tdt-0.6b-v3-Q5_K_M.gguf");

        let items = checklist(
            &[bundled.path(), per_user.path()],
            "",
            "",
            &OllamaProbe::NotAnswering,
        );

        assert!(item(&items, "speech-model").ready);
        assert!(item(&items, "whisper-fallback").ready);
    }

    #[test]
    fn tiny_en_alone_does_not_count_as_the_speech_model() {
        let runtime = TempRuntime::new("tiny-only");
        runtime.add("models/ggml-tiny.en-q5_1.bin");

        let items = checklist(&[runtime.path()], "", "", &OllamaProbe::NotAnswering);

        assert!(!item(&items, "speech-model").ready);
    }

    #[test]
    fn a_configured_model_is_checked_instead_of_parakeet() {
        let runtime = TempRuntime::new("configured");
        runtime.add("custom/my-model.bin");
        let present = runtime.path().join("custom").join("my-model.bin");
        let absent = runtime.path().join("custom").join("gone.bin");

        let found = speech_model_item(&[], present.to_str().unwrap());
        let missing = speech_model_item(&[runtime.path()], absent.to_str().unwrap());

        assert!(found.ready);
        assert_eq!(
            found.label,
            "Speech recognition (the model set in Settings)"
        );
        assert!(!missing.ready);
        assert_eq!(
            missing.fix.as_deref(),
            Some("Clear the model path in Settings > Models, or choose a file that exists")
        );
    }

    #[test]
    fn a_partial_phonon_folder_is_not_ready() {
        let runtime = TempRuntime::new("phonon-partial");
        runtime.add(&format!("models/phonon/{}", PHONON_REQUIRED_FILES[0]));

        assert!(!conversation_voice_item(&[runtime.path()]).ready);
    }

    #[test]
    fn ollama_is_ready_only_with_the_chosen_local_model() {
        let installed = ollama_with(&["llama3.2:3b", "qwen2.5:3b"]);

        assert!(ollama_item("llama3.2:3b", &installed).ready);
        assert!(
            ollama_item("", &installed).ready,
            "empty means the default model"
        );
        assert!(ollama_item(" QWEN2.5:3b ", &installed).ready);
        assert!(ollama_item("mistral", &ollama_with(&["mistral:latest"])).ready);
        assert!(ollama_item("mistral:latest", &ollama_with(&["mistral"])).ready);

        let missing = ollama_item("gemma3:4b", &installed);
        assert!(!missing.ready);
        assert_eq!(missing.fix.as_deref(), Some("Run: ollama pull gemma3:4b"));
        assert_eq!(missing.label, "Language model (Ollama, gemma3:4b)");
    }

    #[test]
    fn ollama_problems_get_their_own_fix() {
        assert_eq!(
            ollama_item("llama3.2:3b", &OllamaProbe::NotAnswering)
                .fix
                .as_deref(),
            Some("Install Ollama and run: ollama pull llama3.2:3b")
        );
        assert_eq!(
            ollama_item("llama3.2:3b", &OllamaProbe::TimedOut)
                .fix
                .as_deref(),
            Some("Restart Ollama, then check again")
        );
        assert_eq!(
            ollama_item("gemma4:cloud", &ollama_with(&["gemma4:cloud"]))
                .fix
                .as_deref(),
            Some("Choose a local model in Settings > Models; cloud models are refused")
        );
    }

    #[test]
    fn an_odd_model_name_is_not_echoed_into_a_command() {
        let item = ollama_item("bad name; rm -rf", &OllamaProbe::NotAnswering);

        assert_eq!(item.label, "Language model (Ollama)");
        assert_eq!(
            item.fix.as_deref(),
            Some("Install Ollama and run: ollama pull <the model chosen in Settings > Models>")
        );
        assert_eq!(
            speakable_model_name("hf.co/org/model:Q4_K_M"),
            Some("hf.co/org/model:Q4_K_M")
        );
        assert_eq!(speakable_model_name(&"a".repeat(101)), None);
    }

    #[test]
    fn whisper_fallback_names_the_missing_part() {
        let programs_only = TempRuntime::new("whisper-programs");
        programs_only
            .add("whisper/Release/whisper-cli.exe")
            .add("whisper/Release/whisper-server.exe");
        let model_only = TempRuntime::new("whisper-model");
        model_only.add("models/ggml-tiny.en-q5_1.bin");
        let cli_only = TempRuntime::new("whisper-cli-only");
        cli_only
            .add("whisper/Release/whisper-cli.exe")
            .add("models/ggml-tiny.en-q5_1.bin");

        assert_eq!(
            whisper_fallback_item(&[programs_only.path()])
                .fix
                .as_deref(),
            Some("Run scripts\\Get-AsrModel.ps1 -Model tiny.en")
        );
        assert_eq!(
            whisper_fallback_item(&[model_only.path()]).fix.as_deref(),
            Some("Reinstall LocalFlow, or run scripts\\Get-WhisperRuntime.ps1")
        );
        assert!(!whisper_fallback_item(&[cli_only.path()]).ready);
    }

    #[test]
    fn fish_needs_engine_model_and_tokenizer_together() {
        let engine = TempRuntime::new("fish-engine");
        engine.add("fish/s2.exe");
        let model = TempRuntime::new("fish-model");
        model
            .add("models/fish/s2-pro-q4_k_m.gguf")
            .add("models/fish/tokenizer.json");
        let no_tokenizer = TempRuntime::new("fish-no-tokenizer");
        no_tokenizer
            .add("fish/s2.exe")
            .add("models/fish/s2-pro-q4_k_m.gguf");

        assert_eq!(
            fish_item(&[engine.path()]).fix.as_deref(),
            Some("Optional: run scripts\\Get-FishModel.ps1 (non-commercial use only)")
        );
        assert_eq!(
            fish_item(&[model.path()]).fix.as_deref(),
            Some("Optional: run scripts\\Build-FishS2.ps1")
        );
        let split = fish_item(&[engine.path(), model.path()]);
        assert!(!split.ready);
        assert!(split.fix.unwrap().contains("same -Destination"));
        assert!(!fish_item(&[no_tokenizer.path()]).ready);
        assert!(!fish_item(&[engine.path()]).required);
    }

    #[test]
    fn no_label_or_fix_contains_a_path() {
        let runtime = TempRuntime::new("private-user-name");
        let configured = runtime.path().join("missing-model.bin");
        let items = checklist(
            &[runtime.path()],
            configured.to_str().unwrap(),
            "",
            &OllamaProbe::NotAnswering,
        );

        for item in &items {
            for text in [Some(&item.label), item.fix.as_ref()].into_iter().flatten() {
                assert!(!text.contains("private-user-name"), "{text}");
                assert!(!text.contains(":\\"), "{text}");
                assert!(
                    !text.contains(&*env::temp_dir().to_string_lossy()),
                    "{text}"
                );
            }
        }
    }

    #[test]
    fn items_serialize_with_camel_case_fields() {
        let value = serde_json::to_value(ReadinessItem::new(
            "smart-turn",
            "End-of-turn model (Smart Turn)",
            false,
            Some("Run scripts\\Get-SmartTurnModel.ps1".to_string()),
        ))
        .unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "id": "smart-turn",
                "label": "End-of-turn model (Smart Turn)",
                "ready": false,
                "required": false,
                "fix": "Run scripts\\Get-SmartTurnModel.ps1"
            })
        );
        let ready = serde_json::to_value(ReadinessItem::new("x", "X", true, None)).unwrap();
        assert_eq!(ready["fix"], serde_json::Value::Null);
        assert_eq!(ready["ready"], true);
    }
}
