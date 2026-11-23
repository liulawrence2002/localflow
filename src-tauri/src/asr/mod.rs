use serde::{Deserialize, Serialize};

pub mod engine_select;
pub mod metrics;
pub mod provider;
pub mod sidecar;
pub mod stabilizer;
pub mod streaming;
pub mod transcribe_cpp;
pub mod whisper_cli;
pub mod whisper_server;
pub mod whisper_text;
pub mod windows;

/// Decoding settings shared by every whisper.cpp-backed provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhisperDecodeOptions {
    /// CPU threads for whisper.cpp.
    pub threads: usize,
    /// Beam width. `1` is greedy decoding, which is the fastest; whisper.cpp's CLI default is 5.
    pub beam_size: u32,
    /// How much of Whisper's fixed 30 second encoder window to compute.
    pub audio_context: AudioContext,
}

impl Default for WhisperDecodeOptions {
    fn default() -> Self {
        Self {
            threads: sidecar::default_asr_thread_count(),
            beam_size: 1,
            audio_context: AudioContext::Full,
        }
    }
}

/// Whisper's encoder always processes a 30 second window (1500 frames of 20 ms), however
/// short the audio is. `Dynamic` shrinks the window to the utterance plus a safety margin,
/// which cuts encoder time on short dictations. It is a non-standard truncation and it costs
/// accuracy: on the benchmark corpus a tight margin took base.en from 2.42 % to 5.98 % WER,
/// and even the wide margin used here gave 2.97 %. The live path therefore uses `Full`;
/// `Dynamic` exists so the trade-off can be re-measured with `examples/asr_bench.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioContext {
    Full,
    Dynamic,
}

/// Frames in Whisper's full encoder window.
pub const WHISPER_FULL_AUDIO_CTX: u32 = 1500;
/// Extra frames kept beyond the audio length under [`AudioContext::Dynamic`] (10 s).
const DYNAMIC_AUDIO_CTX_MARGIN: u32 = 500;
/// Never go below this many frames (15 s); smaller windows caused repetition loops.
const DYNAMIC_AUDIO_CTX_FLOOR: u32 = 750;

/// The dynamic audio-context constants, recorded in benchmark output so a result can be tied
/// to the margin that produced it.
pub fn describe_dynamic_audio_context() -> String {
    format!(
        "audio frames + {DYNAMIC_AUDIO_CTX_MARGIN}, floor {DYNAMIC_AUDIO_CTX_FLOOR}, \
         full window {WHISPER_FULL_AUDIO_CTX}"
    )
}

impl AudioContext {
    /// The `audio_ctx` value to send for `audio`, or `None` to use the full window.
    pub fn frames_for(self, audio: std::time::Duration) -> Option<u32> {
        match self {
            Self::Full => None,
            Self::Dynamic => {
                let needed = (audio.as_secs_f64() * 50.0).ceil() as u32 + DYNAMIC_AUDIO_CTX_MARGIN;
                let frames = needed.max(DYNAMIC_AUDIO_CTX_FLOOR);
                (frames < WHISPER_FULL_AUDIO_CTX).then_some(frames)
            }
        }
    }
}

/// File name of a model path for diagnostics. Never the full path, which can reveal a user
/// name.
pub fn model_label(model_path: &std::path::Path) -> String {
    model_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown model".to_string())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn full_audio_context_never_sends_a_value() {
        assert_eq!(AudioContext::Full.frames_for(Duration::from_secs(3)), None);
    }

    #[test]
    fn dynamic_audio_context_has_a_floor_a_margin_and_a_ceiling() {
        // Short clips are held at the floor.
        assert_eq!(
            AudioContext::Dynamic.frames_for(Duration::from_secs(2)),
            Some(750)
        );
        // 10 s of audio needs 500 frames plus the 500-frame margin.
        assert_eq!(
            AudioContext::Dynamic.frames_for(Duration::from_secs(10)),
            Some(1000)
        );
        // Near or past the full window the default is used.
        assert_eq!(
            AudioContext::Dynamic.frames_for(Duration::from_secs(25)),
            None
        );
        assert_eq!(
            AudioContext::Dynamic.frames_for(Duration::from_secs(60)),
            None
        );
    }

    #[test]
    fn model_label_is_the_file_name_only() {
        let label = model_label(std::path::Path::new(
            "C:/Users/someone/models/ggml-base.en.bin",
        ));
        assert_eq!(label, "ggml-base.en.bin");
    }
}
