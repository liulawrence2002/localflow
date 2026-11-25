//! Persistent whisper.cpp server provider.
//!
//! Keeps one `whisper-server.exe` alive with the model loaded, bound to the loopback interface
//! on an ephemeral port, and sends each utterance to it as an in-memory WAV. Compared with
//! spawning the CLI per utterance this removes process start, model load, and the temporary
//! audio file from the latency path.
//!
//! Supervision rules:
//! - The server is started lazily by `prepare` or the first `transcribe`.
//! - A server that exited or stopped answering is restarted once per request.
//! - A request that exceeds its deadline kills the server so a stuck decode cannot keep
//!   burning CPU; the next request starts a fresh one.
//! - The process is killed on `shutdown`, on drop, and (on Windows) by a kill-on-close job
//!   object if LocalFlow itself dies.
//! - A server that fails to start is not retried for a minute, so a broken sidecar costs
//!   one startup timeout instead of one per dictation; the caller's fallback engine is used.
//! - On Windows the listener on the reserved port must be owned by the child LocalFlow
//!   started. If another local process took the port, no audio is sent to it.
//!
//! Privacy: audio only ever travels to `127.0.0.1`, the HTTP client ignores system and
//! environment proxies, and the server's stdout and stderr are discarded.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    provider::{
        transcription_deadline, AsrError, SpeechToTextProvider, Transcription, TranscriptionRequest,
    },
    sidecar::{hidden_sidecar_command, peak_memory_bytes, tcp_listener_pid, ChildLifetimeGuard},
    whisper_text::parse_whisper_json,
    WhisperDecodeOptions,
};
use crate::audio::wav::encode_wav_16k_mono;

pub const WHISPER_SERVER_PROVIDER_ID: &str = "whisper-server";

#[derive(Debug, Clone)]
pub struct WhisperServerConfig {
    pub server_path: PathBuf,
    pub model_path: PathBuf,
    pub decode: WhisperDecodeOptions,
    /// How long to wait for the model to load and the port to open.
    pub startup_timeout: Duration,
    /// Optional Silero VAD model. When set, whisper.cpp drops non-speech audio before
    /// decoding, which stops Whisper inventing text for silence or background noise.
    pub vad_model_path: Option<PathBuf>,
}

impl WhisperServerConfig {
    pub fn new(server_path: PathBuf, model_path: PathBuf, decode: WhisperDecodeOptions) -> Self {
        Self {
            server_path,
            model_path,
            decode,
            startup_timeout: Duration::from_secs(15),
            vad_model_path: None,
        }
    }

    pub fn with_vad_model(mut self, vad_model_path: Option<PathBuf>) -> Self {
        self.vad_model_path = vad_model_path.filter(|path| path.is_file());
        self
    }
}

struct RunningServer {
    child: Child,
    port: u16,
    // Declared after `child` on purpose: closing the job kills the process if it is still up.
    _lifetime: Option<ChildLifetimeGuard>,
}

impl RunningServer {
    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// How long a failed start is remembered before another attempt is made.
const START_RETRY_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Default)]
struct ServerState {
    running: Option<RunningServer>,
    /// When the last start attempt failed, and why.
    last_start_failure: Option<(Instant, String)>,
}

pub struct WhisperServerProvider {
    config: WhisperServerConfig,
    server: Mutex<ServerState>,
    client: reqwest::blocking::Client,
    starts: AtomicU64,
    requests: AtomicU64,
    /// Set by [`SpeechToTextProvider::shutdown`]: the provider has been replaced or the app is
    /// closing, so its server is never started again. Without it, the request that was
    /// running when the server was stopped retried and started a server nobody would stop.
    retired: AtomicBool,
}

impl WhisperServerProvider {
    pub fn new(config: WhisperServerConfig) -> Result<Self, AsrError> {
        let client = reqwest::blocking::Client::builder()
            // Loopback traffic must never be routed through a system or environment proxy.
            .no_proxy()
            // A loopback sidecar has no reason to redirect; following one could leave the host.
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| {
                AsrError::Unavailable(format!("Could not create local ASR client: {error}"))
            })?;

