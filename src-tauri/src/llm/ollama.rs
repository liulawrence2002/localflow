//! Client for the local Ollama server. Loopback only: no proxy, no redirects, and only
//! models Ollama says run on this computer.
//!
//! Ollama can also serve "cloud" models, which it forwards to Ollama's servers. Their tags
//! usually end in `cloud`, but a model can be copied or created under any name, so the name
//! is only the first check: before any text is sent, Ollama is asked about the model
//! (`/api/show`) and anything that names a remote host or remote model is refused. So is a
//! model it cannot vouch for (an error or an answer that is not a model description): when
//! in doubt, nothing is sent.

use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Serialize)]
pub(crate) struct OllamaGenerateRequest<'a> {
    pub(crate) model: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) stream: bool,
    pub(crate) format: &'a str,
    pub(crate) keep_alive: &'a str,
    pub(crate) options: OllamaOptions,
}

#[derive(Debug, Serialize)]
pub(crate) struct OllamaOptions {
    pub(crate) temperature: f32,
    /// Fixed context window. Keeping it constant between the warm-up and every cleanup
    /// request means Ollama never reloads the model to resize it, and bounds memory.
    pub(crate) num_ctx: u32,
    /// Upper bound on generated tokens so a confused model cannot run on until the timeout.
    pub(crate) num_predict: u32,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaGenerateResponse {
    pub(crate) response: String,
}

pub(crate) const OLLAMA_MODEL: &str = "llama3.2:3b";
pub(crate) const OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";
const OLLAMA_TAGS_URL: &str = "http://127.0.0.1:11434/api/tags";
pub(crate) const OLLAMA_GENERATE_URL: &str = "http://127.0.0.1:11434/api/generate";
/// How long Ollama's answer about a model is trusted. Short, because a model can be replaced
/// under the same name (`ollama cp`); long enough that a conversation's turns, and the
/// keep-warm requests between them, do not ask each time.
const MODEL_CHECK_TTL: Duration = Duration::from_secs(60);

pub(crate) const OLLAMA_KEEP_ALIVE: &str = "30m";
/// Enough for the longest dictation (120 s is roughly 400 words) sent twice in the cleanup
/// prompt, the rules, and the reply.
pub(crate) const OLLAMA_NUM_CTX: u32 = 4096;
pub(crate) const OLLAMA_NUM_PREDICT: u32 = 1024;

pub(crate) fn request_ollama_generate(
    prompt: &str,
    timeout: Duration,
    temperature: f32,
    model: &str,
) -> Result<String, String> {
    ensure_local_model(OLLAMA_BASE_URL, model).map_err(|refusal| refusal.message(model))?;

    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        // Dictated text must only ever reach the loopback Ollama server: ignore system and
        // environment proxies and never follow a redirect to another host.
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| format!("Could not create local Ollama client: {error}"))?;

    let response = client
        .post(OLLAMA_GENERATE_URL)
        .json(&OllamaGenerateRequest {
            model,
            prompt,
            stream: false,
            format: "json",
            keep_alive: OLLAMA_KEEP_ALIVE,
            options: OllamaOptions {
                temperature,
                num_ctx: OLLAMA_NUM_CTX,
                num_predict: OLLAMA_NUM_PREDICT,
            },
        })
        .send()
        .map_err(|error| {
            format!("Could not reach local Ollama at {OLLAMA_GENERATE_URL}: {error}")
        })?;

    let status = response.status();
    if !status.is_success() {
        // The body is not included: it can echo the prompt, which holds the dictated text,
        // and this error is logged.
        if status.as_u16() == 404 {
            return Err(format!(
                "Ollama model \"{model}\" was not found locally. Run: ollama pull {model}"
            ));
        }
        return Err(format!("Ollama returned HTTP {}.", status.as_u16()));
    }

    let payload: OllamaGenerateResponse = response
        .json()
        .map_err(|error| format!("Ollama returned invalid JSON: {error}"))?;

    Ok(payload.response)
}

/// Why a model was not used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelRefusal {
    /// It runs on Ollama's servers: its tag says so, or Ollama does.
    Cloud,
    NotInstalled,
    OllamaNotRunning,
    /// Ollama's answer could not be read, so the model is not known to run here.
    Unknown,
}

