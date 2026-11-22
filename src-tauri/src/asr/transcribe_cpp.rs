//! In-process speech-to-text through transcribe.cpp (ggml).
//!
//! This is the engine for NVIDIA Parakeet GGUF models. The model is loaded once and kept in
//! memory, so each dictation pays only for inference. Compared with the small Whisper models
//! it is far more accurate and writes its own punctuation and capitalisation.
//!
//! The model and session live inside the LocalFlow process. A run that passes its deadline is
//! cancelled cooperatively through the library's abort callback rather than by killing a
//! process. The library does no networking and its native logging is disabled, so neither
//! audio nor text leaves this module except through the returned transcript.

use std::{
    path::PathBuf,
    sync::{mpsc, Mutex, Once},
    thread,
    time::Instant,
};

use transcribe_cpp::{CancelToken, ErrorKind, Feature, Model, RunOptions, Session};

use super::provider::{
    transcription_deadline, AsrError, SpeechToTextProvider, Transcription, TranscriptionRequest,
};

pub const TRANSCRIBE_CPP_PROVIDER_ID: &str = "transcribe-cpp";

/// Whether `model_path` is a GGUF model for this engine. The whisper.cpp sidecars use the
/// older `ggml-*.bin` container, so the extension is enough to tell them apart.
pub fn is_transcribe_cpp_model(model_path: &std::path::Path) -> bool {
    model_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
}

#[derive(Debug, Clone)]
pub struct TranscribeCppConfig {
    pub model_path: PathBuf,
}

struct Loaded {
    session: Session,
    /// Compute backend the library chose, for diagnostics (for example `cpu` or `vulkan`).
    backend: String,
    supports_vocabulary: bool,
}

pub struct TranscribeCppProvider {
    config: TranscribeCppConfig,
    loaded: Mutex<Option<Loaded>>,
}

static SILENCE_NATIVE_LOGGING: Once = Once::new();

impl TranscribeCppProvider {
    pub fn new(config: TranscribeCppConfig) -> Self {
        Self {
            config,
            loaded: Mutex::new(None),
        }
    }

