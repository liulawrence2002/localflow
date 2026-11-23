//! The streaming voice boundary used by conversation mode.
//!
//! [`super::provider::TextToSpeechProvider`] returns a whole clip, which suits read-aloud
//! with a slow, high-quality voice. A conversation needs audio as soon as the first of it
//! exists and must be able to stop mid-sentence, so this trait hands audio out in pieces and
//! takes a cancel signal.

pub use crate::cancel::CancelFlag;

use super::provider::TtsError;

/// A local voice engine that produces audio incrementally.
pub trait StreamingTextToSpeech: Send + Sync {
    /// Stable identifier, for example `phonon`.
    fn id(&self) -> &'static str;

    /// Sample rate of the mono audio handed to `on_audio`, in Hz.
    fn sample_rate(&self) -> u32;

    /// Load the model so the first request does not pay for it. Idempotent.
    fn prepare(&self) -> Result<(), TtsError>;

    /// Speak `text`, calling `on_audio` with each piece of mono audio in `[-1, 1]` as it is
    /// generated. Returns when the text is finished or soon after `cancel` is set; a
    /// cancelled request is not an error. Implementations must not log or return the text.
    fn synthesize_stream(
        &self,
        text: &str,
        cancel: &CancelFlag,
        on_audio: &mut dyn FnMut(&[f32]),
    ) -> Result<(), TtsError>;

    /// Free the model. Idempotent.
    fn shutdown(&self) {}
}

#[cfg(test)]
pub mod mock {
    //! A scripted voice for tests: a fixed amount of audio per character, optional delays and
    //! failures, and a record of what it was asked to say.

    use std::{sync::Mutex, thread, time::Duration};

    use super::*;

    pub struct MockVoice {
        pub sample_rate: u32,
        /// Samples produced per character of text.
        pub samples_per_char: usize,
        /// Samples per call to `on_audio`.
        pub chunk_samples: usize,
        /// Pause before each chunk, to give cancellation something to interrupt.
        pub chunk_delay: Duration,
        pub fail_with: Option<TtsError>,
        pub spoken: Mutex<Vec<String>>,
    }

    impl Default for MockVoice {
        fn default() -> Self {
            Self {
                sample_rate: 16_000,
                samples_per_char: 160,
                chunk_samples: 320,
                chunk_delay: Duration::ZERO,
                fail_with: None,
                spoken: Mutex::new(Vec::new()),
            }
        }
    }

    impl StreamingTextToSpeech for MockVoice {
        fn id(&self) -> &'static str {
            "mock-voice"
        }

        fn sample_rate(&self) -> u32 {
            self.sample_rate
        }

        fn prepare(&self) -> Result<(), TtsError> {
            Ok(())
        }

        fn synthesize_stream(
            &self,
            text: &str,
            cancel: &CancelFlag,
            on_audio: &mut dyn FnMut(&[f32]),
        ) -> Result<(), TtsError> {
            if let Some(error) = &self.fail_with {
                return Err(error.clone());
            }
            self.spoken.lock().unwrap().push(text.to_string());

            let mut remaining = text.chars().count() * self.samples_per_char;
            let chunk = vec![0.25f32; self.chunk_samples];
            while remaining > 0 {
                if cancel.is_cancelled() {
                    return Ok(());
                }
                if !self.chunk_delay.is_zero() {
                    thread::sleep(self.chunk_delay);
                }
                let take = remaining.min(self.chunk_samples);
                on_audio(&chunk[..take]);
                remaining -= take;
            }
            Ok(())
        }
    }

    #[test]
    fn the_mock_voice_stops_when_cancelled() {
        let voice = MockVoice::default();
        let cancel = CancelFlag::default();
        let mut pieces = 0;

        voice
            .synthesize_stream("a long enough sentence", &cancel, &mut |_| {
                pieces += 1;
                cancel.cancel();
            })
            .unwrap();

        assert_eq!(pieces, 1);
    }
}