impl ModelRefusal {
    /// A sentence for the user. Names the configured model, never any text.
    pub(crate) fn message(self, model: &str) -> String {
        match self {
            Self::Cloud => format!(
                "Ollama model \"{model}\" is a cloud-hosted model that runs on Ollama's servers. \
                 LocalFlow only uses models that run on this computer."
            ),
            Self::NotInstalled => {
                format!("Ollama model \"{model}\" was not found locally. Run: ollama pull {model}")
            }
            Self::OllamaNotRunning => {
                "Local Ollama is not running. Start Ollama and try again.".to_string()
            }
            Self::Unknown => format!(
                "LocalFlow could not confirm that Ollama model \"{model}\" runs on this computer, \
                 so nothing was sent to it."
            ),
        }
    }
}

type ModelVerdicts = HashMap<(String, String), (Result<(), ModelRefusal>, Instant)>;

fn verdicts() -> &'static Mutex<ModelVerdicts> {
    static VERDICTS: OnceLock<Mutex<ModelVerdicts>> = OnceLock::new();
    VERDICTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Refuse a model unless the Ollama server at `base_url` says it runs on this computer.
/// Checked before every request that carries text; a clear answer is remembered for
/// [`MODEL_CHECK_TTL`], anything else is asked again next time.
pub(crate) fn ensure_local_model(base_url: &str, model: &str) -> Result<(), ModelRefusal> {
    if is_cloud_model_tag(model) {
        return Err(ModelRefusal::Cloud);
    }
    let key = (base_url.to_string(), model.trim().to_string());
    {
        let mut known = verdicts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match known.get(&key) {
            Some((verdict, at)) if at.elapsed() < MODEL_CHECK_TTL => return *verdict,
            Some(_) => {
                known.remove(&key);
            }
            None => {}
        }
    }
    let verdict = ask_ollama_about(base_url, model);
    if matches!(verdict, Ok(()) | Err(ModelRefusal::Cloud)) {
        verdicts()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key, (verdict, Instant::now()));
    }
    verdict
}

fn ask_ollama_about(base_url: &str, model: &str) -> Result<(), ModelRefusal> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .connect_timeout(Duration::from_secs(2))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ModelRefusal::Unknown)?;
    let response = client
        .post(format!("{base_url}/api/show"))
        .json(&serde_json::json!({ "model": model }))
        .send()
        .map_err(|error| {
            if error.is_connect() {
                ModelRefusal::OllamaNotRunning
            } else {
                ModelRefusal::Unknown
            }
        })?;
    match response.status().as_u16() {
        200..=299 => {}
        404 => return Err(ModelRefusal::NotInstalled),
        _ => return Err(ModelRefusal::Unknown),
    }
    let body = response.text().map_err(|_| ModelRefusal::Unknown)?;
    model_location(&body)
}

/// Whether an `/api/show` reply describes a model that runs here. A cloud model's reply
/// carries `remote_host` and `remote_model`; a reply without the fields every model
/// description has is not taken as a yes.
fn model_location(body: &str) -> Result<(), ModelRefusal> {
    let value: Value = serde_json::from_str(body).map_err(|_| ModelRefusal::Unknown)?;
    let fields = value.as_object().ok_or(ModelRefusal::Unknown)?;
    if names_a_remote(fields) {
        return Err(ModelRefusal::Cloud);
    }
    let describes_a_model = [
        "details",
        "modelfile",
        "model_info",
        "capabilities",
        "template",
        "parameters",
    ]
    .iter()
    .any(|key| fields.contains_key(*key));
    if describes_a_model {
        Ok(())
    } else {
        Err(ModelRefusal::Unknown)
    }
}

/// Whether a model description (from `/api/show` or one `/api/tags` entry) names a remote
/// host or model.
fn names_a_remote(fields: &Map<String, Value>) -> bool {
    ["remote_host", "remote_model"]
        .iter()
        .any(|key| match fields.get(*key) {
            None | Some(Value::Null) => false,
            Some(Value::String(text)) => !text.trim().is_empty(),
            Some(_) => true,
        })
}

/// Whether an Ollama model name refers to a cloud-hosted model (for example
/// `gemma4:cloud` or `qwen3:480b-cloud`). Those run on Ollama's servers, so they are refused.
pub(crate) fn is_cloud_model_tag(model: &str) -> bool {
    let tag = model
        .trim()
        .rsplit_once(':')
        .map(|(_, tag)| tag)
        .unwrap_or("")
        .to_ascii_lowercase();
    tag == "cloud" || tag.ends_with("-cloud")
}

