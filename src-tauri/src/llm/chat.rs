//! Streaming chat with the local language model, for conversation mode.
//!
//! [`ChatProvider`] is the boundary the conversation code sees; [`OllamaChat`] is the only
//! real implementation and talks to Ollama on loopback with the same guards as dictation
//! cleanup (no proxy, no redirects, cloud-hosted model tags refused). Replies arrive token by
//! token so the voice can start on the first sentence, and a cancel signal ends the request
//! by closing the connection. The model is given no tools: its output is only ever spoken.

use std::{
    io::{BufRead, BufReader},
    time::Duration,
};

use serde::{Deserialize, Serialize};

use super::ollama::{ensure_local_model, ModelRefusal, OLLAMA_KEEP_ALIVE, OLLAMA_NUM_CTX};
use crate::cancel::CancelFlag;

const OLLAMA_CHAT_URL: &str = "http://127.0.0.1:11434/api/chat";
/// A spoken reply is short. This bounds a model that will not stop.
pub const CHAT_NUM_PREDICT: u32 = 256;
const CHAT_TEMPERATURE: f32 = 0.6;
/// Longest wait for the whole reply. Loading a model from cold can take tens of seconds.
const CHAT_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: String,
}

impl ChatMessage {
    pub fn new(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: content.into(),
        }
    }
}

/// Why a reply could not be produced. Messages never contain conversation text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// Ollama is not running or cannot be reached on loopback.
    Unavailable(String),
    /// The configured model is not installed locally, or is a cloud-hosted tag.
    ModelRefused(String),
    /// The server answered with an error or something that is not a chat stream.
    Failed(String),
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unavailable(message) | Self::ModelRefused(message) | Self::Failed(message) => {
                formatter.write_str(message)
            }
        }
    }
}

impl std::error::Error for LlmError {}

/// Counters reported by the model when a reply ends. Used for latency diagnostics only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChatStats {
    pub prompt_tokens: u32,
    pub output_tokens: u32,
    /// Time the server spent reading the prompt. Large when its cache of the conversation so
    /// far could not be reused.
    pub prompt_eval_ms: u64,
    /// Time the server spent loading the model for this request (zero when already loaded).
    pub load_ms: u64,
}

/// A local language model that streams its reply.
pub trait ChatProvider: Send + Sync {
    /// Load the model so the first turn does not pay for it.
    fn prepare(&self, model: &str) -> Result<(), LlmError>;

    /// Keep the model ready to answer `messages` plus whatever the user is saying now.
    ///
    /// A laptop GPU drops to a low-power state after a few idle seconds, which is about how
    /// long a spoken question takes, and the next request then waits several hundred
    /// milliseconds for it to wake. Calling this a few times a second while the user speaks
    /// keeps it awake and keeps the server's cache of the conversation current. The default
    /// does nothing.
    fn keep_warm(&self, _model: &str, _messages: &[ChatMessage]) {}

    /// Send `messages` and call `on_text` with each piece of the reply as it arrives.
    /// Returns when the reply ends or soon after `cancel` is set; a cancelled request is not
    /// an error.
    fn stream_chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        cancel: &CancelFlag,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<ChatStats, LlmError>;
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
    /// Reasoning models otherwise think silently for many seconds before the first word.
    think: bool,
    keep_alive: &'a str,
    options: ChatOptions,
}

#[derive(Debug, Serialize)]
struct ChatOptions {
    temperature: f32,
    /// The same context size as every other caller, so the model is never reloaded.
    num_ctx: u32,
    num_predict: u32,
}

#[derive(Debug, Deserialize)]
struct ChatStreamLine {
    #[serde(default)]
    message: Option<ChatStreamMessage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    prompt_eval_count: u32,
    #[serde(default)]
    eval_count: u32,
    /// Nanoseconds.
    #[serde(default)]
    prompt_eval_duration: u64,
    /// Nanoseconds.
    #[serde(default)]
    load_duration: u64,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatStreamMessage {
    #[serde(default)]
    content: String,
}

/// Chat through the local Ollama server.
pub struct OllamaChat {
    url: String,
    /// `http://host:port` of the same server, for asking about a model before using it.
    base_url: String,
    client: reqwest::blocking::Client,
}

impl OllamaChat {
    pub fn new() -> Result<Self, LlmError> {
        Self::with_url(OLLAMA_CHAT_URL)
    }

