//! Chooses, builds and caches the speech-to-text engine for the current settings.

use std::{
    env,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
};

use tauri::{AppHandle, Manager};

use super::{
    model_label,
    provider::{FallbackProvider, SpeechToTextProvider},
    transcribe_cpp::{is_transcribe_cpp_model, TranscribeCppConfig, TranscribeCppProvider},
    whisper_cli::{WhisperCliConfig, WhisperCliProvider},
    whisper_server::{WhisperServerConfig, WhisperServerProvider},
    WhisperDecodeOptions,
};
use crate::runtime_paths::{
    find_file, preferred_model_candidates, runtime_candidates, speech_model_path, whisper_cli_path,
    FALLBACK_WHISPER_MODELS,
};

/// The engine currently loaded, shared by every feature that transcribes speech.
#[derive(Default)]
pub struct AsrEngines {
    slot: Mutex<Option<AsrEngineSlot>>,
}

/// Which engine chain to run for one configuration. The engine is rebuilt (and the old one
/// stopped) only when the plan changes, for example after the user picks another model or
/// toggles low-resource mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AsrEnginePlan {
    /// A GGUF model (Parakeet) kept loaded in-process through transcribe.cpp, with the
    /// one-shot Whisper CLI as a local fallback when a Whisper model is installed.
    InProcess {
        model_path: PathBuf,
        whisper_fallback: Option<WhisperFiles>,
    },
    /// A Whisper model kept loaded in the persistent whisper.cpp server, falling back to the
    /// one-shot CLI with the same model.
    WhisperServer {
        server_path: PathBuf,
        whisper: WhisperFiles,
    },
    /// One whisper.cpp process per utterance: nothing stays in memory between dictations.
    WhisperCli { whisper: WhisperFiles },
}

/// The whisper.cpp CLI and a Whisper model for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhisperFiles {
    pub(crate) cli_path: PathBuf,
    pub(crate) model_path: PathBuf,
}

impl AsrEnginePlan {
    /// The model that will normally produce the transcript.
    pub(crate) fn model_path(&self) -> &Path {
        match self {
            Self::InProcess { model_path, .. } => model_path,
            Self::WhisperServer { whisper, .. } | Self::WhisperCli { whisper } => {
                &whisper.model_path
            }
        }
    }
}

pub(crate) struct AsrEngineSlot {
    pub(crate) plan: AsrEnginePlan,
    pub(crate) provider: Arc<dyn SpeechToTextProvider>,
}

/// The engine to use for one dictation plus the facts diagnostics need about it.
pub(crate) struct ResolvedAsr {
    pub(crate) provider: Arc<dyn SpeechToTextProvider>,
    pub(crate) model_path: PathBuf,
}

/// Decide which engine chain to run. Pure, so every rule is unit-tested.
///
/// - A GGUF model runs in-process and stays loaded, unless low-resource mode is on and a
///   Whisper fallback exists, in which case the one-shot CLI is used so nothing stays loaded.
/// - A whisper.cpp model runs in the persistent server when it is available and low-resource
///   mode is off, otherwise in the one-shot CLI.
pub(crate) fn plan_asr_engine(
    model_path: PathBuf,
    low_resource_mode: bool,
    whisper_cli: Option<PathBuf>,
    whisper_server: Option<PathBuf>,
    whisper_fallback_model: Option<PathBuf>,
) -> Result<AsrEnginePlan, String> {
    if is_transcribe_cpp_model(&model_path) {
        let whisper_fallback = match (whisper_cli, whisper_fallback_model) {
            (Some(cli_path), Some(model_path)) => Some(WhisperFiles {
                cli_path,
                model_path,
            }),
            _ => None,
        };
        return Ok(match (low_resource_mode, whisper_fallback) {
            (true, Some(whisper)) => AsrEnginePlan::WhisperCli { whisper },
            (_, whisper_fallback) => AsrEnginePlan::InProcess {
                model_path,
                whisper_fallback,
            },
        });
    }

    let cli_path = whisper_cli.ok_or_else(|| {
        "Missing whisper.cpp executable (whisper-cli.exe) for the configured Whisper model."
            .to_string()
    })?;
    let whisper = WhisperFiles {
        cli_path,
        model_path,
    };
    Ok(match whisper_server {
        Some(server_path) if !low_resource_mode => AsrEnginePlan::WhisperServer {
            server_path,
            whisper,
        },
        _ => AsrEnginePlan::WhisperCli { whisper },
    })
}

