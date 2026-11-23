//! The conversation voice: Gradium Phonon 31m (English), run in-process on the CPU.
//!
//! It generates faster than real time without a GPU and hands audio out in small pieces as
//! it goes, which is what lets a reply start within a fraction of a second. The model files
//! are read from a local folder (`scripts/Get-PhononModel.ps1` installs them). The library is
//! built without its download feature, so nothing here can reach the network.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use ptts::{
    checkpoint::{Checkpoint, ResolveOptions},
    preprocess::Lang,
    synth::{DeviceKind, Quant, Synth},
};

use super::{
    provider::TtsError,
    stream::{CancelFlag, StreamingTextToSpeech},
};

/// Folder name under the runtime `models` directory.
pub const PHONON_MODEL_DIR: &str = "phonon";
/// Files that must be present for the voice to load.
pub const PHONON_REQUIRED_FILES: &[&str] = &[
    "config.json",
    "tokenizer.json",
    "default-voice.safetensors",
    "model.q8.gguf",
];
/// Output rate of the model. Reported before the model is loaded; checked when it loads.
const PHONON_SAMPLE_RATE: u32 = 24_000;
/// Most threads the voice may use. Measured on a 22-thread Core Ultra 7 165H: with the
/// library's default of one thread per logical core it ran at 1.1 times real time, because
/// the threads fight over a small model; with four it ran at 4.4 times real time, and more
/// than eight was slower again. Four also leaves the rest of the machine for speech
/// recognition and the language model.
const PHONON_MAX_THREADS: usize = 4;

/// Threads to give the voice on a machine with `logical_cores`.
fn voice_threads(logical_cores: usize) -> usize {
    (logical_cores / 2).clamp(1, PHONON_MAX_THREADS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhononConfig {
    /// Folder holding the files in [`PHONON_REQUIRED_FILES`].
    pub model_dir: PathBuf,
}

pub struct PhononProvider {
    config: PhononConfig,
    /// Loaded on first use and kept. `Synth` is `Send + Sync`; requests share it.
    loaded: Mutex<Option<Arc<Synth>>>,
}

impl PhononProvider {
    pub fn new(config: PhononConfig) -> Self {
        Self {
            config,
            loaded: Mutex::new(None),
        }
    }

    /// The first required file that is missing from `model_dir`, if any.
    pub fn missing_file(config: &PhononConfig) -> Option<&'static str> {
        PHONON_REQUIRED_FILES
            .iter()
            .copied()
            .find(|name| !config.model_dir.join(name).is_file())
    }

    fn synth(&self) -> Result<Arc<Synth>, TtsError> {
        let mut slot = self
            .loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(synth) = slot.as_ref() {
            return Ok(synth.clone());
        }

        if let Some(missing) = Self::missing_file(&self.config) {
            return Err(TtsError::Unavailable(format!(
                "The conversation voice is not installed ({missing} is missing). Run scripts\\Get-PhononModel.ps1."
            )));
        }

        // The thread count is a process-wide setting of the tensor backend, which only this
        // voice uses.
        let cores = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(PHONON_MAX_THREADS);
        xn::set_num_threads(voice_threads(cores));

        // Errors from the library describe files and tensors, never the text being spoken,
        // but may contain local paths, so only their kind is reported.
        let checkpoint = Checkpoint::resolve(
            &self.config.model_dir,
            ResolveOptions {
                quant: Quant::Q80,
                weights: None,
            },
        )
        .map_err(|error| load_error("read", &error))?;
        let mut synth = checkpoint
            .builder(Lang::En)
            .device(DeviceKind::Cpu)
            .build()
            .map_err(|error| load_error("load", &error))?;
        checkpoint.register_voices(&mut synth);

        if synth.sample_rate() != PHONON_SAMPLE_RATE {
            return Err(TtsError::Unavailable(format!(
                "The conversation voice model produces {} Hz audio; {PHONON_SAMPLE_RATE} Hz was expected.",
                synth.sample_rate()
            )));
        }

        let synth = Arc::new(synth);
        *slot = Some(synth.clone());
        Ok(synth)
    }
}

fn load_error(stage: &str, error: &ptts::Error) -> TtsError {
    tracing::warn!(
        stage,
        kind = error_kind(error),
        "conversation voice could not start"
    );
    TtsError::Unavailable(format!(
        "The conversation voice model could not be {stage} ({}).",
        error_kind(error)
    ))
}

/// The class of a library error, without its message.
fn error_kind(error: &ptts::Error) -> &'static str {
    match error {
        ptts::Error::NotFound(_) => "file not found",
        ptts::Error::Io(_) => "file error",
        _ => "model error",
    }
}

