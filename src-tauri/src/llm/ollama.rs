//! Client for the local Ollama server. Loopback only: no proxy, no redirects, and
//! cloud-hosted model tags are refused.

use std::{thread, time::Duration};

use serde::{Deserialize, Serialize};

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
const OLLAMA_TAGS_URL: &str = "http://127.0.0.1:11434/api/tags";
pub(crate) const OLLAMA_GENERATE_URL: &str = "http://127.0.0.1:11434/api/generate";

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
    if is_cloud_model_tag(model) {
        return Err(format!(
            "Ollama model \"{model}\" is a cloud-hosted tag. LocalFlow only uses local models."
        ));
    }

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
        let body = response.text().unwrap_or_default();
        if status.as_u16() == 404 {
            return Err(format!(
                "Ollama model \"{model}\" was not found locally. {body}"
            ));
        }
        return Err(format!("Ollama returned HTTP {status}: {body}"));
    }

    let payload: OllamaGenerateResponse = response
        .json()
        .map_err(|error| format!("Ollama returned invalid JSON: {error}"))?;

    Ok(payload.response)
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
    models: Vec<OllamaTag>,
}

#[derive(Debug, Deserialize)]
struct OllamaTag {
    name: String,
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

/// Model names from an Ollama `/api/tags` reply, cloud-hosted tags removed.
fn parse_model_names(body: &str) -> Result<Vec<String>, String> {
    let tags: OllamaTagsResponse = serde_json::from_str(body)
        .map_err(|_| "Local Ollama sent an unreadable reply.".to_string())?;
    Ok(tags
        .models
        .into_iter()
        .map(|tag| tag.name)
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

    #[test]
    fn installed_models_are_listed_without_cloud_tags() {
        let body = r#"{"models":[{"name":"llama3.2:3b","size":1},{"name":"gemma4:cloud"},{"name":"qwen3:480b-cloud"},{"name":"nomic-embed-text:latest"},{"name":"  "}]}"#;

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
