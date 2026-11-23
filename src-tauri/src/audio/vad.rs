//! Loudness-based voice activity and end-of-speech detection.

use std::time::{Duration, Instant};

pub(crate) const VAD_START_RMS_THRESHOLD: f32 = 0.008;
pub(crate) const VAD_CONTINUE_RMS_THRESHOLD: f32 = 0.0045;
pub(crate) const VAD_CONFIRM_SPEECH_MS: u64 = 120;
pub(crate) const END_OF_SPEECH_TIMEOUT_MS: u64 = 550;
pub(crate) const MIN_AUTO_STOP_RECORDING_MS: u64 = 350;

pub(crate) const NO_SPEECH_TIMEOUT_MS: u64 = 2_500;
pub(crate) const MAX_RECORDING_SECS: u64 = 120;

#[derive(Debug)]
pub(crate) struct EndOfSpeechDetector {
    pub(crate) started_at: Instant,
    pub(crate) speech_started_at: Option<Instant>,
    pub(crate) last_voice_at: Option<Instant>,
    pub(crate) speech_seen: bool,
}

impl EndOfSpeechDetector {
    pub(crate) fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            speech_started_at: None,
            last_voice_at: None,
            speech_seen: false,
        }
    }

    /// Whether the recording should end at `now` given what has been heard so far, without
    /// feeding in any new audio: maximum length, no speech at all, or enough silence after
    /// speech.
    pub(crate) fn timed_out(&self, now: Instant) -> bool {
        let elapsed = now.duration_since(self.started_at);

        if elapsed >= Duration::from_secs(MAX_RECORDING_SECS) {
            return true;
        }

        if !self.speech_seen && elapsed >= Duration::from_millis(NO_SPEECH_TIMEOUT_MS) {
            return true;
        }

        self.speech_seen
            && elapsed >= Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS)
            && self.last_voice_at.is_some_and(|last_voice_at| {
                now.duration_since(last_voice_at) >= Duration::from_millis(END_OF_SPEECH_TIMEOUT_MS)
            })
    }

    pub(crate) fn observe(&mut self, now: Instant, rms: f32) -> bool {
        let elapsed = now.duration_since(self.started_at);

        if elapsed >= Duration::from_secs(MAX_RECORDING_SECS) {
            return true;
        }

        if !self.speech_seen && elapsed >= Duration::from_millis(NO_SPEECH_TIMEOUT_MS) {
            return true;
        }

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

        self.speech_seen
            && now.duration_since(self.started_at)
                >= Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS)
            && self.last_voice_at.is_some_and(|last_voice_at| {
                now.duration_since(last_voice_at) >= Duration::from_millis(END_OF_SPEECH_TIMEOUT_MS)
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_of_speech_detector_waits_for_actual_voice() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(started_at + Duration::from_millis(900), 0.0));
        assert!(!detector.observe(started_at + Duration::from_millis(1_800), 0.001));
    }

    #[test]
    fn end_of_speech_detector_times_out_when_no_voice_arrives() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS - 100),
            0.0,
        ));
        assert!(detector.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS),
            0.0,
        ));
    }

    #[test]
    fn end_of_speech_detector_stops_after_post_speech_silence() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(started_at + Duration::from_millis(80), 0.02));
        assert!(!detector.observe(started_at + Duration::from_millis(220), 0.012));
        assert!(!detector.observe(
            started_at + Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS + 120),
            0.0,
        ));
        assert!(detector.observe(
            started_at
                + Duration::from_millis(
                    MIN_AUTO_STOP_RECORDING_MS + END_OF_SPEECH_TIMEOUT_MS + 120,
                ),
            0.0,
        ));
    }

    #[test]
    fn no_speech_timeout_is_distinguishable_from_end_of_speech() {
        let started_at = Instant::now();

        // Abandoned tap: never any voice -> stops via NO_SPEECH_TIMEOUT with speech_seen false,
        // so the recorder cancels silently instead of flashing an error.
        let mut idle = EndOfSpeechDetector::new(started_at);
        assert!(idle.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS),
            0.0,
        ));
        assert!(!idle.speech_seen, "abandoned tap must not report speech");

        // Real speech then silence -> stops via END_OF_SPEECH with speech_seen true.
        let mut spoken = EndOfSpeechDetector::new(started_at);
        assert!(!spoken.observe(started_at + Duration::from_millis(80), 0.02));
        assert!(!spoken.observe(started_at + Duration::from_millis(220), 0.012));
        assert!(spoken.observe(
            started_at
                + Duration::from_millis(
                    MIN_AUTO_STOP_RECORDING_MS + END_OF_SPEECH_TIMEOUT_MS + 120,
                ),
            0.0,
        ));
        assert!(spoken.speech_seen, "spoken dictation must report speech");
    }

    #[test]
    fn end_of_speech_detector_caps_long_recordings() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(detector.observe(
            started_at + Duration::from_secs(MAX_RECORDING_SECS + 1),
            0.0
        ));
    }

    #[test]
    fn timeouts_fire_without_new_audio_when_the_device_stops_delivering() {
        // A disconnected device yields empty polls, which only check the timeouts.
        let started_at = Instant::now();

        // After speech: the end-of-speech timeout ends the recording.
        let mut detector = EndOfSpeechDetector::new(started_at);
        assert!(!detector.observe(started_at + Duration::from_millis(100), 0.05));
        assert!(!detector.observe(started_at + Duration::from_millis(400), 0.05));
        assert!(!detector.timed_out(started_at + Duration::from_millis(500)));
        assert!(detector
            .timed_out(started_at + Duration::from_millis(400 + END_OF_SPEECH_TIMEOUT_MS + 10)));

        // Never any speech: the no-speech timeout ends it.
        let silent = EndOfSpeechDetector::new(started_at);
        assert!(!silent.timed_out(started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS - 10)));
        assert!(silent.timed_out(started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS + 10)));

        // And nothing outlives the maximum recording length.
        assert!(silent.timed_out(started_at + Duration::from_secs(MAX_RECORDING_SECS + 1)));
    }

    #[test]
    fn an_empty_poll_does_not_reset_speech_confirmation() {
        // Quiet speech is confirmed only after 120 ms above the start threshold. An empty
        // poll between two quiet chunks must not restart that clock.
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);
        let quiet = VAD_START_RMS_THRESHOLD * 1.1;

        assert!(!detector.observe(started_at + Duration::from_millis(60), quiet));
        assert!(!detector.timed_out(started_at + Duration::from_millis(115)));
        detector.observe(started_at + Duration::from_millis(190), quiet);

        assert!(detector.speech_seen);
    }
}