    /// The compute backend in use, once the model is loaded.
    pub fn backend(&self) -> Option<String> {
        self.lock().as_ref().map(|loaded| loaded.backend.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Loaded>> {
        self.loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn load(&self) -> Result<Loaded, AsrError> {
        if !self.config.model_path.is_file() {
            return Err(AsrError::Unavailable(format!(
                "Missing speech model ({})",
                super::model_label(&self.config.model_path)
            )));
        }

        // The native library prints diagnostics to stderr by default. Nothing it prints is
        // needed, and silence is the safe default for a dictation app.
        SILENCE_NATIVE_LOGGING.call_once(transcribe_cpp::disable_logging);

        let started_at = Instant::now();
        let model = Model::load(&self.config.model_path).map_err(|error| {
            AsrError::Unavailable(format!(
                "Could not load speech model ({}): {:?} error",
                super::model_label(&self.config.model_path),
                error.kind()
            ))
        })?;
        let session = model.session().map_err(|error| {
            AsrError::Unavailable(format!(
                "Could not start a speech session: {:?} error",
                error.kind()
            ))
        })?;
        let backend = model.backend();

        tracing::info!(
            backend = %backend,
            load_ms = started_at.elapsed().as_millis() as u64,
            "transcribe.cpp model loaded"
        );

        Ok(Loaded {
            session,
            backend,
            supports_vocabulary: model.supports(Feature::Vocabulary),
        })
    }
}

/// Dictionary terms recovered from the vocabulary prompt (`Glossary: a, b, c.`), so engines
/// with a native vocabulary feature receive a term list instead of a sentence.
fn vocabulary_terms(initial_prompt: Option<&str>) -> Vec<String> {
    let Some(prompt) = initial_prompt else {
        return Vec::new();
    };
    prompt
        .trim()
        .trim_start_matches("Glossary:")
        .trim_end_matches('.')
        .split(',')
        .map(|term| term.trim().to_string())
        .filter(|term| !term.is_empty())
        .collect()
}

impl SpeechToTextProvider for TranscribeCppProvider {
    fn id(&self) -> &'static str {
        TRANSCRIBE_CPP_PROVIDER_ID
    }

    fn describe(&self) -> String {
        format!(
            "transcribe.cpp in-process, model kept loaded ({})",
            super::model_label(&self.config.model_path)
        )
    }

    fn prepare(&self) -> Result<(), AsrError> {
        let mut guard = self.lock();
        if guard.is_none() {
            *guard = Some(self.load()?);
        }
        Ok(())
    }

    fn transcribe(&self, request: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
        let mut guard = self.lock();
        if guard.is_none() {
            *guard = Some(self.load()?);
        }
        let loaded = guard.as_mut().expect("model was just loaded");

        let options = RunOptions {
            language: Some(request.language.to_string()),
            vocabulary: if loaded.supports_vocabulary {
                vocabulary_terms(request.initial_prompt)
            } else {
                Vec::new()
            },
            ..Default::default()
        };

        // Deadline: a watchdog thread cancels the run through the library's abort callback
        // unless the run finishes first and signals it.
        let cancel = CancelToken::new();
        loaded.session.set_cancel_token(&cancel);
        let deadline = transcription_deadline(request.audio_duration());
        let started_at = Instant::now();
        let (finished_tx, finished_rx) = mpsc::channel::<()>();

        let result = thread::scope(|scope| {
            let watchdog_cancel = cancel.clone();
            scope.spawn(move || {
                if finished_rx.recv_timeout(deadline).is_err() {
                    watchdog_cancel.cancel();
                }
            });
            let result = loaded.session.run(request.samples, &options);
            let _ = finished_tx.send(());
            result
        });
        loaded.session.clear_cancel_token();

        match result {
            Ok(transcript) => Ok(Transcription {
                text: transcript
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                provider: TRANSCRIBE_CPP_PROVIDER_ID,
                fallback_reason: None,
            }),
            Err(error) if error.kind() == ErrorKind::Aborted => Err(AsrError::Timeout {
                provider: TRANSCRIBE_CPP_PROVIDER_ID,
                waited: started_at.elapsed(),
            }),
            // Audio too short to decode is not a failure; it simply contains no words.
            Err(error) if error.kind() == ErrorKind::InputTooShort => Ok(Transcription {
                text: String::new(),
                provider: TRANSCRIBE_CPP_PROVIDER_ID,
                fallback_reason: None,
            }),
            Err(error) => {
                // A failed run can leave the session in an unknown state; reload next time.
                // Only the error kind is reported: the full message can carry partial text.
                let kind = error.kind();
                *guard = None;
                Err(AsrError::Failed(format!(
                    "transcribe.cpp run failed: {kind:?} error"
                )))
            }
        }
    }

    fn shutdown(&self) {
        // Dropping the session and model frees the weights.
        *self.lock() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn gguf_files_select_this_engine_and_ggml_bins_do_not() {
        assert!(is_transcribe_cpp_model(Path::new(
            "models/parakeet-tdt-0.6b-v3-Q5_K_M.gguf"
        )));
        assert!(is_transcribe_cpp_model(Path::new("MODEL.GGUF")));
        assert!(!is_transcribe_cpp_model(Path::new(
            "models/ggml-tiny.en-q5_1.bin"
        )));
        assert!(!is_transcribe_cpp_model(Path::new("models/gguf")));
    }

    #[test]
    fn vocabulary_terms_are_recovered_from_the_prompt() {
        assert_eq!(
            vocabulary_terms(Some("Glossary: Tauri, cpal, LocalFlow.")),
            vec!["Tauri", "cpal", "LocalFlow"]
        );
        assert!(vocabulary_terms(None).is_empty());
        assert!(vocabulary_terms(Some("Glossary: .")).is_empty());
    }

    #[test]
    fn a_missing_model_is_unavailable_and_names_no_path() {
        let provider = TranscribeCppProvider::new(TranscribeCppConfig {
            model_path: PathBuf::from("Z:/someone/models/parakeet.gguf"),
        });

        let error = provider.prepare().unwrap_err();

        assert_eq!(error.to_string(), "Missing speech model (parakeet.gguf)");
        assert_eq!(provider.backend(), None);
    }

    /// End-to-end check with the real model. Ignored by default; run with
    /// `cargo test --release --lib real_parakeet -- --ignored`.
    #[test]
    #[ignore = "needs the Parakeet GGUF model and the bench corpus in .localflow-runtime"]
    fn real_parakeet_transcribes_and_reloads_after_shutdown() {
        let runtime = std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(".localflow-runtime"),
        )
        .unwrap();
        let audio = crate::audio::wav::read_wav_mono(
            &runtime.join("bench-corpus").join("wav").join("jfk.wav"),
        )
        .expect("bench corpus jfk.wav");
        let samples = crate::audio::resample::resample_to_16khz(&audio.samples, audio.sample_rate);
        let provider = TranscribeCppProvider::new(TranscribeCppConfig {
            model_path: runtime
                .join("models")
                .join("parakeet-tdt-0.6b-v3-Q5_K_M.gguf"),
        });
        let request = TranscriptionRequest {
            samples: &samples,
            language: "en",
            initial_prompt: None,
        };

        let first = provider.transcribe(&request).unwrap();
        assert!(first.text.to_lowercase().contains("fellow americans"));
        assert!(provider.backend().is_some());

        provider.shutdown();
        assert_eq!(provider.backend(), None);
        assert_eq!(provider.transcribe(&request).unwrap().text, first.text);
    }
}