/// Resolve (building or reusing) the ASR engine for the current settings.
pub(crate) fn resolve_asr(
    app: &AppHandle,
    settings: &crate::app_state::SettingsSnapshot,
) -> Result<ResolvedAsr, String> {
    let model_path = speech_model_path(app, &settings.models.whisper_model_path)?;
    let whisper_cli = whisper_cli_path(app).ok();
    let whisper_server = whisper_cli
        .as_ref()
        .map(|cli| cli.with_file_name("whisper-server.exe"))
        .filter(|server| server.is_file());
    let whisper_fallback_model = find_file(
        "Whisper model",
        preferred_model_candidates(&runtime_candidates(app), FALLBACK_WHISPER_MODELS),
    )
    .ok();
    let plan = plan_asr_engine(
        model_path,
        settings.models.low_resource_mode,
        whisper_cli,
        whisper_server,
        whisper_fallback_model,
    )?;
    let model_path = plan.model_path().to_path_buf();

    let engines = app.state::<AsrEngines>();
    let mut slot = engines
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    if let Some(existing) = slot.as_ref() {
        if existing.plan == plan {
            return Ok(ResolvedAsr {
                provider: existing.provider.clone(),
                model_path,
            });
        }
    }
    if let Some(previous) = slot.take() {
        previous.provider.shutdown();
    }

    let provider = build_asr_provider(&plan);
    tracing::info!(engine = %provider.describe(), "ASR engine selected");
    *slot = Some(AsrEngineSlot {
        plan,
        provider: provider.clone(),
    });

    Ok(ResolvedAsr {
        provider,
        model_path,
    })
}

/// Build the engine chain for `plan`. Every engine here is local; there is no remote
/// fallback.
pub(crate) fn build_asr_provider(plan: &AsrEnginePlan) -> Arc<dyn SpeechToTextProvider> {
    let decode = WhisperDecodeOptions::default();
    let whisper_cli = |whisper: &WhisperFiles| {
        WhisperCliProvider::new(WhisperCliConfig {
            cli_path: whisper.cli_path.clone(),
            model_path: whisper.model_path.clone(),
            decode,
            temp_dir: env::temp_dir().join("localflow"),
        })
    };

    match plan {
        AsrEnginePlan::InProcess {
            model_path,
            whisper_fallback,
        } => {
            let in_process = TranscribeCppProvider::new(TranscribeCppConfig {
                model_path: model_path.clone(),
            });
            match whisper_fallback {
                Some(whisper) => Arc::new(FallbackProvider::new(
                    Box::new(in_process),
                    Box::new(whisper_cli(whisper)),
                )),
                None => Arc::new(in_process),
            }
        }
        AsrEnginePlan::WhisperServer {
            server_path,
            whisper,
        } => match WhisperServerProvider::new(WhisperServerConfig::new(
            server_path.clone(),
            whisper.model_path.clone(),
            decode,
        )) {
            Ok(server) => Arc::new(FallbackProvider::new(
                Box::new(server),
                Box::new(whisper_cli(whisper)),
            )),
            Err(error) => {
                tracing::warn!(error = %error, "persistent ASR engine unavailable; using the CLI");
                Arc::new(whisper_cli(whisper))
            }
        },
        AsrEnginePlan::WhisperCli { whisper } => Arc::new(whisper_cli(whisper)),
    }
}

/// Start loading the ASR model while the user is still speaking, so it is warm by the time
/// the recording ends.
pub(crate) fn prepare_asr_in_background(app: AppHandle) {
    thread::spawn(move || {
        let settings = app
            .state::<crate::app_state::LocalFlowRuntime>()
            .current_settings();
        match resolve_asr(&app, &settings) {
            Ok(asr) => {
                if let Err(error) = asr.provider.prepare() {
                    tracing::warn!(error = %error, "ASR engine warm-up failed");
                }
            }
            Err(error) => tracing::warn!(error = %error, "ASR engine could not be resolved"),
        }
    });
}

/// Stop any ASR sidecar. Called when the app exits.
pub fn shutdown(app: &AppHandle) {
    let engines = app.state::<AsrEngines>();
    let slot = engines
        .slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take();
    if let Some(slot) = slot {
        slot.provider.shutdown();
    }
}