        Ok(Self {
            config,
            server: Mutex::new(ServerState::default()),
            client,
            starts: AtomicU64::new(0),
            requests: AtomicU64::new(0),
            retired: AtomicBool::new(false),
        })
    }

    /// How many times a server process has been started (first start plus restarts).
    pub fn start_count(&self) -> u64 {
        self.starts.load(Ordering::Relaxed)
    }

    /// Peak resident memory of the running server in bytes, when measurable.
    pub fn peak_memory_bytes(&self) -> Option<u64> {
        let guard = self.lock_server();
        guard
            .running
            .as_ref()
            .and_then(|server| peak_memory_bytes(&server.child))
    }

    /// Kill the server process without telling the provider, as a crash would. Test and
    /// benchmark hook for exercising recovery.
    pub fn simulate_crash(&self) {
        if let Some(server) = self.lock_server().running.as_mut() {
            let _ = server.child.kill();
            let _ = server.child.wait();
        }
    }

    fn lock_server(&self) -> std::sync::MutexGuard<'_, ServerState> {
        self.server
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn check_files(&self) -> Result<(), AsrError> {
        if !self.config.server_path.is_file() {
            // File names only: full paths can contain the Windows user name and these
            // messages reach the log and the health file.
            return Err(AsrError::Unavailable(format!(
                "Missing whisper.cpp server executable ({})",
                super::model_label(&self.config.server_path)
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

    /// Return the port of a live server, starting one if needed. Holding the lock for the
    /// whole start means concurrent callers wait for the same process instead of racing to
    /// spawn several.
    fn ensure_running(&self) -> Result<u16, AsrError> {
        if self.is_retired() {
            return Err(retired_error());
        }
        let mut guard = self.lock_server();

        if let Some(server) = guard.running.as_mut() {
            if server.is_alive() {
                return Ok(server.port);
            }
            tracing::warn!("whisper-server exited unexpectedly; restarting");
        }
        if let Some(dead) = guard.running.take() {
            dead.stop();
        }

        if let Some((failed_at, reason)) = &guard.last_start_failure {
            if failed_at.elapsed() < START_RETRY_BACKOFF {
                return Err(AsrError::Unavailable(format!(
                    "whisper.cpp server is not being restarted yet after a failed start: {reason}"
                )));
            }
        }

        match self.check_files().and_then(|()| self.start_server()) {
            // Retired while it was starting (the shutdown could not take the lock).
            Ok(server) if self.is_retired() => {
                server.stop();
                Err(retired_error())
            }
            Ok(server) => {
                let port = server.port;
                guard.running = Some(server);
                guard.last_start_failure = None;
                Ok(port)
            }
            Err(error) => {
                guard.last_start_failure = Some((Instant::now(), error.to_string()));
                Err(error)
            }
        }
    }

    fn start_server(&self) -> Result<RunningServer, AsrError> {
        let port = reserve_loopback_port()?;
        let mut command = hidden_sidecar_command(&self.config.server_path);
        if let Some(runtime_dir) = self.config.server_path.parent() {
            command.current_dir(runtime_dir);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("-m")
            .arg(&self.config.model_path)
            .arg("--host")
            .arg(Ipv4Addr::LOCALHOST.to_string())
            .arg("--port")
            .arg(port.to_string())
            .arg("-t")
            .arg(self.config.decode.threads.to_string())
            .arg("-bs")
            .arg(self.config.decode.beam_size.to_string())
            .arg("-bo")
            .arg(self.config.decode.beam_size.to_string())
            .arg("-nt");
        if let Some(vad_model) = &self.config.vad_model_path {
            command.arg("--vad").arg("-vm").arg(vad_model);
        }

        let started_at = Instant::now();
        let mut child = command.spawn().map_err(|error| {
            AsrError::Unavailable(format!("Could not start whisper.cpp server: {error}"))
        })?;
        let lifetime = ChildLifetimeGuard::attach(&child);
        if lifetime.is_none() && cfg!(windows) {
            tracing::warn!(
                "could not bind whisper-server to a job object; relying on drop to stop it"
            );
        }
        self.starts.fetch_add(1, Ordering::Relaxed);

        // The server opens its port only after the model has loaded, so a successful TCP
        // connect means it is ready to transcribe.
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return Err(AsrError::Unavailable(format!(
                    "whisper.cpp server exited during startup with {status}"
                )));
            }
            if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
                // Something is listening. Make sure it is the child and not another local
                // process that bound the port between its reservation and the child's bind.
                if listener_belongs_to(port, child.id()) {
                    break;
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(AsrError::Unavailable(
                    "another process is listening on the port reserved for whisper.cpp; \
                     no audio was sent"
                        .to_string(),
                ));
            }
            if started_at.elapsed() >= self.config.startup_timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AsrError::Unavailable(format!(
                    "whisper.cpp server did not become ready within {} ms",
                    self.config.startup_timeout.as_millis()
                )));
            }
            thread::sleep(Duration::from_millis(15));
        }

        tracing::info!(
            port,
            startup_ms = started_at.elapsed().as_millis() as u64,
            "whisper-server ready"
        );

        Ok(RunningServer {
            child,
            port,
            _lifetime: lifetime,
        })
    }

    fn stop_server(&self) {
        if let Some(server) = self.lock_server().running.take() {
            server.stop();
        }
    }

    fn is_retired(&self) -> bool {
        self.retired.load(Ordering::SeqCst)
    }

    /// Stop for good. Never waits for a server that is starting: that start stops the
    /// process itself once it sees the flag.
    fn retire(&self) {
        self.retired.store(true, Ordering::SeqCst);
        let running = match self.server.try_lock() {
            Ok(mut guard) => guard.running.take(),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                poisoned.into_inner().running.take()
            }
            Err(std::sync::TryLockError::WouldBlock) => None,
        };
        if let Some(server) = running {
            server.stop();
        }
    }

    /// Stop the server only if it is still the instance that was listening on `port`. A
    /// request that failed against an old instance must not kill a newer one that another
    /// dictation is using.
    fn stop_server_on_port(&self, port: u16) {
        let mut guard = self.lock_server();
        if guard
            .running
            .as_ref()
            .is_some_and(|server| server.port == port)
        {
            if let Some(server) = guard.running.take() {
                server.stop();
            }
        }
    }

    fn post_inference(
        &self,
        port: u16,
        body: &[u8],
        boundary: &str,
        deadline: Duration,
    ) -> Result<String, reqwest::Error> {
        self.client
            .post(format!("http://127.0.0.1:{port}/inference"))
            .timeout(deadline)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(body.to_vec())
            .send()?
            .error_for_status()?
            .text()
    }
}

