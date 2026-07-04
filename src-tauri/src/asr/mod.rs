use serde::{Deserialize, Serialize};

pub mod metrics;
pub mod stabilizer;
pub mod streaming;
pub mod windows;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AsrSegment {
    pub start_ms: u32,
    pub end_ms: u32,
    pub text: String,
}

pub trait AsrProvider: Send + Sync {
    fn transcribe(&self) -> Result<String, String>;
}

#[derive(Debug, Clone)]
pub struct MockAsrProvider {
    transcript: String,
}

impl MockAsrProvider {
    pub fn new(transcript: String) -> Self {
        Self { transcript }
    }
}

impl AsrProvider for MockAsrProvider {
    fn transcribe(&self) -> Result<String, String> {
        Ok(self.transcript.clone())
    }
}

#[derive(Debug, Clone)]
pub struct WhisperCppProviderConfig {
    pub model_path: String,
    pub language: String,
    pub threads: u8,
    pub initial_prompt: String,
    pub timeout_ms: u64,
}