pub(crate) fn warm_ollama_in_background(model: String) {
    thread::spawn(move || {
        let prompt = serde_json::json!({
            "task": "localflow.warm_dictation_cleanup",
            "instruction": "Return only JSON.",
            "requiredShape": {
                "text": "ready",
                "confidence": 1.0,
                "resolved_corrections": [],
                "warnings": []
            }
        })
        .to_string();

        if let Err(error) = request_ollama_generate(&prompt, Duration::from_secs(12), 0.0, &model) {
            tracing::debug!(error = %error, model = %model, "ollama warmup skipped");
        }
    });
}

#[derive(Debug, Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<Map<String, Value>>,
}

/// Names of the models installed in local Ollama, without cloud-hosted tags (which LocalFlow
/// refuses to use). Asked from here rather than from the web UI because Ollama rejects
/// requests whose origin is the app window, and so the web UI needs no network access at all.
pub(crate) fn list_local_models() -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(4))
        .connect_timeout(Duration::from_secs(2))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create the local Ollama client.".to_string())?;
    let response = client.get(OLLAMA_TAGS_URL).send().map_err(|error| {
        if error.is_connect() {
            "Local Ollama is not running. Start Ollama and check again.".to_string()
        } else {
            "Local Ollama did not answer.".to_string()
        }
    })?;
    if !response.status().is_success() {
        return Err(format!(
            "Local Ollama returned HTTP {}.",
            response.status().as_u16()
        ));
    }
    let body = response
        .text()
        .map_err(|_| "Local Ollama sent an unreadable reply.".to_string())?;
    parse_model_names(&body)
}

