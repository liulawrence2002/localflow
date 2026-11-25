//! Loudness-based voice activity and end-of-speech detection for dictation.
//!
//! Hands-free dictation (a quick tap of the hotkey) ends after the user's configured
//! end-of-speech silence. Push-to-talk (holding the hotkey) is ended by the release only: while
//! the key is held the voice-activity stops are suppressed and only the maximum length
//! applies.

use std::time::{Duration, Instant};

pub(crate) const VAD_START_RMS_THRESHOLD: f32 = 0.008;
pub(crate) const VAD_CONTINUE_RMS_THRESHOLD: f32 = 0.0045;
pub(crate) const VAD_CONFIRM_SPEECH_MS: u64 = 120;
/// The end-of-speech silence the setting is clamped to.
pub(crate) const MIN_END_OF_SPEECH_MS: u16 = 300;
pub(crate) const MAX_END_OF_SPEECH_MS: u16 = 3_000;
pub(crate) const MIN_AUTO_STOP_RECORDING_MS: u64 = 350;

pub(crate) const NO_SPEECH_TIMEOUT_MS: u64 = 2_500;
pub(crate) const MAX_RECORDING_SECS: u64 = 120;

/// The end-of-speech silence for `settings.microphone.end_of_speech_ms`, clamped to
/// 300-3000 ms.
pub(crate) fn end_of_speech_timeout(setting_ms: u16) -> Duration {
    Duration::from_millis(u64::from(
        setting_ms.clamp(MIN_END_OF_SPEECH_MS, MAX_END_OF_SPEECH_MS),
    ))
}

/// Why the detector ended a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopReason {
    /// The recording reached its maximum length. Applies even while the hotkey is held.
    MaxLength,
    /// Nothing that sounded like speech was heard in time (hands-free only).
    NoSpeech,
    /// Speech was followed by the end-of-speech silence (hands-free only).
    EndOfSpeech,
}

#[derive(Debug)]
pub(crate) struct EndOfSpeechDetector {
    pub(crate) started_at: Instant,
    pub(crate) end_of_speech: Duration,
    pub(crate) speech_started_at: Option<Instant>,
    pub(crate) last_voice_at: Option<Instant>,
    pub(crate) speech_seen: bool,
}

impl EndOfSpeechDetector {
    pub(crate) fn new(started_at: Instant, end_of_speech: Duration) -> Self {
        Self {
            started_at,
            end_of_speech,
            speech_started_at: None,
            last_voice_at: None,
            speech_seen: false,
        }
    }

    /// Feed in the loudness of the audio gathered since the last call.
    pub(crate) fn observe(&mut self, now: Instant, rms: f32) {
        let voice_threshold = if self.speech_seen {
            VAD_CONTINUE_RMS_THRESHOLD
        } else {
            VAD_START_RMS_THRESHOLD
        };

        if rms >= voice_threshold {
            let speech_started_at = self.speech_started_at.get_or_insert(now);
            self.last_voice_at = Some(now);

            if now.duration_since(*speech_started_at)
                >= Duration::from_millis(VAD_CONFIRM_SPEECH_MS)
                || rms >= VAD_START_RMS_THRESHOLD * 1.6
            {
                self.speech_seen = true;
            }
        } else if !self.speech_seen {
            self.speech_started_at = None;
        }
    }

    /// Whether the recording should end at `now`, and why. `hands_free` is false while the
    /// hotkey is held (push-to-talk): then only the maximum length ends it.
    pub(crate) fn stop_reason(&self, now: Instant, hands_free: bool) -> Option<StopReason> {
        let elapsed = now.duration_since(self.started_at);

        if elapsed >= Duration::from_secs(MAX_RECORDING_SECS) {
            return Some(StopReason::MaxLength);
        }
        if !hands_free {
            return None;
        }

        if !self.speech_seen && elapsed >= Duration::from_millis(NO_SPEECH_TIMEOUT_MS) {
            return Some(StopReason::NoSpeech);
        }

        let silence_long_enough = self
            .last_voice_at
            .is_some_and(|last_voice_at| now.duration_since(last_voice_at) >= self.end_of_speech);
        (self.speech_seen
            && elapsed >= Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS)
            && silence_long_enough)
            .then_some(StopReason::EndOfSpeech)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANDS_FREE: bool = true;
    const HELD: bool = false;

