//! Fish Audio S2 Pro voice engine, run locally through the `s2.cpp` server.
//!
//! `s2.cpp` is a community ggml runtime for the S2 Pro model. LocalFlow starts its HTTP
//! server as a supervised sidecar on a loopback ephemeral port, exactly like the whisper
//! server: started lazily, restarted if it exits, killed past its deadline, stopped on app
//! exit, tied to a kill-on-close job object, and checked for listener ownership before any
//! text is sent.
//!
//! The model is large (a 4B-parameter transformer; 3.6 GB at q4). The sidecar is therefore
//! never started in the background and is shut down after a period without use, so it costs
//! nothing unless read-aloud is actually used.
//!
//! Licence: the S2 Pro weights are under the Fish Audio Research License, which permits
//! research and personal non-commercial use only. They are not bundled with the installer.
//!
//! Privacy: the text travels only to `127.0.0.1`, the HTTP client ignores proxies and
//! redirects, the sidecar's output streams are discarded, and no voice cloning is used.

use std::{
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use super::provider::{
    prepare_text_for_speech, synthesis_deadline, SynthesizedAudio, TextToSpeechProvider, TtsError,
};
use crate::{
    asr::{
        model_label,
        sidecar::{hidden_sidecar_command, tcp_listener_pid, ChildLifetimeGuard},
    },
    audio::wav::decode_wav_mono,
    cancel::CancelFlag,
};

pub const FISH_S2_PROVIDER_ID: &str = "fish-s2";

/// Which compute backend to ask `s2.cpp` for. The binary must have been built with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FishS2Backend {
    #[default]
    Cpu,
    Vulkan,
    Cuda,
}