/// Model names from an Ollama `/api/tags` reply, without cloud models: neither those whose
/// tag says so nor those whose entry names a remote host or model, whatever they are called.
fn parse_model_names(body: &str) -> Result<Vec<String>, String> {
    let tags: OllamaTagsResponse = serde_json::from_str(body)
        .map_err(|_| "Local Ollama sent an unreadable reply.".to_string())?;
    Ok(tags
        .models
        .into_iter()
        .filter(|entry| !names_a_remote(entry))
        .filter_map(|entry| match entry.get("name") {
            Some(Value::String(name)) => Some(name.clone()),
            _ => None,
        })
        .filter(|name| !name.trim().is_empty() && !is_cloud_model_tag(name))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_hosted_ollama_tags_are_refused() {
        assert!(is_cloud_model_tag("gemma4:cloud"));
        assert!(is_cloud_model_tag("qwen3-coder:480b-cloud"));
        assert!(is_cloud_model_tag(" Gemma4:Cloud "));
        assert!(!is_cloud_model_tag("llama3.2:3b"));
        assert!(!is_cloud_model_tag("cloudy-model:latest"));
        assert!(!is_cloud_model_tag("llama3.2"));

        let error =
            request_ollama_generate("{}", Duration::from_secs(1), 0.0, "gemma4:cloud").unwrap_err();
        assert!(error.contains("cloud-hosted"));
    }

    /// A loopback server answering one request per entry of `replies` (status line, body),
    /// in order. Returns its base URL and each request's first line.
    fn stand_in_ollama(
        replies: Vec<(&'static str, String)>,
    ) -> (String, std::sync::mpsc::Receiver<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        thread::spawn(move || {
            for (status, body) in replies {
                let Ok((mut socket, _)) = listener.accept() else {
                    return;
                };
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let read = socket.read(&mut buffer).unwrap_or(0);
                    request.extend_from_slice(&buffer[..read]);
                    let text = String::from_utf8_lossy(&request).to_string();
                    if let Some(split) = text.find("\r\n\r\n") {
                        let length = text[..split]
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if request.len() >= split + 4 + length {
                            break;
                        }
                    }
                    if read == 0 {
                        break;
                    }
                }
                let first_line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .to_string();
                let _ = seen_tx.send(first_line);
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes());
                let _ = socket.write_all(body.as_bytes());
            }
        });
        (base, seen_rx)
    }

    const LOCAL_SHOW: &str = r#"{"modelfile":"FROM x","parameters":"stop x","details":{"parent_model":"","format":"gguf","family":"llama"},"capabilities":["completion","tools"],"modified_at":"2026-07-04T09:14:55-04:00"}"#;
    const CLOUD_SHOW: &str = r#"{"remote_model":"gpt-oss:120b","remote_host":"https://ollama.com:443","details":{"format":""},"capabilities":["completion"]}"#;

    #[test]
    fn a_model_description_is_local_only_without_remote_fields() {
        assert_eq!(model_location(LOCAL_SHOW), Ok(()));
        assert_eq!(model_location(CLOUD_SHOW), Err(ModelRefusal::Cloud));
        assert_eq!(
            model_location(r#"{"remote_model":"x","details":{}}"#),
            Err(ModelRefusal::Cloud)
        );
        assert_eq!(model_location(r#"{"remote_host":"","details":{}}"#), Ok(()));
        // Fail closed: nothing recognisable is not a yes.
        assert_eq!(model_location("{}"), Err(ModelRefusal::Unknown));
        assert_eq!(model_location("[]"), Err(ModelRefusal::Unknown));
        assert_eq!(model_location("<html>"), Err(ModelRefusal::Unknown));
    }

    #[test]
    fn a_renamed_cloud_model_is_refused_and_the_answer_remembered() {
        let (base, seen) = stand_in_ollama(vec![("200 OK", CLOUD_SHOW.to_string())]);

        // No "cloud" in the name: only Ollama's answer gives it away.
        assert_eq!(
            ensure_local_model(&base, "my-assistant:latest"),
            Err(ModelRefusal::Cloud)
        );
        assert_eq!(seen.recv().unwrap(), "POST /api/show HTTP/1.1");
        // Asked once: the stand-in has no second answer, and none is needed.
        assert_eq!(
            ensure_local_model(&base, "my-assistant:latest"),
            Err(ModelRefusal::Cloud)
        );
        assert!(ModelRefusal::Cloud
            .message("my-assistant:latest")
            .contains("cloud-hosted"));
    }

    #[test]
    fn a_local_model_is_allowed_and_an_unclear_answer_is_not_remembered() {
        let (base, _seen) = stand_in_ollama(vec![
            ("500 Internal Server Error", "{}".to_string()),
            ("200 OK", "not a model".to_string()),
            ("200 OK", LOCAL_SHOW.to_string()),
            (
                "404 Not Found",
                r#"{"error":"model not found"}"#.to_string(),
            ),
        ]);

        assert_eq!(
            ensure_local_model(&base, "llama3.2:3b"),
            Err(ModelRefusal::Unknown)
        );
        assert_eq!(
            ensure_local_model(&base, "llama3.2:3b"),
            Err(ModelRefusal::Unknown)
        );
        assert_eq!(ensure_local_model(&base, "llama3.2:3b"), Ok(()));
        assert_eq!(ensure_local_model(&base, "llama3.2:3b"), Ok(()));
        assert_eq!(
            ensure_local_model(&base, "missing:1b"),
            Err(ModelRefusal::NotInstalled)
        );
    }

    #[test]
    fn a_server_that_is_not_running_refuses_the_model() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert_eq!(
            ensure_local_model(&format!("http://127.0.0.1:{port}"), "llama3.2:3b"),
            Err(ModelRefusal::OllamaNotRunning)
        );
    }

    /// Checks the fields against the Ollama installed here. Ignored by default: it needs a
    /// running Ollama with `llama3.2:3b` pulled. Read-only (`/api/show`).
    #[test]
    #[ignore = "needs a local Ollama with llama3.2:3b"]
    fn real_ollama_reports_an_installed_model_as_local() {
        assert_eq!(ensure_local_model(OLLAMA_BASE_URL, OLLAMA_MODEL), Ok(()));
    }

    #[test]
    fn installed_models_are_listed_without_cloud_tags() {
        let body = r#"{"models":[{"name":"llama3.2:3b","size":1},{"name":"gemma4:cloud"},{"name":"qwen3:480b-cloud"},{"name":"nomic-embed-text:latest"},{"name":"  "},{"name":"renamed:latest","remote_model":"gpt-oss:120b","remote_host":"https://ollama.com:443"},{"name":"half:latest","remote_host":"https://ollama.com:443"},{"size":3}]}"#;

        assert_eq!(
            parse_model_names(body).unwrap(),
            vec!["llama3.2:3b", "nomic-embed-text:latest"]
        );
        assert_eq!(
            parse_model_names(r#"{"models":[]}"#).unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(parse_model_names("{}").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn an_unreadable_model_list_is_reported_without_echoing_it() {
        let error = parse_model_names("<html>SECRET</html>").unwrap_err();

        assert!(!error.contains("SECRET"));
    }
}
