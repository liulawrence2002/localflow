//! One-shot whisper.cpp CLI provider.
//!
//! Spawns `whisper-cli.exe` for each utterance and reads its JSON output. It pays process
//! start and model load every time, so it is the compatibility fallback and the low-resource
//! option (nothing stays in memory between dictations), not the default engine.

use std::{
    fs,
    path::PathBuf,
    process::Stdio,
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

use super::{
    provider::{
        transcription_deadline, AsrError, SpeechToTextProvider, Transcription, TranscriptionRequest,
    },
    sidecar::{hidden_sidecar_command, peak_memory_bytes},
    whisper_text::parse_whisper_json,
    WhisperDecodeOptions,
};
use crate::audio::wav::write_wav_16k_mono;

pub const WHISPER_CLI_PROVIDER_ID: &str = "whisper-cli";

#[derive(Debug, Clone)]
pub struct WhisperCliConfig {
    pub cli_path: PathBuf,
    pub model_path: PathBuf,
    pub decode: WhisperDecodeOptions,
    /// Directory for the short-lived WAV and JSON files. They are deleted after every run.
    pub temp_dir: PathBuf,
}

pub struct WhisperCliProvider {
    config: WhisperCliConfig,
    sequence: AtomicU64,
    last_peak_memory: AtomicU64,
}

impl WhisperCliProvider {
    pub fn new(config: WhisperCliConfig) -> Self {
        Self {
            config,
            sequence: AtomicU64::new(0),
            last_peak_memory: AtomicU64::new(0),
        }
    }

    /// Peak resident memory of the most recent CLI run, in bytes, when measurable.
    pub fn last_peak_memory_bytes(&self) -> Option<u64> {
        match self.last_peak_memory.load(Ordering::Relaxed) {
            0 => None,
            bytes => Some(bytes),
        }
    }

    fn check_files(&self) -> Result<(), AsrError> {
        if !self.config.cli_path.is_file() {
            // File names only: full paths can contain the Windows user name and these
            // messages reach the log and the health file.
            return Err(AsrError::Unavailable(format!(
                "Missing whisper.cpp executable ({})",
                super::model_label(&self.config.cli_path)
            )));
        }
        if !self.config.model_path.is_file() {
            return Err(AsrError::Unavailable(format!(
                "Missing Whisper model ({})",
                super::model_label(&self.config.model_path)
            )));
        }
        Ok(())
    }

    fn run(&self, request: &TranscriptionRequest<'_>) -> Result<String, AsrError> {
        self.check_files()?;
        fs::create_dir_all(&self.config.temp_dir).map_err(|error| {
            AsrError::Unavailable(format!(
                "Could not create temporary audio directory: {error}"
            ))
        })?;

        // Unique per process and per call so concurrent sessions can never share a file.
        let stem = format!(
            "dictation-{}-{}",
            std::process::id(),
            self.sequence.fetch_add(1, Ordering::Relaxed)
        );
        let wav_path = self.config.temp_dir.join(format!("{stem}.wav"));
        let output_base = self.config.temp_dir.join(&stem);
        let json_path = output_base.with_extension("json");

        let result = write_wav_16k_mono(&wav_path, request.samples)
            .map_err(AsrError::Failed)
            .and_then(|()| self.spawn_and_wait(request, &wav_path, &output_base))
            .and_then(|()| {
                fs::read_to_string(&json_path).map_err(|error| {
                    AsrError::Failed(format!("Could not read whisper.cpp JSON output: {error}"))
                })
            })
            .and_then(|payload| parse_whisper_json(&payload).map_err(AsrError::Failed));

        // Audio and transcript files never outlive the call, on success or failure.
        let _ = fs::remove_file(&wav_path);
        let _ = fs::remove_file(&json_path);

        result
    }

    fn spawn_and_wait(
        &self,
        request: &TranscriptionRequest<'_>,
        wav_path: &std::path::Path,
        output_base: &std::path::Path,
    ) -> Result<(), AsrError> {
        let mut command = hidden_sidecar_command(&self.config.cli_path);
        if let Some(runtime_dir) = self.config.cli_path.parent() {
            command.current_dir(runtime_dir);
        }
        command
            .stdin(Stdio::null())
            // The transcript is printed to stdout; it is discarded so it can never reach a log.
            .stdout(Stdio::null())
            // stderr can name the temporary audio path (which includes the user name), so it
            // is discarded too; the exit status is enough to report a failure.
            .stderr(Stdio::null())
            .arg("-m")
            .arg(&self.config.model_path)
            .arg("-f")
            .arg(wav_path)
            .arg("-oj")
            .arg("-of")
            .arg(output_base)
            .arg("-np")
            .arg("-nt")
            .arg("-l")
            .arg(request.language)
            .arg("-t")
            .arg(self.config.decode.threads.to_string())
            .arg("-bs")
            .arg(self.config.decode.beam_size.to_string())
            .arg("-bo")
            .arg(self.config.decode.beam_size.to_string());

        if let Some(frames) = self
            .config
            .decode
            .audio_context
            .frames_for(request.audio_duration())
        {
            command.arg("-ac").arg(frames.to_string());
        }

        // Passed as a single argv element, so dictionary text cannot inject extra flags.
        if let Some(prompt) = request.initial_prompt {
            command.arg("--prompt").arg(prompt);
        }

        let mut child = command.spawn().map_err(|error| {
            AsrError::Unavailable(format!("Could not start whisper.cpp: {error}"))
        })?;

        let deadline = transcription_deadline(request.audio_duration());
        let started_at = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started_at.elapsed() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(AsrError::Timeout {
                            provider: WHISPER_CLI_PROVIDER_ID,
                            waited: started_at.elapsed(),
                        });
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(AsrError::Failed(format!(
                        "Could not wait for whisper.cpp: {error}"
                    )));
                }
            }
        };

        if let Some(bytes) = peak_memory_bytes(&child) {
            self.last_peak_memory.store(bytes, Ordering::Relaxed);
        }

        if status.success() {
            return Ok(());
        }

        Err(AsrError::Failed(format!(
            "whisper.cpp exited with {status}"
        )))
    }
}

