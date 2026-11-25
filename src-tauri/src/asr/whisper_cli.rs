//! One-shot whisper.cpp CLI provider.
//!
//! Spawns `whisper-cli.exe` for each utterance and reads its JSON output. It pays process
//! start and model load every time, so it is the compatibility fallback and the low-resource
//! option (nothing stays in memory between dictations), not the default engine.
//!
//! The process runs in a kill-on-close job, so it never outlives LocalFlow. Its two temporary
//! files are created fresh (never opened if something already exists under the name), deleted
//! after every run, and swept at start-up if a crash left them behind
//! ([`sweep_stale_temp_files`]).

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Once,
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    provider::{
        transcription_deadline, AsrError, SpeechToTextProvider, Transcription, TranscriptionRequest,
    },
    sidecar::{hidden_sidecar_command, peak_memory_bytes},
    whisper_text::parse_whisper_json,
    WhisperDecodeOptions,
};
use crate::{
    audio::wav::encode_wav_16k_mono,
    platform::process::{is_process_running, ContainedChild},
};

pub const WHISPER_CLI_PROVIDER_ID: &str = "whisper-cli";

/// Name prefix of the CLI's temporary files: `dictation-<pid>-<n>.wav` and `.json`.
const TEMP_FILE_PREFIX: &str = "dictation-";
/// A temporary file this old is left over from a run that crashed or was killed; no run
/// lasts this long (the transcription deadline is at most three minutes).
const STALE_TEMP_FILE_AGE: Duration = Duration::from_secs(10 * 60);
/// How many file names a run tries before giving up.
const TEMP_NAME_ATTEMPTS: u32 = 32;

/// Where the CLI's short-lived WAV and JSON files go by default: `%TEMP%\localflow`.
pub fn default_temp_dir() -> PathBuf {
    std::env::temp_dir().join("localflow")
}

/// Delete WAV and JSON files left in [`default_temp_dir`] by a dictation that crashed or was
/// killed before it could clean up. Safe to call at any time: a file is removed only when the
/// process that wrote it has gone, or it is older than any run can last. Returns how many
/// files were removed. Nothing about the files (names, paths, contents) is logged.
pub fn sweep_stale_temp_files() -> usize {
    sweep_stale_temp_files_in(&default_temp_dir(), SystemTime::now())
}

pub(crate) fn sweep_stale_temp_files_in(dir: &Path, now: SystemTime) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let own_pid = std::process::id();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(owner) = name.to_str().and_then(temp_file_owner) else {
            continue;
        };
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok();
        if is_stale_temp_file(owner, modified, now, own_pid, is_process_running)
            && remove_file_with_retry(&entry.path())
        {
            removed += 1;
        }
    }
    if removed > 0 {
        tracing::info!(removed, "removed leftover temporary dictation files");
    }
    removed
}

/// The process id in the name of a temporary file this provider writes, or `None` for any
/// other file.
pub(crate) fn temp_file_owner(name: &str) -> Option<u32> {
    let stem = name
        .strip_suffix(".wav")
        .or_else(|| name.strip_suffix(".json"))?;
    let (pid, sequence) = stem.strip_prefix(TEMP_FILE_PREFIX)?.split_once('-')?;
    if sequence.is_empty() || !sequence.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    pid.parse().ok()
}

/// Whether a temporary file written by process `owner` may be deleted. Pure, so every rule
/// is unit-tested.
pub(crate) fn is_stale_temp_file(
    owner: u32,
    modified: Option<SystemTime>,
    now: SystemTime,
    own_pid: u32,
    is_running: impl Fn(u32) -> bool,
) -> bool {
    let older_than_any_run = modified
        .and_then(|modified| now.duration_since(modified).ok())
        .is_some_and(|age| age >= STALE_TEMP_FILE_AGE);
    if older_than_any_run {
        return true;
    }
    // A run of this process, or of another LocalFlow still running, may be using it.
    owner != own_pid && !is_running(owner)
}

/// Delete `path`, retrying briefly while another program (an antivirus scanner, the search
/// indexer) holds it open. A file that is already gone counts as deleted.
fn remove_file_with_retry(path: &Path) -> bool {
    for attempt in 0..5u64 {
        match fs::remove_file(path) {
            Ok(()) => return true,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return true,
            Err(_) => thread::sleep(Duration::from_millis(20 * (attempt + 1))),
        }
    }
    false
}