    /// `url` must be a loopback address. Separate from [`Self::new`] so tests can point the
    /// client at a stand-in server.
    pub fn with_url(url: &str) -> Result<Self, LlmError> {
        if !is_loopback_url(url) {
            return Err(LlmError::Unavailable(
                "The language model address is not on this computer.".to_string(),
            ));
        }
        let client = reqwest::blocking::Client::builder()
            .timeout(CHAT_TIMEOUT)
            .connect_timeout(Duration::from_secs(2))
            // Conversation text must only ever reach the loopback server: ignore system and
            // environment proxies and never follow a redirect to another host.
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| {
                LlmError::Unavailable("Could not create the local model client.".to_string())
            })?;
        let authority = url
            .strip_prefix("http://")
            .and_then(|rest| rest.split('/').next())
            .unwrap_or_default();
        Ok(Self {
            url: url.to_string(),
            base_url: format!("http://{authority}"),
            client,
        })
    }

    fn send(
        &self,
        model: &str,
        messages: &[ChatMessage],
        num_predict: u32,
    ) -> Result<reqwest::blocking::Response, LlmError> {
        // Before any conversation text leaves for the server: is this model run here?
        ensure_local_model(&self.base_url, model).map_err(|refusal| match refusal {
            ModelRefusal::OllamaNotRunning => LlmError::Unavailable(
                "The local language model is not running. Start Ollama and try again.".to_string(),
            ),
            other => LlmError::ModelRefused(other.message(model)),
        })?;

        let response = self
            .client
            .post(&self.url)
            .json(&ChatRequest {
                model,
                messages,
                stream: true,
                think: false,
                keep_alive: OLLAMA_KEEP_ALIVE,
                options: ChatOptions {
                    temperature: CHAT_TEMPERATURE,
                    num_ctx: OLLAMA_NUM_CTX,
                    num_predict,
                },
            })
            .send()
            .map_err(|error| {
                if error.is_connect() {
                    LlmError::Unavailable(
                        "The local language model is not running. Start Ollama and try again."
                            .to_string(),
                    )
                } else if error.is_timeout() {
                    LlmError::Failed("The local language model did not answer in time.".to_string())
                } else {
                    LlmError::Failed("The request to the local language model failed.".to_string())
                }
            })?;

        let status = response.status();
        if status.as_u16() == 404 {
            return Err(LlmError::ModelRefused(format!(
                "Ollama model \"{model}\" is not installed. Run: ollama pull {model}"
            )));
        }
        if !status.is_success() {
            // The body can echo the request, so it is not included.
            return Err(LlmError::Failed(format!(
                "The local language model returned HTTP {}.",
                status.as_u16()
            )));
        }
        Ok(response)
    }
}

impl ChatProvider for OllamaChat {
    fn prepare(&self, model: &str) -> Result<(), LlmError> {
        // An empty message list loads the model without generating anything.
        let response = self.send(model, &[], 1)?;
        // Read to the end so the connection closes cleanly.
        for line in BufReader::new(response).lines() {
            if line.is_err() {
                break;
            }
        }
        Ok(())
    }

    fn keep_warm(&self, model: &str, messages: &[ChatMessage]) {
        // One generated token: enough to wake the GPU and evaluate any history the server
        // has not cached, and short enough that a real request never waits long behind it.
        if let Ok(response) = self.send(model, messages, 1) {
            for line in BufReader::new(response).lines() {
                if line.is_err() {
                    break;
                }
            }
        }
    }