impl SpeechToTextProvider for WhisperCliProvider {
    fn id(&self) -> &'static str {
        WHISPER_CLI_PROVIDER_ID
    }

    fn describe(&self) -> String {
        format!(
            "whisper.cpp CLI, one process per utterance ({})",
            super::model_label(&self.config.model_path)
        )
    }

    fn prepare(&self) -> Result<(), AsrError> {
        self.check_files()
    }

    fn transcribe(&self, request: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
        let text = self.run(request)?;
        Ok(Transcription {
            text,
            provider: WHISPER_CLI_PROVIDER_ID,
            fallback_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(cli: &str, model: &str) -> WhisperCliProvider {
        WhisperCliProvider::new(WhisperCliConfig {
            cli_path: PathBuf::from(cli),
            model_path: PathBuf::from(model),
            decode: WhisperDecodeOptions::default(),
            temp_dir: std::env::temp_dir().join("localflow-cli-test"),
        })
    }

    #[test]
    fn reports_a_missing_executable_as_unavailable() {
        let provider = provider(
            "Z:/definitely/missing/whisper-cli.exe",
            "Z:/missing/model.bin",
        );

        let error = provider.prepare().unwrap_err();

        assert!(matches!(error, AsrError::Unavailable(_)));
        assert_eq!(
            error.to_string(),
            "Missing whisper.cpp executable (whisper-cli.exe)"
        );
    }

    #[test]
    fn transcribe_fails_closed_without_leaving_temp_files_when_unavailable() {
        let provider = provider(
            "Z:/definitely/missing/whisper-cli.exe",
            "Z:/missing/model.bin",
        );
        let request = TranscriptionRequest {
            samples: &[0.0; 1_600],
            language: "en",
            initial_prompt: None,
        };

        assert!(matches!(
            provider.transcribe(&request),
            Err(AsrError::Unavailable(_))
        ));
        assert_eq!(provider.last_peak_memory_bytes(), None);
    }

    /// Runs a real process that exits without producing output, then checks that the audio
    /// file written for it is gone and that the error carries no path.
    #[cfg(windows)]
    #[test]
    fn a_failed_run_removes_its_temp_files_and_reports_no_paths() {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:/Windows".to_string());
        // `where.exe` rejects whisper's flags and exits non-zero straight away.
        let stand_in = PathBuf::from(system_root)
            .join("System32")
            .join("where.exe");
        let temp_dir =
            std::env::temp_dir().join(format!("localflow-cli-cleanup-test-{}", std::process::id()));
        let provider = WhisperCliProvider::new(WhisperCliConfig {
            cli_path: stand_in.clone(),
            model_path: stand_in,
            decode: WhisperDecodeOptions::default(),
            temp_dir: temp_dir.clone(),
        });

        let error = provider
            .transcribe(&TranscriptionRequest {
                samples: &[0.0; 1_600],
                language: "en",
                initial_prompt: Some("Glossary: Tauri."),
            })
            .unwrap_err();

        let leftovers: Vec<_> = fs::read_dir(&temp_dir)
            .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        let _ = fs::remove_dir_all(&temp_dir);

        assert!(matches!(error, AsrError::Failed(_)), "{error:?}");
        assert!(leftovers.is_empty(), "left behind: {leftovers:?}");
        let message = error.to_string();
        assert!(!message.contains("localflow-cli-cleanup-test"), "{message}");
        assert!(
            !message.contains(":\\") && !message.contains(":/"),
            "{message}"
        );
    }
}
