//! The speech-to-text provider boundary used by the live dictation path.
//!
//! Every local engine (the persistent whisper.cpp server, the one-shot whisper.cpp CLI, or a
//! future Parakeet / sherpa-onnx engine) implements [`SpeechToTextProvider`]. The dictation
//! coordinator only ever sees this trait, so engines can be added, swapped, or chained without
//! touching capture, formatting, or insertion code.
//!
//! Providers are strictly local. Nothing behind this trait may contact a remote host; a
//! fallback is always another local engine, never a cloud service.

use std::{fmt, time::Duration};

/// One utterance to transcribe. Audio is always mono 16 kHz in `[-1, 1]`.
#[derive(Debug, Clone, Copy)]
pub struct TranscriptionRequest<'a> {
    pub samples: &'a [f32],
    /// ISO language code, for example `en`.
    pub language: &'a str,
    /// Optional vocabulary-biasing prompt built from the user's dictionary.
    pub initial_prompt: Option<&'a str>,
}

impl TranscriptionRequest<'_> {
    pub fn audio_duration(&self) -> Duration {
        Duration::from_secs_f64(
            self.samples.len() as f64 / crate::audio::resample::ASR_SAMPLE_RATE as f64,
        )
    }
}

/// A finished transcription plus which engine produced it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcription {
    pub text: String,
    /// Stable identifier of the engine that produced the text, for diagnostics.
    pub provider: &'static str,
    /// Set when an earlier engine in a chain failed and this one took over. Holds a short,
    /// transcript-free reason suitable for logs and diagnostics.
    pub fallback_reason: Option<String>,
}

/// Why a provider could not transcribe. Messages never contain transcript text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsrError {
    /// The engine cannot run at all (missing binary or model, failed to start).
    Unavailable(String),
    /// The engine did not answer within its deadline and was stopped.
    Timeout {
        provider: &'static str,
        waited: Duration,
    },
    /// The engine ran but returned an error or unusable output.
    Failed(String),
}

impl fmt::Display for AsrError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable(message) | Self::Failed(message) => formatter.write_str(message),
            Self::Timeout { provider, waited } => write!(
                formatter,
                "{provider} did not finish within {} ms and was stopped.",
                waited.as_millis()
            ),
        }
    }
}

impl std::error::Error for AsrError {}

/// A replaceable local speech-to-text engine.
pub trait SpeechToTextProvider: Send + Sync {
    /// Stable identifier, for example `whisper-server`.
    fn id(&self) -> &'static str;

    /// Human-readable description of the engine and model for diagnostics.
    fn describe(&self) -> String;

    /// Make the engine ready (start a sidecar, load the model). Idempotent and safe to call
    /// from a background thread while the user is still speaking. The default does nothing.
    fn prepare(&self) -> Result<(), AsrError> {
        Ok(())
    }

    /// Make the engine ready and pay any one-time first-run cost (GPU shader compiles, first
    /// touch of the weights) with a short throw-away transcription, so the next real phrase
    /// is fast. Idempotent. Meant for the start of a conversation. The default is `prepare`.
    fn warm_up(&self) -> Result<(), AsrError> {
        self.prepare()
    }

    /// Transcribe one utterance, blocking until done, failed, or timed out.
    fn transcribe(&self, request: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError>;

    /// Release resources (stop a sidecar). Idempotent. The default does nothing.
    fn shutdown(&self) {}
}

/// Deadline for transcribing `audio`: a fixed allowance for model load and scheduling plus a
/// multiple of the audio length, capped so a hung engine can never block dictation for long.
pub fn transcription_deadline(audio: Duration) -> Duration {
    const BASE: Duration = Duration::from_secs(20);
    const MAX: Duration = Duration::from_secs(180);
    (BASE + audio * 4).min(MAX)
}

/// Tries `primary`, and if it fails for any reason, tries `fallback`. Both are local engines.
pub struct FallbackProvider {
    primary: Box<dyn SpeechToTextProvider>,
    fallback: Box<dyn SpeechToTextProvider>,
}

impl FallbackProvider {
    pub fn new(
        primary: Box<dyn SpeechToTextProvider>,
        fallback: Box<dyn SpeechToTextProvider>,
    ) -> Self {
        Self { primary, fallback }
    }
}

impl SpeechToTextProvider for FallbackProvider {
    fn id(&self) -> &'static str {
        self.primary.id()
    }