/// Create `path`, failing if anything already exists there, so a file or link planted in the
/// temporary folder is never written through or read back.
fn create_new(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

/// The two temporary files of one run. Both are deleted when this is dropped.
struct TempFiles {
    wav: PathBuf,
    json: PathBuf,
    /// What whisper.cpp is given with `-of`; it appends `.json`.
    output_base: PathBuf,
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        // Audio and transcript files never outlive the call, on success or failure.
        let wav_removed = remove_file_with_retry(&self.wav);
        let json_removed = remove_file_with_retry(&self.json);
        if !(wav_removed && json_removed) {
            tracing::warn!(
                "a temporary dictation file could not be deleted yet; it is removed at the next start"
            );
        }
    }
}

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
        // Files left by an earlier run that crashed are removed once per process.
        static SWEPT: Once = Once::new();
        SWEPT.call_once(|| {
            sweep_stale_temp_files_in(&self.config.temp_dir, SystemTime::now());
        });

        let wav = encode_wav_16k_mono(request.samples).map_err(AsrError::Failed)?;
        let files = self.create_temp_files(&wav)?;

        // `files` is dropped at the end of this function, deleting both files.
        self.spawn_and_wait(request, &files.wav, &files.output_base)
            .and_then(|()| {
                fs::read_to_string(&files.json).map_err(|error| {
                    AsrError::Failed(format!("Could not read whisper.cpp JSON output: {error}"))
                })
            })
            .and_then(|payload| parse_whisper_json(&payload).map_err(AsrError::Failed))
    }

    /// Claim a fresh pair of file names (unique per process and per call, so concurrent
    /// sessions never share a file) and write the audio. The JSON file is created empty first,
    /// so a stale or planted file under that name can never be read back as this transcript.
    fn create_temp_files(&self, wav: &[u8]) -> Result<TempFiles, AsrError> {
        for _ in 0..TEMP_NAME_ATTEMPTS {
            let stem = format!(
                "{TEMP_FILE_PREFIX}{}-{}",
                std::process::id(),
                self.sequence.fetch_add(1, Ordering::Relaxed)
            );
            let output_base = self.config.temp_dir.join(&stem);
            let wav_path = self.config.temp_dir.join(format!("{stem}.wav"));
            let json_path = self.config.temp_dir.join(format!("{stem}.json"));

            match create_new(&json_path) {
                Ok(_) => {}
                // Left by an earlier process with the same id: try the next name.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(AsrError::Unavailable(format!(
                        "Could not create a temporary output file: {error}"
                    )))
                }
            }
            let mut wav_file = match create_new(&wav_path) {
                Ok(file) => file,
                Err(error) => {
                    let _ = remove_file_with_retry(&json_path);
                    if error.kind() == io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(AsrError::Unavailable(format!(
                        "Could not create a temporary audio file: {error}"
                    )));
                }
            };
            // From here on both files are deleted whatever happens.
            let files = TempFiles {
                wav: wav_path,
                json: json_path,
                output_base,
            };
            wav_file
                .write_all(wav)
                .and_then(|()| wav_file.flush())
                .map_err(|error| {
                    AsrError::Failed(format!("Could not write the temporary audio file: {error}"))
                })?;
            // Closed before whisper.cpp opens it.
            drop(wav_file);
            return Ok(files);
        }
        Err(AsrError::Unavailable(
            "Could not find a free name for the temporary audio file.".to_string(),
        ))
    }

    fn spawn_and_wait(
        &self,
        request: &TranscriptionRequest<'_>,
        wav_path: &Path,
        output_base: &Path,
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

        // Inside a kill-on-close job: if LocalFlow exits or crashes mid-run, Windows ends the
        // process instead of leaving it running with the audio file open.
        let mut child = ContainedChild::spawn_hidden(&mut command).map_err(|error| {
            AsrError::Unavailable(format!("Could not start whisper.cpp: {error}"))
        })?;

        let deadline = transcription_deadline(request.audio_duration());
        let started_at = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    if started_at.elapsed() >= deadline {
                        child.kill_tree();
                        return Err(AsrError::Timeout {
                            provider: WHISPER_CLI_PROVIDER_ID,
                            waited: started_at.elapsed(),
                        });
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    child.kill_tree();
                    return Err(AsrError::Failed(format!(
                        "Could not wait for whisper.cpp: {error}"
                    )));
                }
            }
        };

        if let Some(bytes) = peak_memory_bytes(child.child_mut()) {
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

    fn stand_in_program() -> PathBuf {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:/Windows".to_string());
        // `where.exe` rejects whisper's flags and exits non-zero straight away.
        PathBuf::from(system_root)
            .join("System32")
            .join("where.exe")
    }

    /// Runs a real process that exits without producing output, then checks that the audio
    /// file written for it is gone and that the error carries no path.
    #[cfg(windows)]
    #[test]
    fn a_failed_run_removes_its_temp_files_and_reports_no_paths() {
        let stand_in = stand_in_program();
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

    /// A file already sitting under the next name (a crashed run of a process that had the
    /// same id) is neither overwritten nor read back: the run moves to a fresh name.
    #[cfg(windows)]
    #[test]
    fn an_existing_file_under_the_name_is_never_reused() {
        let stand_in = stand_in_program();
        let temp_dir =
            std::env::temp_dir().join(format!("localflow-cli-name-test-{}", std::process::id()));
        fs::create_dir_all(&temp_dir).unwrap();
        let planted = temp_dir.join(format!("dictation-{}-0.json", std::process::id()));
        fs::write(&planted, r#"{"transcription":[{"text":"planted"}]}"#).unwrap();
        let provider = WhisperCliProvider::new(WhisperCliConfig {
            cli_path: stand_in.clone(),
            model_path: stand_in,
            decode: WhisperDecodeOptions::default(),
            temp_dir: temp_dir.clone(),
        });

        let files = provider.create_temp_files(b"RIFF").unwrap();
        let claimed = (files.wav.clone(), files.json.clone());
        let planted_kept = fs::read_to_string(&planted).unwrap();
        drop(files);
        let claimed_gone = !claimed.0.exists() && !claimed.1.exists();
        let _ = fs::remove_dir_all(&temp_dir);

        assert_ne!(claimed.1, planted);
        assert!(planted_kept.contains("planted"));
        assert!(claimed_gone, "dropping the run's files deletes both");
    }

    #[test]
    fn only_this_providers_file_names_are_recognised() {
        assert_eq!(temp_file_owner("dictation-1234-0.wav"), Some(1234));
        assert_eq!(temp_file_owner("dictation-1234-17.json"), Some(1234));
        assert_eq!(temp_file_owner("dictation-1234-17.txt"), None);
        assert_eq!(temp_file_owner("dictation-1234.wav"), None);
        assert_eq!(temp_file_owner("dictation-abc-1.wav"), None);
        assert_eq!(temp_file_owner("dictation-12-x.wav"), None);
        assert_eq!(temp_file_owner("notes.wav"), None);
    }

    #[test]
    fn a_file_is_stale_when_its_process_is_gone_or_it_is_very_old() {
        let now = SystemTime::now();
        let fresh = Some(now - Duration::from_secs(5));
        let ancient = Some(now - STALE_TEMP_FILE_AGE - Duration::from_secs(1));
        let running = |pid: u32| pid == 50;

        // This process's own run in progress: kept.
        assert!(!is_stale_temp_file(7, fresh, now, 7, running));
        // Another LocalFlow that is still running: kept.
        assert!(!is_stale_temp_file(50, fresh, now, 7, running));
        // Its process has gone: removed.
        assert!(is_stale_temp_file(99, fresh, now, 7, running));
        assert!(is_stale_temp_file(99, None, now, 7, running));
        // Older than any run can last: removed, whoever wrote it.
        assert!(is_stale_temp_file(50, ancient, now, 7, running));
        assert!(is_stale_temp_file(7, ancient, now, 7, running));
    }

    #[test]
    fn the_sweep_removes_only_stale_dictation_files() {
        let dir =
            std::env::temp_dir().join(format!("localflow-cli-sweep-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        // u32::MAX is never a real process id, so its files are stale.
        let stale_wav = dir.join(format!("dictation-{}-3.wav", u32::MAX));
        let stale_json = dir.join(format!("dictation-{}-3.json", u32::MAX));
        let own = dir.join(format!("dictation-{}-9.wav", std::process::id()));
        let unrelated = dir.join("keep-me.wav");
        for path in [&stale_wav, &stale_json, &own, &unrelated] {
            fs::write(path, b"x").unwrap();
        }

        let removed = sweep_stale_temp_files_in(&dir, SystemTime::now());
        let remaining: Vec<bool> = [&stale_wav, &stale_json, &own, &unrelated]
            .iter()
            .map(|path| path.exists())
            .collect();
        let _ = fs::remove_dir_all(&dir);

        assert_eq!(removed, 2);
        assert_eq!(remaining, vec![false, false, true, true]);
    }

    #[test]
    fn sweeping_a_missing_folder_does_nothing() {
        let missing = std::env::temp_dir().join("localflow-cli-sweep-test-missing-folder");
        assert_eq!(sweep_stale_temp_files_in(&missing, SystemTime::now()), 0);
    }
}