/// Language code to pass to whisper. English-only models (`*.en*`) always get `en`; otherwise
/// the user's setting is used, defaulting to English when it is empty.
pub(crate) fn asr_language(configured: &str, model_path: &Path) -> String {
    let english_only = model_label(model_path).contains(".en");
    let configured = configured.trim();

    if english_only || configured.is_empty() {
        "en".to_string()
    } else {
        configured.to_ascii_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_only_models_always_use_english() {
        let english = Path::new("models/ggml-base.en-q5_1.bin");
        let multilingual = Path::new("models/ggml-large-v3-turbo-q5_0.bin");

        assert_eq!(asr_language("de", english), "en");
        assert_eq!(asr_language("", multilingual), "en");
        assert_eq!(asr_language(" DE ", multilingual), "de");
        assert_eq!(asr_language("auto", multilingual), "auto");
    }

    fn whisper_files() -> (PathBuf, PathBuf, PathBuf) {
        (
            PathBuf::from("rt/whisper/Release/whisper-cli.exe"),
            PathBuf::from("rt/whisper/Release/whisper-server.exe"),
            PathBuf::from("rt/models/ggml-tiny.en-q5_1.bin"),
        )
    }

    #[test]
    fn a_gguf_model_runs_in_process_with_the_whisper_cli_as_fallback() {
        let (cli, server, tiny) = whisper_files();
        let parakeet = PathBuf::from("rt/models/parakeet-tdt-0.6b-v3-Q5_K_M.gguf");

        let plan = plan_asr_engine(
            parakeet.clone(),
            false,
            Some(cli.clone()),
            Some(server),
            Some(tiny.clone()),
        )
        .unwrap();

        assert_eq!(
            plan,
            AsrEnginePlan::InProcess {
                model_path: parakeet.clone(),
                whisper_fallback: Some(WhisperFiles {
                    cli_path: cli,
                    model_path: tiny,
                }),
            }
        );
        assert_eq!(plan.model_path(), parakeet.as_path());
    }

    #[test]
    fn a_gguf_model_needs_no_whisper_runtime_at_all() {
        let parakeet = PathBuf::from("rt/models/parakeet.gguf");

        let plan = plan_asr_engine(parakeet.clone(), false, None, None, None).unwrap();

        assert_eq!(
            plan,
            AsrEnginePlan::InProcess {
                model_path: parakeet,
                whisper_fallback: None,
            }
        );
    }

    #[test]
    fn low_resource_mode_keeps_nothing_loaded_when_a_whisper_fallback_exists() {
        let (cli, server, tiny) = whisper_files();
        let parakeet = PathBuf::from("rt/models/parakeet.gguf");

        let with_fallback = plan_asr_engine(
            parakeet.clone(),
            true,
            Some(cli.clone()),
            Some(server),
            Some(tiny.clone()),
        )
        .unwrap();
        assert_eq!(
            with_fallback,
            AsrEnginePlan::WhisperCli {
                whisper: WhisperFiles {
                    cli_path: cli,
                    model_path: tiny,
                },
            }
        );

        // Without a Whisper model the only engine is the in-process one.
        let without = plan_asr_engine(parakeet, true, None, None, None).unwrap();
        assert!(matches!(without, AsrEnginePlan::InProcess { .. }));
    }

    #[test]
    fn a_whisper_model_uses_the_server_unless_low_resource_or_missing() {
        let (cli, server, tiny) = whisper_files();
        let whisper = WhisperFiles {
            cli_path: cli.clone(),
            model_path: tiny.clone(),
        };

        assert_eq!(
            plan_asr_engine(
                tiny.clone(),
                false,
                Some(cli.clone()),
                Some(server.clone()),
                None
            )
            .unwrap(),
            AsrEnginePlan::WhisperServer {
                server_path: server.clone(),
                whisper: whisper.clone(),
            }
        );
        assert_eq!(
            plan_asr_engine(tiny.clone(), true, Some(cli.clone()), Some(server), None).unwrap(),
            AsrEnginePlan::WhisperCli {
                whisper: whisper.clone(),
            }
        );
        assert_eq!(
            plan_asr_engine(tiny.clone(), false, Some(cli), None, None).unwrap(),
            AsrEnginePlan::WhisperCli { whisper }
        );
        assert!(plan_asr_engine(tiny, false, None, None, None)
            .unwrap_err()
            .contains("whisper-cli.exe"));
    }
}