    fn describe(&self) -> String {
        format!(
            "{} (falls back to {})",
            self.primary.describe(),
            self.fallback.describe()
        )
    }

    fn prepare(&self) -> Result<(), AsrError> {
        // A primary that cannot start is not fatal: transcription will use the fallback.
        match self.primary.prepare() {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                tracing::warn!(
                    provider = self.primary.id(),
                    error = %primary_error,
                    "primary ASR provider could not be prepared; fallback remains available"
                );
                self.fallback.prepare()
            }
        }
    }

    fn warm_up(&self) -> Result<(), AsrError> {
        match self.primary.warm_up() {
            Ok(()) => Ok(()),
            Err(primary_error) => {
                tracing::warn!(
                    provider = self.primary.id(),
                    error = %primary_error,
                    "primary ASR provider could not be warmed up; fallback remains available"
                );
                self.fallback.prepare()
            }
        }
    }

    fn transcribe(&self, request: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
        let primary_error = match self.primary.transcribe(request) {
            Ok(transcription) => return Ok(transcription),
            Err(error) => error,
        };

        tracing::warn!(
            provider = self.primary.id(),
            fallback = self.fallback.id(),
            error = %primary_error,
            "primary ASR provider failed; using local fallback"
        );

        match self.fallback.transcribe(request) {
            Ok(mut transcription) => {
                transcription.fallback_reason =
                    Some(format!("{} failed: {primary_error}", self.primary.id()));
                Ok(transcription)
            }
            Err(fallback_error) => Err(AsrError::Failed(format!(
                "{} failed ({primary_error}); {} also failed ({fallback_error})",
                self.primary.id(),
                self.fallback.id()
            ))),
        }
    }

    fn shutdown(&self) {
        self.primary.shutdown();
        self.fallback.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    #[derive(Default)]
    struct Counters {
        calls: AtomicUsize,
        shutdowns: AtomicUsize,
        warm_ups: AtomicUsize,
    }

    struct Scripted {
        id: &'static str,
        result: Result<&'static str, AsrError>,
        counters: Arc<Counters>,
    }

    impl Scripted {
        fn boxed(
            id: &'static str,
            result: Result<&'static str, AsrError>,
        ) -> (Box<dyn SpeechToTextProvider>, Arc<Counters>) {
            let counters = Arc::new(Counters::default());
            (
                Box::new(Self {
                    id,
                    result,
                    counters: counters.clone(),
                }),
                counters,
            )
        }
    }

    impl SpeechToTextProvider for Scripted {
        fn id(&self) -> &'static str {
            self.id
        }

        fn describe(&self) -> String {
            self.id.to_string()
        }

        fn transcribe(&self, _: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
            self.counters.calls.fetch_add(1, Ordering::SeqCst);
            self.result.clone().map(|text| Transcription {
                text: text.to_string(),
                provider: self.id,
                fallback_reason: None,
            })
        }

        fn shutdown(&self) {
            self.counters.shutdowns.fetch_add(1, Ordering::SeqCst);
        }

        fn warm_up(&self) -> Result<(), AsrError> {
            self.counters.warm_ups.fetch_add(1, Ordering::SeqCst);
            self.result.clone().map(|_| ())
        }
    }

    /// A provider that keeps the trait's default `warm_up`, which must fall through to
    /// `prepare`.
    struct PrepareOnly(Arc<Counters>);

    impl SpeechToTextProvider for PrepareOnly {
        fn id(&self) -> &'static str {
            "prepare-only"
        }

        fn describe(&self) -> String {
            self.id().to_string()
        }

        fn prepare(&self) -> Result<(), AsrError> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn transcribe(&self, _: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
            Err(AsrError::Failed("unused".to_string()))
        }
    }

    #[test]
    fn warm_up_defaults_to_prepare() {
        let counters = Arc::new(Counters::default());

        PrepareOnly(counters.clone()).warm_up().unwrap();

        assert_eq!(counters.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn warm_up_reaches_the_primary_and_only_prepares_the_fallback_when_it_fails() {
        let (primary, primary_counters) = Scripted::boxed("primary", Ok("a"));
        let (fallback, fallback_counters) = Scripted::boxed("fallback", Ok("b"));
        FallbackProvider::new(primary, fallback).warm_up().unwrap();
        assert_eq!(primary_counters.warm_ups.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_counters.warm_ups.load(Ordering::SeqCst), 0);

        let (primary, _) = Scripted::boxed(
            "primary",
            Err(AsrError::Unavailable("no model".to_string())),
        );
        let (fallback, fallback_counters) = Scripted::boxed("fallback", Ok("b"));
        // The fallback is prepared (its default `prepare` succeeds), not warmed up.
        FallbackProvider::new(primary, fallback).warm_up().unwrap();
        assert_eq!(fallback_counters.warm_ups.load(Ordering::SeqCst), 0);
    }

    fn request() -> TranscriptionRequest<'static> {
        TranscriptionRequest {
            samples: &[0.0; 16],
            language: "en",
            initial_prompt: None,
        }
    }

    #[test]
    fn uses_the_primary_when_it_succeeds_and_never_calls_the_fallback() {
        let (primary, _) = Scripted::boxed("primary", Ok("hello"));
        let (fallback, fallback_counters) = Scripted::boxed("fallback", Ok("unused"));
        let chain = FallbackProvider::new(primary, fallback);

        let result = chain.transcribe(&request()).unwrap();

        assert_eq!(result.text, "hello");
        assert_eq!(result.provider, "primary");
        assert_eq!(result.fallback_reason, None);
        assert_eq!(fallback_counters.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn falls_back_and_records_a_reason_when_the_primary_fails() {
        let (primary, _) = Scripted::boxed(
            "primary",
            Err(AsrError::Unavailable("sidecar missing".to_string())),
        );
        let (fallback, _) = Scripted::boxed("fallback", Ok("hello"));
        let chain = FallbackProvider::new(primary, fallback);

        let result = chain.transcribe(&request()).unwrap();

        assert_eq!(result.text, "hello");
        assert_eq!(result.provider, "fallback");
        assert_eq!(
            result.fallback_reason.as_deref(),
            Some("primary failed: sidecar missing")
        );
    }

    #[test]
    fn falls_back_after_a_timeout() {
        let (primary, _) = Scripted::boxed(
            "primary",
            Err(AsrError::Timeout {
                provider: "primary",
                waited: Duration::from_secs(1),
            }),
        );
        let (fallback, _) = Scripted::boxed("fallback", Ok("hello"));
        let chain = FallbackProvider::new(primary, fallback);

        assert_eq!(chain.transcribe(&request()).unwrap().provider, "fallback");
    }

    #[test]
    fn reports_both_failures_when_every_engine_fails() {
        let (primary, _) = Scripted::boxed("primary", Err(AsrError::Failed("boom".to_string())));
        let (fallback, _) = Scripted::boxed(
            "fallback",
            Err(AsrError::Unavailable("no model".to_string())),
        );
        let chain = FallbackProvider::new(primary, fallback);

        let error = chain.transcribe(&request()).unwrap_err().to_string();

        assert!(error.contains("primary failed (boom)"));
        assert!(error.contains("fallback also failed (no model)"));
    }

    #[test]
    fn shutdown_reaches_every_engine_in_the_chain() {
        let (primary, primary_counters) = Scripted::boxed("primary", Ok("a"));
        let (fallback, fallback_counters) = Scripted::boxed("fallback", Ok("b"));
        let chain = FallbackProvider::new(primary, fallback);

        chain.shutdown();

        assert_eq!(primary_counters.shutdowns.load(Ordering::SeqCst), 1);
        assert_eq!(fallback_counters.shutdowns.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deadline_scales_with_audio_and_is_capped() {
        assert_eq!(
            transcription_deadline(Duration::from_secs(5)),
            Duration::from_secs(40)
        );
        assert_eq!(
            transcription_deadline(Duration::from_secs(600)),
            Duration::from_secs(180)
        );
    }
}
