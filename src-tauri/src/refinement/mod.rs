use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefinementOutput {
    pub text: String,
    pub confidence: f32,
    pub resolved_corrections: Vec<String>,
    pub warnings: Vec<String>,
}

pub trait RefinementProvider: Send + Sync {
    fn refine(&self, transcript: &str) -> Result<RefinementOutput, String>;
}

pub struct MockRefinementProvider;

impl RefinementProvider for MockRefinementProvider {
    fn refine(&self, transcript: &str) -> Result<RefinementOutput, String> {
        let mut chars = transcript.trim().chars();
        let first = chars
            .next()
            .map(|character| character.to_uppercase().to_string())
            .unwrap_or_default();
        let rest = chars.collect::<String>();
        let mut text = format!("{first}{rest}");

        if !matches!(text.chars().last(), Some('.') | Some('!') | Some('?')) {
            text.push('.');
        }

        Ok(RefinementOutput {
            text,
            confidence: 0.82,
            resolved_corrections: Vec::new(),
            warnings: vec!["Mock provider used; no local LLM was contacted.".to_string()],
        })
    }
}

pub struct NoOpRefinementProvider;

impl RefinementProvider for NoOpRefinementProvider {
    fn refine(&self, transcript: &str) -> Result<RefinementOutput, String> {
        Ok(RefinementOutput {
            text: transcript.to_string(),
            confidence: 1.0,
            resolved_corrections: Vec::new(),
            warnings: Vec::new(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct OllamaRefinementProviderConfig {
    pub base_url: String,
    pub model: String,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone)]
pub struct LlamaCppRefinementProviderConfig {
    pub base_url: String,
    pub model_alias: String,
    pub timeout_ms: u64,
}
