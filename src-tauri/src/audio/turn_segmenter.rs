//! Turns a stream of per-frame speech probabilities into "the user started speaking" and
//! "the user finished" events.
//!
//! Pure and frame-driven: time is counted in frames of known length, not read from a clock,
//! so the timing is exact, independent of scheduling, and unit-tested. Any detector that
//! yields a probability per frame can drive it (see [`super::speech_detector`]).

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmenterConfig {
    /// Length of audio each probability covers.
    pub frame: Duration,
    /// A frame at or above this is speech when the user is not yet speaking.
    pub start_threshold: f32,
    /// Once speaking, a frame below this is silence. Lower than the start threshold so a
    /// voice trailing off does not flicker between the two.
    pub continue_threshold: f32,
    /// Speech must last this long before it counts, so a click or a cough starts nothing.
    pub min_speech: Duration,
    /// Silence this long after speech ends the turn.
    pub end_silence: Duration,
}

impl SegmenterConfig {
    /// Defaults for conversation with a detector that scores `frame` of audio at a time.
    ///
    /// 700 ms of silence ends a turn. That is longer than dictation's 550 ms on purpose:
    /// people pause mid-thought when talking to someone, and being cut off is worse than a
    /// slightly later reply. A turn-completion model is the planned way to shorten it.
    pub fn conversation(frame: Duration) -> Self {
        Self {
            frame,
            start_threshold: 0.5,
            continue_threshold: 0.35,
            min_speech: Duration::from_millis(96),
            end_silence: Duration::from_millis(700),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentEvent {
    /// Speech has lasted `min_speech`. It began `frames_ago` frames before this one.
    SpeechStarted { frames_ago: u32 },
    /// Silence has lasted `end_silence`. Speech ended `frames_ago` frames before this one.
    SpeechEnded { frames_ago: u32 },
}

#[derive(Debug, Clone)]
pub struct TurnSegmenter {
    config: SegmenterConfig,
    speaking: bool,
    /// Consecutive speech frames while not yet speaking.
    speech_run: u32,
    /// Consecutive silent frames while speaking.
    silence_run: u32,
}

impl TurnSegmenter {
    pub fn new(config: SegmenterConfig) -> Self {
        Self {
            config,
            speaking: false,
            speech_run: 0,
            silence_run: 0,
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Forget any partial run and go back to waiting for speech.
    pub fn reset(&mut self) {
        self.speaking = false;
        self.speech_run = 0;
        self.silence_run = 0;
    }

    /// Feed the probability of the next frame.
    pub fn observe(&mut self, probability: f32) -> Option<SegmentEvent> {
        if !self.speaking {
            if probability >= self.config.start_threshold {
                self.speech_run += 1;
                if self.speech_run >= self.frames(self.config.min_speech) {
                    self.speaking = true;
                    self.silence_run = 0;
                    let frames_ago = self.speech_run - 1;
                    self.speech_run = 0;
                    return Some(SegmentEvent::SpeechStarted { frames_ago });
                }
            } else {
                self.speech_run = 0;
            }
            return None;
        }

        if probability < self.config.continue_threshold {
            self.silence_run += 1;
            if self.silence_run >= self.frames(self.config.end_silence) {
                let frames_ago = self.silence_run;
                self.reset();
                return Some(SegmentEvent::SpeechEnded { frames_ago });
            }
        } else {
            self.silence_run = 0;
        }
        None
    }

    /// Whole frames needed to cover `duration`, at least one.
    fn frames(&self, duration: Duration) -> u32 {
        let frame = self.config.frame.as_micros().max(1);
        (duration.as_micros().div_ceil(frame) as u32).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(32);

    fn segmenter() -> TurnSegmenter {
        TurnSegmenter::new(SegmenterConfig::conversation(FRAME))
    }

    /// Feed `count` frames of `probability`, returning the events with their frame index.
    fn feed(
        segmenter: &mut TurnSegmenter,
        probability: f32,
        count: usize,
        clock: &mut usize,
    ) -> Vec<(usize, SegmentEvent)> {
        let mut events = Vec::new();
        for _ in 0..count {
            if let Some(event) = segmenter.observe(probability) {
                events.push((*clock, event));
            }
            *clock += 1;
        }
        events
    }

    #[test]
    fn speech_starts_after_three_frames_and_ends_after_700_ms_of_silence() {
        let mut segmenter = segmenter();
        let mut clock = 0;

        assert!(feed(&mut segmenter, 0.02, 30, &mut clock).is_empty());
        let started = feed(&mut segmenter, 0.9, 40, &mut clock);
        // 96 ms is three 32 ms frames: the event arrives on the third, two frames after
        // speech began.
        assert_eq!(
            started,
            vec![(32, SegmentEvent::SpeechStarted { frames_ago: 2 })]
        );

        let ended = feed(&mut segmenter, 0.02, 40, &mut clock);
        // 700 ms is 22 frames (704 ms).
        assert_eq!(
            ended,
            vec![(70 + 21, SegmentEvent::SpeechEnded { frames_ago: 22 })]
        );
        assert!(!segmenter.is_speaking());
    }

    #[test]
    fn a_click_or_cough_starts_nothing() {
        let mut segmenter = segmenter();
        let mut clock = 0;

        for _ in 0..10 {
            assert!(feed(&mut segmenter, 0.95, 2, &mut clock).is_empty());
            assert!(feed(&mut segmenter, 0.01, 5, &mut clock).is_empty());
        }
        assert!(!segmenter.is_speaking());
    }

    #[test]
    fn a_pause_shorter_than_the_limit_keeps_the_turn_open() {
        let mut segmenter = segmenter();
        let mut clock = 0;
        feed(&mut segmenter, 0.9, 10, &mut clock);

        // 21 frames (672 ms) of silence, then speech again: one turn, not two.
        assert!(feed(&mut segmenter, 0.0, 21, &mut clock).is_empty());
        assert!(feed(&mut segmenter, 0.9, 5, &mut clock).is_empty());
        assert!(segmenter.is_speaking());
        // The silence count started over.
        assert!(feed(&mut segmenter, 0.0, 21, &mut clock).is_empty());
        assert_eq!(feed(&mut segmenter, 0.0, 1, &mut clock).len(), 1);
    }

    #[test]
    fn a_voice_trailing_off_does_not_end_the_turn_early() {
        let mut segmenter = segmenter();
        let mut clock = 0;
        feed(&mut segmenter, 0.9, 10, &mut clock);

        // Below the start threshold but above the continue threshold: still speech.
        assert!(feed(&mut segmenter, 0.4, 60, &mut clock).is_empty());
        assert!(segmenter.is_speaking());
        // The same level would not have started a turn.
        let mut fresh = self::segmenter();
        assert!(feed(&mut fresh, 0.4, 60, &mut 0).is_empty());
        assert!(!fresh.is_speaking());
    }

    #[test]
    fn several_turns_in_a_row_each_start_and_end_once() {
        let mut segmenter = segmenter();
        let mut clock = 0;
        let mut events = Vec::new();
        for _ in 0..4 {
            events.extend(feed(&mut segmenter, 0.8, 20, &mut clock));
            events.extend(feed(&mut segmenter, 0.0, 40, &mut clock));
        }

        let kinds: Vec<bool> = events
            .iter()
            .map(|(_, event)| matches!(event, SegmentEvent::SpeechStarted { .. }))
            .collect();
        assert_eq!(
            kinds,
            vec![true, false, true, false, true, false, true, false]
        );
    }

    #[test]
    fn reset_discards_a_turn_in_progress() {
        let mut segmenter = segmenter();
        let mut clock = 0;
        feed(&mut segmenter, 0.9, 10, &mut clock);
        assert!(segmenter.is_speaking());

        segmenter.reset();

        assert!(!segmenter.is_speaking());
        // Silence afterwards ends nothing, because nothing is in progress.
        assert!(feed(&mut segmenter, 0.0, 100, &mut clock).is_empty());
    }

    #[test]
    fn frame_counts_round_up_for_other_frame_lengths() {
        let config = SegmenterConfig::conversation(Duration::from_millis(10));
        let mut segmenter = TurnSegmenter::new(config);
        let mut clock = 0;

        // 96 ms needs ten 10 ms frames.
        assert!(feed(&mut segmenter, 0.9, 9, &mut clock).is_empty());
        assert_eq!(feed(&mut segmenter, 0.9, 1, &mut clock).len(), 1);
    }
}
