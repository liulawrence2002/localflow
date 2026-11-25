//! The text-to-speech provider boundary.
//!
//! Mirrors the speech-to-text boundary: every local voice engine implements
//! [`TextToSpeechProvider`], and the read-aloud coordinator only sees this trait. Providers
//! are strictly local. Text to be spoken is as sensitive as dictated text, so it must never
//! appear in a log or an error message.

use std::{fmt, time::Duration};

use crate::cancel::CancelFlag;

/// Mono audio produced by a voice engine, in `[-1, 1]`.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl SynthesizedAudio {
    pub fn duration(&self) -> Duration {
        if self.sample_rate == 0 {
            return Duration::ZERO;
        }
        Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}

/// Why a provider could not speak. Messages never contain the text being spoken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TtsError {
    /// The engine cannot run at all (not installed, missing model, failed to start).
    Unavailable(String),
    /// The engine did not answer within its deadline and was stopped.
    Timeout {
        provider: &'static str,
        waited: Duration,
    },
    /// The engine ran but returned an error or unusable audio.
    Failed(String),
    /// There was nothing to say after the text was cleaned up.
    NothingToSay,
    /// The request was stopped before it finished.
    Cancelled,
}

impl fmt::Display for TtsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => formatter.write_str(message),
            Self::Timeout { provider, waited } => write!(
                formatter,
                "{provider} did not finish within {} s and was stopped.",
                waited.as_secs()
            ),
            Self::NothingToSay => formatter.write_str("There is no text to read aloud."),
            Self::Cancelled => formatter.write_str("Reading aloud was stopped."),
        }
    }
}

impl std::error::Error for TtsError {}

/// A replaceable local voice engine.
pub trait TextToSpeechProvider: Send + Sync {
    /// Stable identifier, for example `fish-s2`.
    fn id(&self) -> &'static str;

    /// Human-readable description of the engine and model for diagnostics.
    fn describe(&self) -> String;

    /// Make the engine ready (start a sidecar, load the model). Idempotent.
    fn prepare(&self) -> Result<(), TtsError> {
        Ok(())
    }

    /// Turn `text` into audio, blocking until done, failed or timed out. Setting `cancel`
    /// stops the engine working on it (not just the wait for it); the call then returns
    /// [`TtsError::Cancelled`] soon after.
    fn synthesize(&self, text: &str, cancel: &CancelFlag) -> Result<SynthesizedAudio, TtsError>;

    /// Release resources (stop a sidecar, free the model). Idempotent.
    fn shutdown(&self) {}
}

/// Longest text accepted for one read-aloud request, in characters. Longer text is cut at a
/// sentence boundary where possible.
pub const MAX_SPEECH_CHARS: usize = 1_500;

/// Clean text for a voice engine: drop control characters, collapse whitespace, and bound
/// the length. Returns `None` when nothing speakable is left.
pub fn prepare_text_for_speech(text: &str) -> Option<String> {
    let collapsed = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    if !collapsed.chars().any(char::is_alphanumeric) {
        return None;
    }
    if collapsed.chars().count() <= MAX_SPEECH_CHARS {
        return Some(collapsed);
    }

    let bounded: String = collapsed.chars().take(MAX_SPEECH_CHARS).collect();
    // Prefer ending on a sentence; fall back to the last whole word.
    let cut = bounded
        .rfind(['.', '!', '?'])
        .map(|index| index + 1)
        .filter(|index| *index >= MAX_SPEECH_CHARS / 2)
        .or_else(|| bounded.rfind(' '))
        .unwrap_or(bounded.len());
    Some(bounded[..cut].trim_end().to_string())
}

/// Deadline for synthesising `text`. Large autoregressive voices can run slower than real
/// time on CPU, so the allowance is generous but still bounded.
pub fn synthesis_deadline(text: &str) -> Duration {
    const BASE: Duration = Duration::from_secs(60);
    const PER_CHAR: Duration = Duration::from_millis(600);
    const MAX: Duration = Duration::from_secs(900);
    (BASE + PER_CHAR * text.chars().count() as u32).min(MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_collapsed_and_control_characters_are_removed() {
        assert_eq!(
            prepare_text_for_speech("  Hello,\r\n\tworld.\u{0007}  ").as_deref(),
            Some("Hello, world.")
        );
    }

    #[test]
    fn text_without_anything_speakable_is_rejected() {
        assert_eq!(prepare_text_for_speech(""), None);
        assert_eq!(prepare_text_for_speech(" \n\t "), None);
        assert_eq!(prepare_text_for_speech("... --- !!!"), None);
    }

    #[test]
    fn long_text_is_cut_at_a_sentence_boundary_within_the_limit() {
        let sentence = "This is a sentence of moderate length for testing. ";
        let long = sentence.repeat(60);

        let prepared = prepare_text_for_speech(&long).unwrap();

        assert!(prepared.chars().count() <= MAX_SPEECH_CHARS);
        assert!(prepared.ends_with('.'));
        assert!(prepared.chars().count() > MAX_SPEECH_CHARS / 2);
    }

    #[test]
    fn long_text_without_sentences_is_cut_at_a_word_boundary() {
        let long = "word ".repeat(1_000);

        let prepared = prepare_text_for_speech(&long).unwrap();

        assert!(prepared.chars().count() <= MAX_SPEECH_CHARS);
        assert!(prepared.ends_with("word"));
    }

    #[test]
    fn multibyte_text_is_cut_without_splitting_a_character() {
        let long = "héllo wörld ".repeat(400);
        let prepared = prepare_text_for_speech(&long).unwrap();
        assert!(prepared.chars().count() <= MAX_SPEECH_CHARS);
    }

    #[test]
    fn deadline_grows_with_text_and_is_capped() {
        assert_eq!(synthesis_deadline(""), Duration::from_secs(60));
        assert_eq!(
            synthesis_deadline(&"a".repeat(100)),
            Duration::from_secs(120)
        );
        assert_eq!(
            synthesis_deadline(&"a".repeat(10_000)),
            Duration::from_secs(900)
        );
    }

    #[test]
    fn audio_duration_is_samples_over_rate() {
        let audio = SynthesizedAudio {
            samples: vec![0.0; 22_050],
            sample_rate: 44_100,
        };
        assert_eq!(audio.duration(), Duration::from_millis(500));
        assert_eq!(
            SynthesizedAudio {
                samples: vec![0.0; 4],
                sample_rate: 0
            }
            .duration(),
            Duration::ZERO
        );
    }

    #[test]
    fn errors_never_need_the_spoken_text() {
        let timeout = TtsError::Timeout {
            provider: "fish-s2",
            waited: Duration::from_secs(61),
        };
        assert_eq!(
            timeout.to_string(),
            "fish-s2 did not finish within 61 s and was stopped."
        );
        assert_eq!(
            TtsError::NothingToSay.to_string(),
            "There is no text to read aloud."
        );
    }
}