    fn stream_chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        cancel: &CancelFlag,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<ChatStats, LlmError> {
        if cancel.is_cancelled() {
            return Ok(ChatStats::default());
        }
        let response = self.send(model, messages, CHAT_NUM_PREDICT)?;
        let mut stats = ChatStats::default();

        for line in BufReader::new(response).lines() {
            // Returning drops the response, which closes the connection; Ollama stops
            // generating when its client goes away.
            if cancel.is_cancelled() {
                return Ok(stats);
            }
            let line = line.map_err(|_| {
                LlmError::Failed("The connection to the local language model was lost.".to_string())
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let parsed: ChatStreamLine = serde_json::from_str(&line).map_err(|_| {
                LlmError::Failed("The local language model sent an unreadable reply.".to_string())
            })?;
            if parsed.error.is_some() {
                // The error text can quote the prompt, so only its presence is reported.
                return Err(LlmError::Failed(
                    "The local language model reported an error.".to_string(),
                ));
            }
            if let Some(message) = parsed.message {
                if !message.content.is_empty() {
                    on_text(&message.content);
                }
            }
            if parsed.done {
                stats.prompt_tokens = parsed.prompt_eval_count;
                stats.output_tokens = parsed.eval_count;
                stats.prompt_eval_ms = parsed.prompt_eval_duration / 1_000_000;
                stats.load_ms = parsed.load_duration / 1_000_000;
                break;
            }
        }
        Ok(stats)
    }
}

/// Whether `url` is plain HTTP to this computer.
fn is_loopback_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    // Reject "user@host" forms outright: the host is whatever follows the last '@'.
    if authority.contains('@') {
        return false;
    }
    let host = match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

#[cfg(test)]
pub mod mock {
    //! A scripted language model for tests.

    use std::{sync::Mutex, thread};

    use super::*;

    pub struct MockChat {
        /// Pieces of the reply, sent in order.
        pub pieces: Vec<String>,
        /// Pause before each piece.
        pub piece_delay: Duration,
        pub fail_with: Option<LlmError>,
        /// Every message list this model was asked to answer.
        pub requests: Mutex<Vec<Vec<ChatMessage>>>,
        /// How many keep-warm requests arrived.
        pub warm_calls: std::sync::atomic::AtomicUsize,
    }

    impl MockChat {
        pub fn replying(pieces: &[&str]) -> Self {
            Self {
                pieces: pieces.iter().map(|piece| piece.to_string()).collect(),
                piece_delay: Duration::ZERO,
                fail_with: None,
                requests: Mutex::new(Vec::new()),
                warm_calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl ChatProvider for MockChat {
        fn prepare(&self, _model: &str) -> Result<(), LlmError> {
            Ok(())
        }

        fn keep_warm(&self, _model: &str, _messages: &[ChatMessage]) {
            self.warm_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        fn stream_chat(
            &self,
            _model: &str,
            messages: &[ChatMessage],
            cancel: &CancelFlag,
            on_text: &mut dyn FnMut(&str),
        ) -> Result<ChatStats, LlmError> {
            self.requests.lock().unwrap().push(messages.to_vec());
            if let Some(error) = &self.fail_with {
                return Err(error.clone());
            }
            for piece in &self.pieces {
                if !self.piece_delay.is_zero() {
                    thread::sleep(self.piece_delay);
                }
                if cancel.is_cancelled() {
                    break;
                }
                on_text(piece);
            }
            Ok(ChatStats::default())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::mpsc,
        thread,
    };

    use super::*;

    /// A one-request HTTP server on loopback. Sends `status` and then each body line with a
    /// short pause between them; reports the request body and whether the client hung up
    /// before everything was sent.
    fn stand_in_server(
        status: &'static str,
        lines: Vec<String>,
        line_delay: Duration,
    ) -> (String, mpsc::Receiver<(String, bool)>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/api/chat", listener.local_addr().unwrap());
        let (report_tx, report_rx) = mpsc::channel();

        thread::spawn(move || loop {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            // Read headers, then the body announced by Content-Length.
            let body = loop {
                let read = socket.read(&mut buffer).unwrap();
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(split) = text.find("\r\n\r\n") {
                    let length = text[..split]
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if request.len() >= split + 4 + length {
                        break text[split + 4..].to_string();
                    }
                }
                if read == 0 {
                    break String::new();
                }
            };
            // The model check that precedes every request: a local model.
            if request.starts_with(b"POST /api/show ") {
                let answer = r#"{"details":{"format":"gguf"},"capabilities":["completion"]}"#;
                let _ = socket.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                );
                continue;
            }

            let mut hung_up = false;
            let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/x-ndjson\r\nConnection: close\r\n\r\n");
            if socket.write_all(head.as_bytes()).is_err() {
                hung_up = true;
            }
            for line in lines {
                thread::sleep(line_delay);
                if socket.write_all(format!("{line}\n").as_bytes()).is_err()
                    || socket.flush().is_err()
                {
                    hung_up = true;
                    break;
                }
            }
            let _ = report_tx.send((body, hung_up));
            break;
        });

        (url, report_rx)
    }

    fn token(text: &str) -> String {
        serde_json::json!({"message": {"role": "assistant", "content": text}, "done": false})
            .to_string()
    }

    fn done() -> String {
        serde_json::json!({"message": {"role": "assistant", "content": ""}, "done": true, "prompt_eval_count": 42, "eval_count": 7, "prompt_eval_duration": 31_000_000u64, "load_duration": 12_000_000u64}).to_string()
    }

    #[test]
    fn a_reply_is_delivered_piece_by_piece_with_its_counters() {
        let (url, report) = stand_in_server(
            "200 OK",
            vec![token("Hello"), token(" there."), done()],
            Duration::ZERO,
        );
        let chat = OllamaChat::with_url(&url).unwrap();
        let mut pieces = Vec::new();

        let stats = chat
            .stream_chat(
                "llama3.2:3b",
                &[ChatMessage::new(ChatRole::User, "hi")],
                &CancelFlag::default(),
                &mut |piece| pieces.push(piece.to_string()),
            )
            .unwrap();

        assert_eq!(pieces, vec!["Hello", " there."]);
        assert_eq!(
            stats,
            ChatStats {
                prompt_tokens: 42,
                output_tokens: 7,
                prompt_eval_ms: 31,
                load_ms: 12,
            }
        );

        // What was sent: streaming on, thinking off, the shared context size, no tools.
        let (body, _) = report.recv().unwrap();
        let request: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(request["stream"], true);
        assert_eq!(request["think"], false);
        assert_eq!(request["options"]["num_ctx"], OLLAMA_NUM_CTX);
        assert_eq!(request["messages"][0]["role"], "user");
        assert!(request.get("tools").is_none());
    }

    #[test]
    fn cancelling_stops_reading_and_closes_the_connection() {
        let many: Vec<String> = (0..200)
            .map(|index| token(&format!("word{index} ")))
            .collect();
        let (url, report) = stand_in_server("200 OK", many, Duration::from_millis(10));
        let chat = OllamaChat::with_url(&url).unwrap();
        let cancel = CancelFlag::default();
        let mut pieces = 0usize;

        chat.stream_chat(
            "llama3.2:3b",
            &[ChatMessage::new(ChatRole::User, "hi")],
            &cancel,
            &mut |_| {
                pieces += 1;
                if pieces == 3 {
                    cancel.cancel();
                }
            },
        )
        .expect("a cancelled request is not an error");

        assert_eq!(pieces, 3);
        // The server sees its client go away long before it has sent 200 lines.
        let (_, hung_up) = report.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(hung_up);
    }

    #[test]
    fn a_missing_model_says_how_to_install_it_without_the_response_body() {
        let (url, _) = stand_in_server(
            "404 Not Found",
            vec![r#"{"error":"model 'x' not found, SECRET-BODY"}"#.to_string()],
            Duration::ZERO,
        );
        let chat = OllamaChat::with_url(&url).unwrap();

        let error = chat
            .stream_chat("x:1b", &[], &CancelFlag::default(), &mut |_| {})
            .unwrap_err();

        assert!(matches!(error, LlmError::ModelRefused(_)));
        assert!(error.to_string().contains("ollama pull x:1b"));
        assert!(!error.to_string().contains("SECRET-BODY"));
    }

    #[test]
    fn an_error_inside_the_stream_is_reported_without_its_text() {
        let (url, _) = stand_in_server(
            "200 OK",
            vec![
                token("Hi"),
                r#"{"error":"prompt was: the user's private words"}"#.to_string(),
            ],
            Duration::ZERO,
        );
        let chat = OllamaChat::with_url(&url).unwrap();

        let error = chat
            .stream_chat("llama3.2:3b", &[], &CancelFlag::default(), &mut |_| {})
            .unwrap_err();

        assert!(matches!(error, LlmError::Failed(_)));
        assert!(!error.to_string().contains("private"));
    }

    #[test]
    fn a_server_that_is_not_running_is_reported_as_unavailable() {
        // Bind and drop to get a loopback port with nothing listening on it.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let chat = OllamaChat::with_url(&format!("http://127.0.0.1:{port}/api/chat")).unwrap();

        let error = chat
            .stream_chat("llama3.2:3b", &[], &CancelFlag::default(), &mut |_| {})
            .unwrap_err();

        assert!(matches!(error, LlmError::Unavailable(_)));
    }

    #[test]
    fn cloud_tags_and_non_loopback_addresses_are_refused_before_any_request() {
        let chat = OllamaChat::new().unwrap();
        let error = chat
            .stream_chat("gemma4:cloud", &[], &CancelFlag::default(), &mut |_| {})
            .unwrap_err();
        assert!(matches!(error, LlmError::ModelRefused(_)));

        for url in [
            "http://example.com/api/chat",
            "https://127.0.0.1:11434/api/chat",
            "http://127.0.0.1.example.com/api/chat",
            "http://127.0.0.1@example.com/api/chat",
            "http://192.168.1.10:11434/api/chat",
        ] {
            assert!(OllamaChat::with_url(url).is_err(), "{url}");
        }
        for url in [
            "http://127.0.0.1:11434/api/chat",
            "http://localhost:11434/x",
            "http://[::1]:11434/x",
        ] {
            assert!(OllamaChat::with_url(url).is_ok(), "{url}");
        }
    }
}