impl StreamingTextToSpeech for PhononProvider {
    fn id(&self) -> &'static str {
        "phonon"
    }

    fn sample_rate(&self) -> u32 {
        PHONON_SAMPLE_RATE
    }

    fn prepare(&self) -> Result<(), TtsError> {
        self.synth().map(|_| ())
    }

    fn synthesize_stream(
        &self,
        text: &str,
        cancel: &CancelFlag,
        on_audio: &mut dyn FnMut(&[f32]),
    ) -> Result<(), TtsError> {
        if cancel.is_cancelled() {
            return Ok(());
        }
        let synth = self.synth()?;
        let stream = synth.stream(text).map_err(|error| {
            TtsError::Failed(format!(
                "The conversation voice could not start speaking ({}).",
                error_kind(&error)
            ))
        })?;

        for piece in stream {
            // Leaving the loop drops the stream, which stops and joins its worker threads.
            if cancel.is_cancelled() {
                break;
            }
            let samples = piece.map_err(|error| {
                TtsError::Failed(format!(
                    "The conversation voice stopped unexpectedly ({}).",
                    error_kind(&error)
                ))
            })?;
            on_audio(&samples);
        }
        Ok(())
    }

    fn shutdown(&self) {
        self.loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "localflow-phonon-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_model_is_reported_with_the_script_that_installs_it() {
        let dir = temp_dir("missing");
        std::fs::write(dir.join("config.json"), "{}").unwrap();
        let provider = PhononProvider::new(PhononConfig {
            model_dir: dir.clone(),
        });

        let error = provider.prepare().unwrap_err();

        let message = error.to_string();
        assert!(matches!(error, TtsError::Unavailable(_)));
        assert!(message.contains("tokenizer.json"));
        assert!(message.contains("Get-PhononModel.ps1"));
        // The message names the missing file, not where the folder is.
        assert!(!message.contains(dir.to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_voice_takes_a_few_threads_however_many_cores_there_are() {
        assert_eq!(voice_threads(1), 1);
        assert_eq!(voice_threads(4), 2);
        assert_eq!(voice_threads(8), 4);
        assert_eq!(voice_threads(22), 4);
        assert_eq!(voice_threads(64), 4);
    }

    #[test]
    fn a_cancelled_request_does_no_work() {
        // No model exists at this path: reaching the loader would fail the request.
        let provider = PhononProvider::new(PhononConfig {
            model_dir: PathBuf::from("does-not-exist"),
        });
        let cancel = CancelFlag::default();
        cancel.cancel();

        let result = provider.synthesize_stream("hello", &cancel, &mut |_| {
            panic!("no audio expected after cancellation")
        });

        assert_eq!(result, Ok(()));
    }

    /// Needs the real model: `scripts\Get-PhononModel.ps1`, then
    /// `cargo test --release --lib real_phonon -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the Phonon model in .localflow-runtime/models/phonon"]
    fn real_phonon_speaks_and_stops_when_cancelled() {
        let model_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".localflow-runtime")
            .join("models")
            .join(PHONON_MODEL_DIR);
        let provider = PhononProvider::new(PhononConfig { model_dir });
        provider.prepare().expect("model should load");

        let mut samples = 0usize;
        provider
            .synthesize_stream(
                "Hello from LocalFlow.",
                &CancelFlag::default(),
                &mut |piece| {
                    samples += piece.len();
                },
            )
            .expect("speech should be produced");
        // At least half a second of audio for a three-word sentence.
        assert!(samples > PHONON_SAMPLE_RATE as usize / 2);

        let cancel = CancelFlag::default();
        let mut pieces = 0usize;
        provider
            .synthesize_stream(
                "This sentence is long enough that it would produce many pieces of audio if it were allowed to run to the end.",
                &cancel,
                &mut |_| {
                    pieces += 1;
                    cancel.cancel();
                },
            )
            .expect("a cancelled request is not an error");
        assert_eq!(pieces, 1);
    }
}