    fn detector(started_at: Instant) -> EndOfSpeechDetector {
        EndOfSpeechDetector::new(started_at, end_of_speech_timeout(900))
    }

    fn at(started_at: Instant, ms: u64) -> Instant {
        started_at + Duration::from_millis(ms)
    }

    #[test]
    fn the_setting_is_clamped_to_300_to_3000_ms() {
        assert_eq!(end_of_speech_timeout(900), Duration::from_millis(900));
        assert_eq!(end_of_speech_timeout(0), Duration::from_millis(300));
        assert_eq!(end_of_speech_timeout(100), Duration::from_millis(300));
        assert_eq!(end_of_speech_timeout(3_000), Duration::from_millis(3_000));
        assert_eq!(end_of_speech_timeout(60_000), Duration::from_millis(3_000));
    }

    #[test]
    fn end_of_speech_detector_waits_for_actual_voice() {
        let started_at = Instant::now();
        let mut detector = detector(started_at);

        detector.observe(at(started_at, 900), 0.0);
        assert_eq!(detector.stop_reason(at(started_at, 900), HANDS_FREE), None);
        detector.observe(at(started_at, 1_800), 0.001);
        assert_eq!(
            detector.stop_reason(at(started_at, 1_800), HANDS_FREE),
            None
        );
    }

    #[test]
    fn end_of_speech_detector_times_out_when_no_voice_arrives() {
        let started_at = Instant::now();
        let detector = detector(started_at);

        assert_eq!(
            detector.stop_reason(at(started_at, NO_SPEECH_TIMEOUT_MS - 100), HANDS_FREE),
            None
        );
        assert_eq!(
            detector.stop_reason(at(started_at, NO_SPEECH_TIMEOUT_MS), HANDS_FREE),
            Some(StopReason::NoSpeech)
        );
    }

    #[test]
    fn hands_free_stops_after_the_configured_silence() {
        let started_at = Instant::now();
        let mut detector = detector(started_at);

        detector.observe(at(started_at, 80), 0.02);
        detector.observe(at(started_at, 220), 0.012);
        assert!(detector.speech_seen);

        // 899 ms after the last voice: still listening; 900 ms: done.
        assert_eq!(
            detector.stop_reason(at(started_at, 220 + 899), HANDS_FREE),
            None
        );
        assert_eq!(
            detector.stop_reason(at(started_at, 220 + 900), HANDS_FREE),
            Some(StopReason::EndOfSpeech)
        );
    }

    #[test]
    fn a_shorter_setting_stops_sooner() {
        let started_at = Instant::now();
        let mut quick = EndOfSpeechDetector::new(started_at, end_of_speech_timeout(300));

        quick.observe(at(started_at, 80), 0.02);
        quick.observe(at(started_at, 220), 0.012);

        assert_eq!(
            quick.stop_reason(at(started_at, 220 + 300), HANDS_FREE),
            Some(StopReason::EndOfSpeech)
        );
        // The minimum recording length still applies to a very early silence.
        let mut early = EndOfSpeechDetector::new(started_at, end_of_speech_timeout(300));
        early.observe(at(started_at, 0), 0.05);
        assert_eq!(early.stop_reason(at(started_at, 300), HANDS_FREE), None);
        assert_eq!(
            early.stop_reason(at(started_at, MIN_AUTO_STOP_RECORDING_MS), HANDS_FREE),
            Some(StopReason::EndOfSpeech)
        );
    }

    #[test]
    fn a_held_hotkey_is_never_stopped_by_silence() {
        let started_at = Instant::now();
        let mut detector = detector(started_at);

        // Speech, then a long pause to think: push-to-talk keeps recording.
        detector.observe(at(started_at, 80), 0.02);
        detector.observe(at(started_at, 220), 0.012);
        assert_eq!(detector.stop_reason(at(started_at, 10_000), HELD), None);

        // Holding without saying anything is not abandoned either.
        let silent = EndOfSpeechDetector::new(started_at, end_of_speech_timeout(900));
        assert_eq!(silent.stop_reason(at(started_at, 30_000), HELD), None);
    }