impl FishS2Backend {
    /// Parse the value written by the build script (`cpu`, `vulkan`, `cuda`).
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "vulkan" => Self::Vulkan,
            "cuda" => Self::Cuda,
            _ => Self::Cpu,
        }
    }

    fn server_args(self) -> &'static [&'static str] {
        match self {
            Self::Cpu => &[],
            Self::Vulkan => &["--vulkan", "0"],
            Self::Cuda => &["--cuda", "0"],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Vulkan => "vulkan",
            Self::Cuda => "cuda",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FishS2Config {
    pub server_path: PathBuf,
    pub model_path: PathBuf,
    pub tokenizer_path: PathBuf,
    pub backend: FishS2Backend,
    pub threads: usize,
    /// How long to wait for the model to load and the port to open.
    pub startup_timeout: Duration,
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
/// How often a request in progress checks whether it was cancelled.
const CANCEL_POLL: Duration = Duration::from_millis(25);

/// Where the server process is in its life.
#[derive(Default)]
enum ServerPhase {
    #[default]
    Stopped,
    /// A caller is starting it, without holding the lock (loading the model can take minutes).
    /// Setting the flag makes that start give up and kill the process.
    Starting {
        abort: Arc<AtomicBool>,
    },
    Running(RunningServer),
}

#[derive(Default)]
struct ServerState {
    phase: ServerPhase,
    last_start_failure: Option<(Instant, String)>,
}

pub struct FishS2Provider {
    config: FishS2Config,
    server: Mutex<ServerState>,
    /// Signalled when a start finishes, for callers waiting on it.
    started: Condvar,
    /// One request at a time: two would share, and on failure kill, one server.
    one_request: Mutex<()>,
    client: reqwest::blocking::Client,
    starts: AtomicU64,
}

impl FishS2Provider {
    pub fn new(config: FishS2Config) -> Result<Self, TtsError> {
        let client = loopback_client()?;
        Ok(Self {
            config,
            server: Mutex::new(ServerState::default()),
            started: Condvar::new(),
            one_request: Mutex::new(()),
            client,
            starts: AtomicU64::new(0),
        })
    }

    /// How many times a server process has been started.
    pub fn start_count(&self) -> u64 {
        self.starts.load(Ordering::Relaxed)
    }

    /// Whether the sidecar is currently loaded.
    pub fn is_loaded(&self) -> bool {
        match &mut self.lock_server().phase {
            ServerPhase::Running(server) => server.is_alive(),
            _ => false,
        }
    }

    fn lock_server(&self) -> std::sync::MutexGuard<'_, ServerState> {
        self.server
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn check_files(&self) -> Result<(), TtsError> {
        // File names only: full paths can contain the Windows user name.
        for (path, what) in [
            (&self.config.server_path, "Fish S2 engine"),
            (&self.config.model_path, "Fish S2 model"),
            (&self.config.tokenizer_path, "Fish S2 tokenizer"),
        ] {
            if !path.is_file() {
                return Err(TtsError::Unavailable(format!(
                    "Missing {what} ({})",
                    model_label(path)
                )));
            }
        }
        Ok(())
    }

    /// The port of a live server, starting one if needed. The lock is not held while the
    /// server starts, so stopping or unloading never waits for a model load; a second caller
    /// waits for the start already under way instead of starting another process.
    fn ensure_running(&self) -> Result<u16, TtsError> {
        let mut guard = self.lock_server();
        loop {
            match &mut guard.phase {
                ServerPhase::Running(server) => {
                    if server.is_alive() {
                        return Ok(server.port);
                    }
                    tracing::warn!("fish-s2 server exited unexpectedly; restarting");
                    if let ServerPhase::Running(dead) = std::mem::take(&mut guard.phase) {
                        dead.stop();
                    }
                }
                ServerPhase::Starting { .. } => {
                    guard = self
                        .started
                        .wait(guard)
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                }
                ServerPhase::Stopped => break,
            }
        }

        if let Some((failed_at, reason)) = &guard.last_start_failure {
            if failed_at.elapsed() < START_RETRY_BACKOFF {
                return Err(TtsError::Unavailable(format!(
                    "Fish S2 is not being restarted yet after a failed start: {reason}"
                )));
            }
        }

        let abort = Arc::new(AtomicBool::new(false));
        guard.phase = ServerPhase::Starting {
            abort: abort.clone(),
        };
        drop(guard);

        let result = self.check_files().and_then(|()| self.start_server(&abort));

        let mut guard = self.lock_server();
        let (outcome, discard) = match result {
            Ok(server) if !abort.load(Ordering::SeqCst) => {
                let port = server.port;
                guard.phase = ServerPhase::Running(server);
                guard.last_start_failure = None;
                (Ok(port), None)
            }
            Ok(server) => {
                guard.phase = ServerPhase::Stopped;
                (Err(stopped_while_starting()), Some(server))
            }
            Err(error) => {
                guard.phase = ServerPhase::Stopped;
                // A start that was called off is not a failure to back off from.
                if !abort.load(Ordering::SeqCst) {
                    guard.last_start_failure = Some((Instant::now(), error.to_string()));
                }
                (Err(error), None)
            }
        };
        drop(guard);
        self.started.notify_all();
        if let Some(server) = discard {
            server.stop();
        }
        outcome
    }

    fn start_server(&self, abort: &AtomicBool) -> Result<RunningServer, TtsError> {
        let port = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .and_then(|listener| listener.local_addr())
            .map(|address| address.port())
            .map_err(|error| {
                TtsError::Unavailable(format!(
                    "Could not reserve a local port for speech: {error}"
                ))
            })?;

        let mut command = hidden_sidecar_command(&self.config.server_path);
        // The server resolves `./voices` against its working directory; keep that inside the
        // engine folder rather than wherever LocalFlow happened to start.
        if let Some(engine_dir) = self.config.server_path.parent() {
            command.current_dir(engine_dir);
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .arg("--server")
            .arg("--host")
            .arg(Ipv4Addr::LOCALHOST.to_string())
            .arg("--port")
            .arg(port.to_string())
            .arg("--model")
            .arg(&self.config.model_path)
            .arg("--tokenizer")
            .arg(&self.config.tokenizer_path)
            .arg("--threads")
            .arg(self.config.threads.to_string())
            // Errors only, and discarded anyway: generation logs must not echo text.
            .arg("--log-level")
            .arg("error")
            .args(self.config.backend.server_args());

        let started_at = Instant::now();
        let mut child = command.spawn().map_err(|error| {
            TtsError::Unavailable(format!("Could not start the Fish S2 engine: {error}"))
        })?;
        let lifetime = ChildLifetimeGuard::attach(&child);
        if lifetime.is_none() && cfg!(windows) {
            tracing::warn!("could not bind fish-s2 to a job object; relying on drop to stop it");
        }
        self.starts.fetch_add(1, Ordering::Relaxed);

        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        loop {
            if abort.load(Ordering::SeqCst) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(stopped_while_starting());
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(TtsError::Unavailable(format!(
                    "The Fish S2 engine exited during startup with {status}"
                )));
            }
            if TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_ok() {
                if !cfg!(windows) || tcp_listener_pid(port) == Some(child.id()) {
                    break;
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(TtsError::Unavailable(
                    "another process is listening on the port reserved for Fish S2; \
                     no text was sent"
                        .to_string(),
                ));
            }
            if started_at.elapsed() >= self.config.startup_timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(TtsError::Unavailable(format!(
                    "The Fish S2 engine did not become ready within {} s",
                    self.config.startup_timeout.as_secs()
                )));
            }
            thread::sleep(Duration::from_millis(50));
        }

        tracing::info!(
            port,
            backend = self.config.backend.label(),
            startup_ms = started_at.elapsed().as_millis() as u64,
            "fish-s2 server ready"
        );

        Ok(RunningServer {
            child,
            port,
            _lifetime: lifetime,
        })
    }

    /// Stop the server, or call off a start in progress. Never waits for a model load.
    fn stop_server(&self) {
        let taken = {
            let mut guard = self.lock_server();
            match &guard.phase {
                ServerPhase::Starting { abort } => {
                    abort.store(true, Ordering::SeqCst);
                    None
                }
                ServerPhase::Running(_) => match std::mem::take(&mut guard.phase) {
                    ServerPhase::Running(server) => Some(server),
                    _ => None,
                },
                ServerPhase::Stopped => None,
            }
        };
        if let Some(server) = taken {
            server.stop();
        }
    }

    fn stop_server_on_port(&self, port: u16) {
        let taken = {
            let mut guard = self.lock_server();
            match &guard.phase {
                ServerPhase::Running(server) if server.port == port => {
                    match std::mem::take(&mut guard.phase) {
                        ServerPhase::Running(server) => Some(server),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        if let Some(server) = taken {
            server.stop();
        }
    }

    /// One request, at most two attempts: a server that exited while idle is restarted once.
    /// `port` is where the request was sent, for the watcher that stops it on cancel.
    fn synthesize_once(
        &self,
        text: &str,
        cancel: &CancelFlag,
        port_in_use: &AtomicU32,
    ) -> Result<SynthesizedAudio, TtsError> {
        let deadline = synthesis_deadline(text);
        let mut last_error = String::new();
        for attempt in 0..2 {
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let port = self.ensure_running()?;
            port_in_use.store(u32::from(port), Ordering::SeqCst);
            // A cancel that came while the server was starting found no port to stop.
            if cancel.is_cancelled() {
                return Err(TtsError::Cancelled);
            }
            let started_at = Instant::now();

            match request_speech(&self.client, port, text, deadline) {
                Ok(audio) => return Ok(audio),
                // The server was stopped to end this request: do not start it again for it.
                Err(_) if cancel.is_cancelled() => return Err(TtsError::Cancelled),
                Err(RequestError::Timeout) => {
                    self.stop_server_on_port(port);
                    return Err(TtsError::Timeout {
                        provider: FISH_S2_PROVIDER_ID,
                        waited: started_at.elapsed(),
                    });
                }
                Err(RequestError::BadAudio(reason)) => {
                    return Err(TtsError::Failed(format!(
                        "Fish S2 returned unusable audio: {reason}"
                    )));
                }
                Err(RequestError::Transport(reason)) => {
                    tracing::warn!(attempt, error = %reason, "fish-s2 request failed; restarting");
                    last_error = reason;
                    self.stop_server_on_port(port);
                }
            }
        }

        Err(TtsError::Failed(format!(
            "Fish S2 request failed twice: {last_error}"
        )))
    }
}

fn stopped_while_starting() -> TtsError {
    TtsError::Unavailable("The Fish S2 engine was stopped while it was starting.".to_string())
}
// An HTTP client that can only talk to loopback as written: no proxy, no redirects.
fn loopback_client() -> Result<reqwest::blocking::Client, TtsError> {
    reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(2))
        .build()
        .map_err(|error| {
            TtsError::Unavailable(format!("Could not create local speech client: {error}"))
        })
}

/// A `multipart/form-data` body carrying only the text field. The boundary is extended until
/// it does not occur in the text.
fn build_generate_form(text: &str) -> (String, Vec<u8>) {
    use std::hash::{BuildHasher, Hasher};

    let random = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let mut boundary = format!("----localflow-tts-{random:016x}");
    while text.contains(&boundary) {
        boundary.push('x');
    }

    let mut body = Vec::with_capacity(text.len() + 256);
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"text\"\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(text.as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    (boundary, body)
}

/// Why one `/generate` request failed.
enum RequestError {
    Timeout,
    /// Transport failure or a non-success status. Carries a transcript-free description.
    Transport(String),
    /// The engine answered but the body was not usable audio.
    BadAudio(String),
}

/// Send `text` to an `s2.cpp` server on `port` and decode the WAV it returns.
fn request_speech(
    client: &reqwest::blocking::Client,
    port: u16,
    text: &str,
    deadline: Duration,
) -> Result<SynthesizedAudio, RequestError> {
    let (boundary, body) = build_generate_form(text);

    let response = client
        .post(format!("http://127.0.0.1:{port}/generate"))
        .timeout(deadline)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(body)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                RequestError::Timeout
            } else {
                RequestError::Transport(error.without_url().to_string())
            }
        })?;

    let status = response.status();
    if !status.is_success() {
        // The body of an error response may quote the request, so only the status is kept.
        return Err(RequestError::Transport(format!(
            "the engine answered HTTP {}",
            status.as_u16()
        )));
    }

    let bytes = response.bytes().map_err(|error| {
        if error.is_timeout() {
            RequestError::Timeout
        } else {
            RequestError::Transport(error.without_url().to_string())
        }
    })?;

    let decoded = decode_wav_mono(&bytes).map_err(RequestError::BadAudio)?;
    if decoded.samples.is_empty() || decoded.sample_rate == 0 {
        return Err(RequestError::BadAudio(
            "the engine returned empty audio".to_string(),
        ));
    }

    Ok(SynthesizedAudio {
        samples: decoded.samples,
        sample_rate: decoded.sample_rate,
    })
}

impl TextToSpeechProvider for FishS2Provider {
    fn id(&self) -> &'static str {
        FISH_S2_PROVIDER_ID
    }

    fn describe(&self) -> String {
        format!(
            "Fish S2 Pro through s2.cpp on {} ({})",
            self.config.backend.label(),
            model_label(&self.config.model_path)
        )
    }

    fn prepare(&self) -> Result<(), TtsError> {
        self.ensure_running().map(|_| ())
    }

    /// One request at a time. While it runs, a watcher stops the server if `cancel` is set:
    /// the engine keeps generating for a client that has hung up, and minutes of work for
    /// audio nobody will hear would also hold up the next request. The next request starts
    /// the server again.
    fn synthesize(&self, text: &str, cancel: &CancelFlag) -> Result<SynthesizedAudio, TtsError> {
        let text = prepare_text_for_speech(text).ok_or(TtsError::NothingToSay)?;
        let _one_request = self
            .one_request
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if cancel.is_cancelled() {
            return Err(TtsError::Cancelled);
        }

        let port_in_use = AtomicU32::new(0);
        let finished = AtomicBool::new(false);
        thread::scope(|scope| {
            scope.spawn(|| {
                while !finished.load(Ordering::SeqCst) {
                    if cancel.is_cancelled() {
                        if let Ok(port) = u16::try_from(port_in_use.load(Ordering::SeqCst)) {
                            if port != 0 {
                                self.stop_server_on_port(port);
                            }
                        }
                        return;
                    }
                    thread::sleep(CANCEL_POLL);
                }
            });
            let result = self.synthesize_once(&text, cancel, &port_in_use);
            finished.store(true, Ordering::SeqCst);
            result
        })
    }

    fn shutdown(&self) {
        self.stop_server();
    }
}

impl Drop for FishS2Provider {
    fn drop(&mut self) {
        self.stop_server();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::wav::encode_wav_mono;
    use std::{
        io::{Read, Write},
        sync::mpsc,
    };

    fn config(server: &str) -> FishS2Config {
        FishS2Config {
            server_path: PathBuf::from(server),
            model_path: PathBuf::from("Z:/someone/models/s2-pro-q4_k_m.gguf"),
            tokenizer_path: PathBuf::from("Z:/someone/models/tokenizer.json"),
            backend: FishS2Backend::Cpu,
            threads: 4,
            startup_timeout: Duration::from_secs(1),
        }
    }

    /// A one-shot HTTP server standing in for `s2.cpp`. It reads one request, hands the raw
    /// bytes back through the returned channel, and answers with `status` and `body`.
    fn stub_server(
        status: &'static str,
        content_type: &'static str,
        body: Vec<u8>,
    ) -> (u16, std::sync::mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (request_tx, request_rx) = std::sync::mpsc::channel();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            // Read headers, then the declared body length.
            let (header_end, content_length) = loop {
                let read = stream.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..position]).to_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (position + 4, length);
                }
                if read == 0 {
                    break (request.len(), 0);
                }
            };
            while request.len() < header_end + content_length {
                let read = stream.read(&mut buffer).unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
            }
            let _ = request_tx.send(request);

            let head = format!(
                "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(&body);
        });

        (port, request_rx)
    }

    #[test]
    fn a_generate_request_sends_the_text_and_decodes_the_returned_wav() {
        let tone: Vec<f32> = (0..4_410)
            .map(|index| (index as f32 * 0.05).sin() * 0.5)
            .collect();
        let wav = encode_wav_mono(&tone, 44_100).unwrap();
        let (port, request_rx) = stub_server("200 OK", "audio/wav", wav);

        let audio = request_speech(
            &loopback_client().unwrap(),
            port,
            "Read this aloud.",
            Duration::from_secs(5),
        )
        .ok()
        .expect("stub answered with audio");

        assert_eq!(audio.sample_rate, 44_100);
        assert_eq!(audio.samples.len(), 4_410);

        let request = String::from_utf8_lossy(&request_rx.recv().unwrap()).to_string();
        assert!(request.starts_with("POST /generate HTTP/1.1"));
        assert!(request.contains("multipart/form-data; boundary=----localflow-tts-"));
        assert!(request.contains("name=\"text\"\r\n\r\nRead this aloud.\r\n"));
    }

    #[test]
    fn an_error_status_is_reported_without_the_response_body() {
        let (port, _request_rx) = stub_server(
            "503 Service Unavailable",
            "text/plain",
            b"Server busy: Read this aloud.".to_vec(),
        );

        let error = request_speech(
            &loopback_client().unwrap(),
            port,
            "Read this aloud.",
            Duration::from_secs(5),
        )
        .expect_err("stub answered with an error");

        match error {
            RequestError::Transport(reason) => {
                assert_eq!(reason, "the engine answered HTTP 503");
            }
            _ => panic!("expected a transport error"),
        }
    }

    #[test]
    fn a_body_that_is_not_audio_is_rejected() {
        let (port, _request_rx) = stub_server("200 OK", "audio/wav", b"not a wav file".to_vec());

        let error = request_speech(
            &loopback_client().unwrap(),
            port,
            "Hello.",
            Duration::from_secs(5),
        )
        .expect_err("stub answered with junk");

        assert!(matches!(error, RequestError::BadAudio(_)));
    }

    #[test]
    fn the_form_boundary_never_collides_with_the_text() {
        let (boundary, body) = build_generate_form("plain text");
        let body = String::from_utf8(body).unwrap();
        assert!(body.starts_with(&format!("--{boundary}\r\n")));
        assert!(body.ends_with(&format!("\r\n--{boundary}--\r\n")));
        assert_eq!(body.matches(&boundary).count(), 2);

        // Text that contains a boundary-like prefix still yields exactly two occurrences.
        let (probe, _) = build_generate_form("x");
        let tricky = format!("say {probe} twice");
        let (boundary, body) = build_generate_form(&tricky);
        let body = String::from_utf8(body).unwrap();
        assert_eq!(body.matches(&format!("--{boundary}")).count(), 2);
    }

    #[test]
    fn a_missing_engine_is_unavailable_names_no_path_and_is_not_retried() {
        let provider = FishS2Provider::new(config("Z:/someone/fish/s2.exe")).unwrap();

        let first = provider.prepare().unwrap_err().to_string();
        let second = provider
            .synthesize("Hello there.", &CancelFlag::default())
            .unwrap_err()
            .to_string();

        assert_eq!(first, "Missing Fish S2 engine (s2.exe)");
        assert!(second.contains("not being restarted yet"));
        assert!(!second.contains("Hello"));
        assert_eq!(provider.start_count(), 0);
        assert!(!provider.is_loaded());
    }

    #[test]
    fn empty_text_is_rejected_before_any_engine_work() {
        let provider = FishS2Provider::new(config("Z:/someone/fish/s2.exe")).unwrap();

        assert_eq!(
            provider.synthesize("  \n ", &CancelFlag::default()),
            Err(TtsError::NothingToSay)
        );
        assert_eq!(provider.start_count(), 0);
    }

    #[test]
    fn a_request_cancelled_before_it_starts_sends_nothing_and_starts_nothing() {
        let provider = FishS2Provider::new(config("Z:/someone/fish/s2.exe")).unwrap();
        let cancel = CancelFlag::default();
        cancel.cancel();

        assert_eq!(
            provider.synthesize("Hello there.", &cancel),
            Err(TtsError::Cancelled)
        );
        assert_eq!(provider.start_count(), 0);
    }

    #[test]
    fn stopping_during_a_start_calls_it_off_without_waiting_for_it() {
        let provider = FishS2Provider::new(config("Z:/someone/fish/s2.exe")).unwrap();
        let abort = Arc::new(AtomicBool::new(false));
        provider.lock_server().phase = ServerPhase::Starting {
            abort: abort.clone(),
        };

        let started = Instant::now();
        provider.shutdown();

        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(abort.load(Ordering::SeqCst));
        // The starter owns the phase until its start returns.
        assert!(matches!(
            provider.lock_server().phase,
            ServerPhase::Starting { .. }
        ));
    }

    #[test]
    fn a_second_caller_waits_for_the_start_already_under_way() {
        let provider = Arc::new(FishS2Provider::new(config("Z:/someone/fish/s2.exe")).unwrap());
        provider.lock_server().phase = ServerPhase::Starting {
            abort: Arc::new(AtomicBool::new(false)),
        };

        let waiting = provider.clone();
        let caller = thread::spawn(move || {
            let started = Instant::now();
            let result = waiting.prepare();
            (result, started.elapsed())
        });
        thread::sleep(Duration::from_millis(150));
        {
            // The first start fails.
            let mut state = provider.lock_server();
            state.phase = ServerPhase::Stopped;
            state.last_start_failure = Some((Instant::now(), "it broke".to_string()));
        }
        provider.started.notify_all();

        let (result, waited) = caller.join().unwrap();
        assert!(waited >= Duration::from_millis(140), "{waited:?}");
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not being restarted yet"));
        assert_eq!(provider.start_count(), 0);
    }

    /// A stand-in for a running `s2.cpp`: a real (harmless) child process in the provider's
    /// state, and a listener on its port that holds each request open until told to hang up.
    #[cfg(windows)]
    fn provider_with_a_hanging_server() -> (Arc<FishS2Provider>, mpsc::Sender<()>, Arc<AtomicU32>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let child = std::process::Command::new("ping")
            .args(["-n", "60", "127.0.0.1"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let provider = Arc::new(FishS2Provider::new(config("Z:/someone/fish/s2.exe")).unwrap());
        provider.lock_server().phase = ServerPhase::Running(RunningServer {
            child,
            port,
            _lifetime: None,
        });

        let (hang_up_tx, hang_up_rx) = mpsc::channel::<()>();
        let connections = Arc::new(AtomicU32::new(0));
        let counted = connections.clone();
        thread::spawn(move || {
            let mut open = Vec::new();
            listener.set_nonblocking(true).unwrap();
            loop {
                if let Ok((stream, _)) = listener.accept() {
                    counted.fetch_add(1, Ordering::SeqCst);
                    open.push(stream);
                }
                if hang_up_rx.try_recv().is_ok() {
                    // As the killed engine would: every connection closes.
                    open.clear();
                }
                thread::sleep(Duration::from_millis(5));
            }
        });
        (provider, hang_up_tx, connections)
    }

    #[cfg(windows)]
    #[test]
    fn cancelling_a_request_in_progress_stops_the_engine_and_does_not_restart_it() {
        let (provider, hang_up, connections) = provider_with_a_hanging_server();
        let cancel = CancelFlag::default();

        let worker = {
            let provider = provider.clone();
            let cancel = cancel.clone();
            thread::spawn(move || provider.synthesize("Read this aloud.", &cancel))
        };
        let wait_until = |what: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(5);
            while !what() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            what()
        };
        assert!(wait_until(&|| connections.load(Ordering::SeqCst) == 1));
        assert!(provider.is_loaded());

        cancel.cancel();
        // The watcher stops the engine; its connections then close.
        assert!(wait_until(&|| !provider.is_loaded()));
        hang_up.send(()).unwrap();

        assert_eq!(worker.join().unwrap(), Err(TtsError::Cancelled));
        assert_eq!(
            provider.start_count(),
            0,
            "a cancelled request must not restart it"
        );
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    #[cfg(windows)]
    #[test]
    fn requests_run_one_at_a_time() {
        let (provider, hang_up, connections) = provider_with_a_hanging_server();
        let first_cancel = CancelFlag::default();
        let second_cancel = CancelFlag::default();

        let first = {
            let (provider, cancel) = (provider.clone(), first_cancel.clone());
            thread::spawn(move || provider.synthesize("First.", &cancel))
        };
        thread::sleep(Duration::from_millis(200));
        let second = {
            let (provider, cancel) = (provider.clone(), second_cancel.clone());
            thread::spawn(move || provider.synthesize("Second.", &cancel))
        };
        thread::sleep(Duration::from_millis(300));
        // The second waits for the first instead of sharing its server.
        assert_eq!(connections.load(Ordering::SeqCst), 1);

        second_cancel.cancel();
        first_cancel.cancel();
        thread::sleep(Duration::from_millis(100));
        hang_up.send(()).unwrap();
        assert_eq!(first.join().unwrap(), Err(TtsError::Cancelled));
        assert_eq!(second.join().unwrap(), Err(TtsError::Cancelled));
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn backend_parsing_defaults_to_cpu() {
        assert_eq!(FishS2Backend::parse("vulkan\n"), FishS2Backend::Vulkan);
        assert_eq!(FishS2Backend::parse("CUDA"), FishS2Backend::Cuda);
        assert_eq!(FishS2Backend::parse("anything else"), FishS2Backend::Cpu);
        assert_eq!(FishS2Backend::Vulkan.server_args(), ["--vulkan", "0"]);
        assert!(FishS2Backend::Cpu.server_args().is_empty());
    }
}