/// Whether the listener on `port` is owned by process `child_pid`. On Windows this is
/// checked against the TCP listener table and fails closed. Other platforms cannot check and
/// accept the listener.
fn listener_belongs_to(port: u16, child_pid: u32) -> bool {
    if cfg!(windows) {
        tcp_listener_pid(port) == Some(child_pid)
    } else {
        true
    }
}

/// Ask the OS for a free loopback port. The listener is released just before the server
/// binds it; if another process wins that race, `start_server` sees that the listener is not
/// the child and refuses to use it.
fn reserve_loopback_port() -> Result<u16, AsrError> {
    TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| {
            AsrError::Unavailable(format!("Could not reserve a local port for ASR: {error}"))
        })
}

/// Build a `multipart/form-data` body with text `fields` and one WAV `file` part.
fn build_multipart_body(boundary: &str, fields: &[(&str, &str)], wav: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(wav.len() + 512);

    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; \
             filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(wav);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    body
}

/// A fresh multipart boundary that does not occur anywhere in the request body.
fn multipart_boundary(fields: &[(&str, &str)], wav: &[u8], sequence: u64) -> String {
    use std::hash::{BuildHasher, Hasher};

    // `RandomState` is seeded from OS randomness, so the boundary is unpredictable.
    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    disambiguate_boundary(
        format!("----localflow-asr-{sequence}-{random:016x}"),
        fields,
        wav,
    )
}