    #[test]
    fn releasing_a_quick_tap_resumes_the_silence_rules() {
        let started_at = Instant::now();
        let mut detector = detector(started_at);
        detector.observe(at(started_at, 80), 0.02);
        detector.observe(at(started_at, 220), 0.012);

        // Held at 1.5 s: nothing. The same moment hands-free: the pause has been long enough.
        assert_eq!(detector.stop_reason(at(started_at, 1_500), HELD), None);
        assert_eq!(
            detector.stop_reason(at(started_at, 1_500), HANDS_FREE),
            Some(StopReason::EndOfSpeech)
        );
    }

    #[test]
    fn no_speech_timeout_is_distinguishable_from_end_of_speech() {
        let started_at = Instant::now();

        // Abandoned tap: never any voice -> stops via NO_SPEECH_TIMEOUT with speech_seen false,
        // so the recorder cancels silently instead of flashing an error.
        let idle = detector(started_at);
        assert_eq!(
            idle.stop_reason(at(started_at, NO_SPEECH_TIMEOUT_MS), HANDS_FREE),
            Some(StopReason::NoSpeech)
        );
        assert!(!idle.speech_seen, "abandoned tap must not report speech");

        // Real speech then silence -> stops via END_OF_SPEECH with speech_seen true.
        let mut spoken = detector(started_at);
        spoken.observe(at(started_at, 80), 0.02);
        spoken.observe(at(started_at, 220), 0.012);
        assert_eq!(
            spoken.stop_reason(at(started_at, 220 + 900), HANDS_FREE),
            Some(StopReason::EndOfSpeech)
        );
        assert!(spoken.speech_seen, "spoken dictation must report speech");
    }

    #[test]
    fn the_maximum_length_applies_held_or_not() {
        let started_at = Instant::now();
        let detector = detector(started_at);
        let past_limit = started_at + Duration::from_secs(MAX_RECORDING_SECS + 1);

        assert_eq!(
            detector.stop_reason(past_limit, HANDS_FREE),
            Some(StopReason::MaxLength)
        );
        assert_eq!(
            detector.stop_reason(past_limit, HELD),
            Some(StopReason::MaxLength)
        );
    }

    #[test]
    fn timeouts_fire_without_new_audio_when_the_device_stops_delivering() {
        // A disconnected device yields empty polls, which only check the timeouts.
        let started_at = Instant::now();

        // After speech: the end-of-speech timeout ends the recording.
        let mut detector = detector(started_at);
        detector.observe(at(started_at, 100), 0.05);
        detector.observe(at(started_at, 400), 0.05);
        assert_eq!(detector.stop_reason(at(started_at, 500), HANDS_FREE), None);
        assert_eq!(
            detector.stop_reason(at(started_at, 400 + 900 + 10), HANDS_FREE),
            Some(StopReason::EndOfSpeech)
        );

        // Never any speech: the no-speech timeout ends it.
        let silent = EndOfSpeechDetector::new(started_at, end_of_speech_timeout(900));
        assert_eq!(
            silent.stop_reason(at(started_at, NO_SPEECH_TIMEOUT_MS - 10), HANDS_FREE),
            None
        );
        assert_eq!(
            silent.stop_reason(at(started_at, NO_SPEECH_TIMEOUT_MS + 10), HANDS_FREE),
            Some(StopReason::NoSpeech)
        );
    }

    #[test]
    fn an_empty_poll_does_not_reset_speech_confirmation() {
        // Quiet speech is confirmed only after 120 ms above the start threshold. An empty
        // poll between two quiet chunks must not restart that clock.
        let started_at = Instant::now();
        let mut detector = detector(started_at);
        let quiet = VAD_START_RMS_THRESHOLD * 1.1;

        detector.observe(at(started_at, 60), quiet);
        assert!(!detector.speech_seen);
        assert_eq!(detector.stop_reason(at(started_at, 115), HANDS_FREE), None);
        detector.observe(at(started_at, 190), quiet);

        assert!(detector.speech_seen);
    }
}
