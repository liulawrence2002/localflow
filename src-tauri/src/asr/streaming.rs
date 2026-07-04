//! Streaming ASR event contract and provider interface (spec §3.2 / §4.3).
//!
//! This is the Phase 3 foundation: a typed event stream modeled on the public real-time ASR
//! interaction pattern (session -> speech-started -> partial/committed -> final), a provider
//! trait so engines (persistent whisper.cpp, sherpa-onnx, or the current one-shot CLI wrapped
//! as a batch provider) are interchangeable, and a `StreamingSession` coordinator that turns
//! rolling-window decodes into that event stream without duplicating committed words.
//!
//! The public surface here is exercised by the unit tests below and will be wired into the
//! native dictation coordinator when a persistent runtime lands. It is intentionally not on
//! the default one-shot path yet, so `dead_code` is allowed at the module level.
#![allow(dead_code)]

use super::stabilizer::TranscriptStabilizer;

pub type SessionId = u64;

/// What an ASR provider can actually do. Callers must not assume timestamps, language
/// detection, keyterm biasing, or streaming unless the provider advertises them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AsrCapabilities {
    pub streaming: bool,
    pub partials: bool,
    pub timestamps: bool,
    pub language_detection: bool,
    pub keyterms: bool,
}

impl AsrCapabilities {
    /// Capabilities of the current one-shot whisper CLI wrapped as a batch provider.
    pub fn batch_only() -> Self {
        Self {
            streaming: false,
            partials: false,
            timestamps: false,
            language_detection: false,
            keyterms: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AsrSessionConfig {
    pub sample_rate: u32,
    pub language: Option<String>,
    pub keyterms: Vec<String>,
}

/// The internal streaming ASR event stream. Every event carries the `session_id` so stale
/// events can be rejected at each boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum AsrEvent {
    SessionStarted {
        session_id: SessionId,
        sample_rate: u32,
    },
    SpeechStarted {
        session_id: SessionId,
        at_ms: u64,
    },
    Partial {
        session_id: SessionId,
        revision: u64,
        text: String,
    },
    Committed {
        session_id: SessionId,
        segment_id: u64,
        text: String,
    },
    SpeechEnded {
        session_id: SessionId,
        at_ms: u64,
    },
    Final {
        session_id: SessionId,
        text: String,
    },
    Warning {
        session_id: SessionId,
        code: String,
        message: String,
    },
    Failed {
        session_id: SessionId,
        code: String,
        message: String,
    },
}

/// A replaceable streaming ASR engine.
pub trait StreamingAsrProvider: Send {
    fn capabilities(&self) -> AsrCapabilities;
    fn start(&mut self, config: AsrSessionConfig) -> Result<SessionId, String>;
    fn push_audio(&mut self, pcm: &[f32]) -> Result<Vec<AsrEvent>, String>;
    fn commit(&mut self) -> Result<Vec<AsrEvent>, String>;
    fn cancel(&mut self) -> Result<(), String>;
}

/// Turns successive full-utterance hypotheses (from rolling-window decodes) into a clean
/// event stream: `SpeechStarted` once, `Committed` for each newly stable chunk, `Partial`
/// for the tentative tail, then `SpeechEnded` + `Final` on finalize. The stabilizer
/// guarantees committed words are never repeated even though windows overlap.
pub struct StreamingSession {
    session_id: SessionId,
    stabilizer: TranscriptStabilizer,
    revision: u64,
    committed_segments: u64,
    speech_started: bool,
}

impl StreamingSession {
    /// Start a session, returning the `SessionStarted` event.
    pub fn start(session_id: SessionId, sample_rate: u32) -> (Self, AsrEvent) {
        (
            Self {
                session_id,
                stabilizer: TranscriptStabilizer::new(),
                revision: 0,
                committed_segments: 0,
                speech_started: false,
            },
            AsrEvent::SessionStarted {
                session_id,
                sample_rate,
            },
        )
    }

    /// Observe a fresh hypothesis decoded over all audio so far.
    pub fn observe_hypothesis(&mut self, hypothesis: &str, at_ms: u64) -> Vec<AsrEvent> {
        let mut events = Vec::new();

        if !self.speech_started && !hypothesis.trim().is_empty() {
            self.speech_started = true;
            events.push(AsrEvent::SpeechStarted {
                session_id: self.session_id,
                at_ms,
            });
        }

        let update = self.stabilizer.update(hypothesis);
        if !update.new_commit.is_empty() {
            self.committed_segments += 1;
            events.push(AsrEvent::Committed {
                session_id: self.session_id,
                segment_id: self.committed_segments,
                text: update.new_commit,
            });
        }
        if !update.provisional_text.is_empty() {
            self.revision += 1;
            events.push(AsrEvent::Partial {
                session_id: self.session_id,
                revision: self.revision,
                text: update.provisional_text,
            });
        }

        events
    }

    /// Finalize when recording stops: commit the remainder, then `SpeechEnded` + `Final`.
    pub fn finalize(&mut self, hypothesis: &str, at_ms: u64) -> Vec<AsrEvent> {
        let mut events = Vec::new();
        let update = self.stabilizer.finalize(hypothesis);

        if !update.new_commit.is_empty() {
            self.committed_segments += 1;
            events.push(AsrEvent::Committed {
                session_id: self.session_id,
                segment_id: self.committed_segments,
                text: update.new_commit,
            });
        }
        events.push(AsrEvent::SpeechEnded {
            session_id: self.session_id,
            at_ms,
        });
        events.push(AsrEvent::Final {
            session_id: self.session_id,
            text: update.committed_text,
        });

        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_emits_started_then_speech_committed_partial() {
        let (mut session, started) = StreamingSession::start(7, 16_000);
        assert_eq!(
            started,
            AsrEvent::SessionStarted {
                session_id: 7,
                sample_rate: 16_000
            }
        );

        // First hypothesis: speech starts, nothing stable yet -> a partial.
        let events = session.observe_hypothesis("hello", 100);
        assert!(matches!(
            events[0],
            AsrEvent::SpeechStarted { at_ms: 100, .. }
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, AsrEvent::Partial { .. })));

        // Second hypothesis agrees on "hello" -> it commits, "world" is tentative.
        let events = session.observe_hypothesis("hello world", 200);
        let committed: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AsrEvent::Committed {
                    text, segment_id, ..
                } => Some((*segment_id, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(committed, vec![(1, "hello".to_string())]);
    }

    #[test]
    fn finalize_emits_speech_ended_and_final_without_duplicates() {
        let (mut session, _) = StreamingSession::start(1, 16_000);
        session.observe_hypothesis("the quick", 100);
        session.observe_hypothesis("the quick brown", 200);

        let events = session.finalize("the quick brown fox", 300);
        assert!(events
            .iter()
            .any(|event| matches!(event, AsrEvent::SpeechEnded { .. })));
        let final_text = events.iter().find_map(|event| match event {
            AsrEvent::Final { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(final_text, Some("the quick brown fox".to_string()));
    }

    /// A scripted provider proving the trait is usable end to end without a real engine.
    struct MockStreamingProvider {
        hypotheses: Vec<String>,
        cursor: usize,
        session: Option<StreamingSession>,
        next_id: SessionId,
    }

    impl StreamingAsrProvider for MockStreamingProvider {
        fn capabilities(&self) -> AsrCapabilities {
            AsrCapabilities {
                streaming: true,
                partials: true,
                timestamps: false,
                language_detection: false,
                keyterms: false,
            }
        }

        fn start(&mut self, config: AsrSessionConfig) -> Result<SessionId, String> {
            self.next_id += 1;
            let (session, _started) = StreamingSession::start(self.next_id, config.sample_rate);
            self.session = Some(session);
            self.cursor = 0;
            Ok(self.next_id)
        }

        fn push_audio(&mut self, _pcm: &[f32]) -> Result<Vec<AsrEvent>, String> {
            let session = self.session.as_mut().ok_or("no session")?;
            let hypothesis = self
                .hypotheses
                .get(self.cursor)
                .cloned()
                .unwrap_or_default();
            self.cursor += 1;
            Ok(session.observe_hypothesis(&hypothesis, self.cursor as u64 * 100))
        }

        fn commit(&mut self) -> Result<Vec<AsrEvent>, String> {
            let last = self.hypotheses.last().cloned().unwrap_or_default();
            let session = self.session.as_mut().ok_or("no session")?;
            Ok(session.finalize(&last, 9_999))
        }

        fn cancel(&mut self) -> Result<(), String> {
            self.session = None;
            Ok(())
        }
    }

    #[test]
    fn mock_provider_drives_the_streaming_contract() {
        let mut provider = MockStreamingProvider {
            hypotheses: vec![
                "hello".to_string(),
                "hello world".to_string(),
                "hello world today".to_string(),
            ],
            cursor: 0,
            session: None,
            next_id: 0,
        };
        assert!(provider.capabilities().streaming);

        let id = provider
            .start(AsrSessionConfig {
                sample_rate: 16_000,
                language: None,
                keyterms: Vec::new(),
            })
            .unwrap();
        assert_eq!(id, 1);

        for _ in 0..3 {
            provider.push_audio(&[0.0; 16]).unwrap();
        }
        let final_events = provider.commit().unwrap();
        let final_text = final_events.iter().find_map(|event| match event {
            AsrEvent::Final { text, .. } => Some(text.clone()),
            _ => None,
        });
        assert_eq!(final_text, Some("hello world today".to_string()));
    }
}