/// Extend `candidate` until it appears in neither a text field nor the audio bytes.
fn disambiguate_boundary(mut candidate: String, fields: &[(&str, &str)], wav: &[u8]) -> String {
    while fields.iter().any(|(_, value)| value.contains(&candidate))
        || contains_bytes(wav, candidate.as_bytes())
    {
        candidate.push('x');
    }
    candidate
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

impl SpeechToTextProvider for WhisperServerProvider {
    fn id(&self) -> &'static str {
        WHISPER_SERVER_PROVIDER_ID
    }

    fn describe(&self) -> String {
        format!(
            "whisper.cpp server, model kept loaded ({}{})",
            super::model_label(&self.config.model_path),
            if self.config.vad_model_path.is_some() {
                ", Silero VAD"
            } else {
                ""
            }
        )
    }

    fn prepare(&self) -> Result<(), AsrError> {
        self.ensure_running().map(|_| ())
    }

    fn transcribe(&self, request: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
        let wav = encode_wav_16k_mono(request.samples).map_err(AsrError::Failed)?;

        let mut fields: Vec<(&str, &str)> = vec![
            ("response_format", "json"),
            ("temperature", "0.0"),
            ("temperature_inc", "0.2"),
            ("no_timestamps", "true"),
            ("language", request.language),
        ];
        if let Some(prompt) = request.initial_prompt {
            fields.push(("prompt", prompt));
        }
        let audio_ctx = self
            .config
            .decode
            .audio_context
            .frames_for(request.audio_duration())
            .map(|frames| frames.to_string());
        if let Some(frames) = audio_ctx.as_deref() {
            fields.push(("audio_ctx", frames));
        }

        let boundary =
            multipart_boundary(&fields, &wav, self.requests.fetch_add(1, Ordering::Relaxed));
        let body = build_multipart_body(&boundary, &fields, &wav);
        let deadline = transcription_deadline(request.audio_duration());

        let mut last_error = String::new();
        // One retry: a server that crashed or was killed between dictations is restarted
        // transparently and the same audio is sent again.
        for attempt in 0..2 {
            let port = self.ensure_running()?;
            let started_at = Instant::now();

            match self.post_inference(port, &body, &boundary, deadline) {
                Ok(payload) => {
                    let text = parse_whisper_json(&payload).map_err(AsrError::Failed)?;
                    return Ok(Transcription {
                        text,
                        provider: WHISPER_SERVER_PROVIDER_ID,
                        fallback_reason: None,
                    });
                }
                Err(error) if error.is_timeout() => {
                    self.stop_server_on_port(port);
                    return Err(AsrError::Timeout {
                        provider: WHISPER_SERVER_PROVIDER_ID,
                        waited: started_at.elapsed(),
                    });
                }
                Err(error) => {
                    // `without_url` keeps the message free of anything but the failure kind.
                    last_error = error.without_url().to_string();
                    tracing::warn!(
                        attempt,
                        error = %last_error,
                        "whisper-server request failed; restarting the server"
                    );
                    self.stop_server_on_port(port);
                }
            }
        }

        Err(AsrError::Failed(format!(
            "whisper.cpp server request failed twice: {last_error}"
        )))
    }

    /// Called only when the provider is replaced or the app exits, so it retires the
    /// provider: the server is stopped and never started again.
    fn shutdown(&self) {
        self.retire();
    }
}

fn retired_error() -> AsrError {
    AsrError::Unavailable("whisper.cpp server was shut down".to_string())
}

impl Drop for WhisperServerProvider {
    fn drop(&mut self) {
        self.stop_server();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_body_has_fields_file_part_and_closing_boundary() {
        let body = build_multipart_body(
            "BOUNDARY",
            &[("response_format", "json"), ("prompt", "Glossary: Tauri.")],
            b"RIFFdata",
        );
        let text = String::from_utf8_lossy(&body);

        assert!(text.starts_with("--BOUNDARY\r\n"));
        assert!(text
            .contains("Content-Disposition: form-data; name=\"response_format\"\r\n\r\njson\r\n"));
        assert!(text.contains("name=\"prompt\"\r\n\r\nGlossary: Tauri.\r\n"));
        assert!(text.contains(
            "name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\nRIFFdata\r\n"
        ));
        assert!(text.ends_with("--BOUNDARY--\r\n"));
    }

    #[test]
    fn boundary_is_extended_until_it_is_absent_from_fields_and_audio() {
        // The candidate "B" occurs in a field and "Bx" occurs in the audio bytes, so the
        // result must be extended twice.
        let fields = [("prompt", "contains B here")];
        assert_eq!(
            disambiguate_boundary("B".to_string(), &fields, b"..Bx.."),
            "Bxx"
        );
        // A candidate that collides with nothing is returned unchanged.
        assert_eq!(
            disambiguate_boundary("Q".to_string(), &fields, b"audio"),
            "Q"
        );
    }

    #[test]
    fn generated_boundaries_differ_between_requests() {
        let first = multipart_boundary(&[], b"", 1);
        let second = multipart_boundary(&[], b"", 1);
        assert_ne!(first, second);
        assert!(first.starts_with("----localflow-asr-1-"));
    }

    #[test]
    fn a_failed_start_is_not_retried_inside_the_backoff_window() {
        let provider = WhisperServerProvider::new(WhisperServerConfig::new(
            PathBuf::from("Z:/definitely/missing/whisper-server.exe"),
            PathBuf::from("Z:/missing/model.bin"),
            WhisperDecodeOptions::default(),
        ))
        .unwrap();

        let first = provider.prepare().unwrap_err().to_string();
        let second = provider.prepare().unwrap_err().to_string();

        assert!(first.contains("Missing whisper.cpp server executable (whisper-server.exe)"));
        assert!(!first.contains("Z:/"), "error must not contain a full path");
        assert!(second.contains("not being restarted yet"));
        assert_eq!(provider.start_count(), 0);
    }

    #[test]
    fn a_retired_provider_never_starts_its_server_again() {
        let provider = WhisperServerProvider::new(WhisperServerConfig::new(
            PathBuf::from("Z:/definitely/missing/whisper-server.exe"),
            PathBuf::from("Z:/missing/model.bin"),
            WhisperDecodeOptions::default(),
        ))
        .unwrap();

        provider.shutdown();

        let error = provider.prepare().unwrap_err().to_string();
        assert!(error.contains("shut down"), "{error}");
        let samples = vec![0.0f32; 1_600];
        let request = TranscriptionRequest {
            samples: &samples,
            language: "en",
            initial_prompt: None,
        };
        // The retry inside a transcription refuses too, rather than starting a server.
        assert!(provider.transcribe(&request).is_err());
        assert_eq!(provider.start_count(), 0);
    }

    #[test]
    fn retiring_does_not_wait_for_a_start_in_progress() {
        let provider = std::sync::Arc::new(
            WhisperServerProvider::new(WhisperServerConfig::new(
                PathBuf::from("Z:/definitely/missing/whisper-server.exe"),
                PathBuf::from("Z:/missing/model.bin"),
                WhisperDecodeOptions::default(),
            ))
            .unwrap(),
        );
        // A start in progress holds the server lock.
        let held = provider.lock_server();
        let started = Instant::now();
        let retiring = {
            let provider = provider.clone();
            thread::spawn(move || provider.shutdown())
        };
        retiring.join().unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(provider.is_retired());
        drop(held);
    }

    #[test]
    fn reserved_port_is_a_usable_loopback_port() {
        let port = reserve_loopback_port().unwrap();
        assert!(port >= 1024);
        // The reservation is released, so the port can be bound again.
        assert!(TcpListener::bind((Ipv4Addr::LOCALHOST, port)).is_ok());
    }

    #[test]
    fn missing_server_binary_is_unavailable_and_starts_nothing() {
        let provider = WhisperServerProvider::new(WhisperServerConfig::new(
            PathBuf::from("Z:/definitely/missing/whisper-server.exe"),
            PathBuf::from("Z:/missing/model.bin"),
            WhisperDecodeOptions::default(),
        ))
        .unwrap();

        let error = provider.prepare().unwrap_err();

        assert!(matches!(error, AsrError::Unavailable(_)));
        assert_eq!(provider.start_count(), 0);
        assert_eq!(provider.peak_memory_bytes(), None);
    }
    /// End-to-end check against the real sidecar in `.localflow-runtime`. Ignored by default
    /// because it needs the local runtime and a model; run with
    /// `cargo test real_server -- --ignored`.
    #[test]
    #[ignore = "needs .localflow-runtime with whisper-server.exe, a model, and the bench corpus"]
    fn real_server_transcribes_recovers_from_a_crash_and_stops() {
        let runtime = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".localflow-runtime");
        let runtime = std::path::absolute(runtime).unwrap();
        let wav = runtime.join("bench-corpus").join("wav").join("jfk.wav");
        let audio = crate::audio::wav::read_wav_mono(&wav).expect("bench corpus jfk.wav");
        let samples = crate::audio::resample::resample_to_16khz(&audio.samples, audio.sample_rate);

        let provider = WhisperServerProvider::new(WhisperServerConfig::new(
            runtime
                .join("whisper")
                .join("Release")
                .join("whisper-server.exe"),
            runtime.join("models").join("ggml-tiny.en-q5_1.bin"),
            WhisperDecodeOptions::default(),
        ))
        .unwrap();
        let request = TranscriptionRequest {
            samples: &samples,
            language: "en",
            initial_prompt: Some("Glossary: Americans."),
        };

        let first = provider.transcribe(&request).unwrap();
        assert!(first.text.to_lowercase().contains("fellow americans"));
        assert_eq!(provider.start_count(), 1);

        // A crashed server is restarted transparently and the same audio still transcribes.
        provider.simulate_crash();
        let second = provider.transcribe(&request).unwrap();
        assert_eq!(second.text, first.text);
        assert_eq!(provider.start_count(), 2);

        provider.shutdown();
        assert_eq!(provider.peak_memory_bytes(), None);
    }
}
