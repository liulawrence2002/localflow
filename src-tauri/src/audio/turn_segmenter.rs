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
    /// While speech is still being confirmed, a frame at or above this keeps the run going
    /// once a frame at or above `start_threshold` has begun it (Silero's hysteresis, where
    /// speech stays speech down to 0.15 below the threshold that started it). `None` for the
    /// plain rule: every frame of the run must reach `start_threshold`.
    pub confirm_threshold: Option<f32>,
    /// Frames below the confirming threshold, up to this long in a row, pause the run
    /// instead of starting it over. They do not count towards `min_speech`. Zero for the
    /// plain rule.
    pub confirm_gap: Duration,
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
            confirm_threshold: None,
            confirm_gap: Duration::ZERO,
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
    /// Speech frames counted towards `min_speech` while not yet speaking.
    speech_run: u32,
    /// Frames since that run began, gaps included.
    run_length: u32,
    /// Consecutive frames below the confirming threshold within the run.
    gap_run: u32,
    /// Consecutive silent frames while speaking.
    silence_run: u32,
}

impl TurnSegmenter {
    pub fn new(config: SegmenterConfig) -> Self {
        Self {
            config,
            speaking: false,
            speech_run: 0,
            run_length: 0,
            gap_run: 0,
            silence_run: 0,
        }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Forget any partial run and go back to waiting for speech.
    pub fn reset(&mut self) {
        self.speaking = false;
        self.clear_run();
        self.silence_run = 0;
    }

    /// Treat speech as already under way, so the next pause is reported. Used when another
    /// detector (with stricter rules) has just confirmed the start of speech.
    pub fn assume_speaking(&mut self) {
        self.speaking = true;
        self.clear_run();
        self.silence_run = 0;
    }

    fn clear_run(&mut self) {
        self.speech_run = 0;
        self.run_length = 0;
        self.gap_run = 0;
    }

    /// Feed the probability of the next frame.
    pub fn observe(&mut self, probability: f32) -> Option<SegmentEvent> {
        if !self.speaking {
            let keep = self
                .config
                .confirm_threshold
                .unwrap_or(self.config.start_threshold);
            let in_run = self.speech_run > 0;
            if probability >= self.config.start_threshold || (in_run && probability >= keep) {
                self.speech_run += 1;
                self.run_length += 1;
                self.gap_run = 0;
                if self.speech_run >= self.frames(self.config.min_speech) {
                    self.speaking = true;
                    self.silence_run = 0;
                    let frames_ago = self.run_length - 1;
                    self.clear_run();
                    return Some(SegmentEvent::SpeechStarted { frames_ago });
                }
            } else if in_run && self.gap_run < self.whole_frames(self.config.confirm_gap) {
                // A moment's dip inside a word: hold the count rather than start again.
                self.gap_run += 1;
                self.run_length += 1;
            } else {
                self.clear_run();
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

    /// Whole frames that fit in `duration`, possibly none.
    fn whole_frames(&self, duration: Duration) -> u32 {
        let frame = self.config.frame.as_micros().max(1);
        (duration.as_micros() / frame) as u32
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
    fn speech_assumed_under_way_ends_at_the_next_pause_without_a_second_start() {
        let mut segmenter = segmenter();
        let mut clock = 0;
        segmenter.assume_speaking();

        assert!(
            feed(&mut segmenter, 0.9, 10, &mut clock).is_empty(),
            "no second start"
        );
        let events = feed(&mut segmenter, 0.0, 30, &mut clock);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0].1, SegmentEvent::SpeechEnded { .. }));
    }

    /// The barge-in gate: enter at 0.65, stay down to 0.5, 288 ms of speech in all, and one
    /// frame below that tolerated inside the run.
    fn guard() -> TurnSegmenter {
        TurnSegmenter::new(SegmenterConfig {
            start_threshold: 0.65,
            confirm_threshold: Some(0.5),
            confirm_gap: FRAME,
            min_speech: Duration::from_millis(288),
            ..SegmenterConfig::conversation(FRAME)
        })
    }

    /// Feed each probability in turn, returning the events with their frame index.
    fn feed_each(
        segmenter: &mut TurnSegmenter,
        probabilities: &[f32],
    ) -> Vec<(usize, SegmentEvent)> {
        probabilities
            .iter()
            .enumerate()
            .filter_map(|(index, probability)| {
                segmenter.observe(*probability).map(|event| (index, event))
            })
            .collect()
    }

    #[test]
    fn with_hysteresis_speech_that_dips_below_the_start_threshold_still_counts() {
        // 288 ms is nine 32 ms frames. Only the first reaches 0.65.
        let probabilities = [0.7, 0.55, 0.6, 0.52, 0.58, 0.6, 0.55, 0.51, 0.6];

        assert_eq!(
            feed_each(&mut guard(), &probabilities),
            vec![(8, SegmentEvent::SpeechStarted { frames_ago: 8 })]
        );
        // The plain rule at the same start threshold starts over at every dip.
        let mut plain = TurnSegmenter::new(SegmenterConfig {
            start_threshold: 0.65,
            min_speech: Duration::from_millis(288),
            ..SegmenterConfig::conversation(FRAME)
        });
        assert!(feed_each(&mut plain, &probabilities).is_empty());
    }

    #[test]
    fn with_hysteresis_a_run_must_still_begin_at_the_start_threshold() {
        assert!(feed_each(&mut guard(), &[0.6; 40]).is_empty());
    }

    #[test]
    fn one_quiet_frame_pauses_the_count_and_two_start_it_over() {
        // Four frames, one quiet frame, five more: nine of speech, ten frames in all.
        let mut probabilities = vec![0.8; 4];
        probabilities.push(0.1);
        probabilities.extend([0.8; 5]);
        assert_eq!(
            feed_each(&mut guard(), &probabilities),
            vec![(9, SegmentEvent::SpeechStarted { frames_ago: 9 })]
        );

        // Two quiet frames in a row: the first four no longer count.
        let mut probabilities = vec![0.8; 4];
        probabilities.extend([0.1, 0.1]);
        probabilities.extend([0.8; 8]);
        assert!(feed_each(&mut guard(), &probabilities).is_empty());
        // Gaps do not count as speech: three frames of it among gaps are still three.
        assert!(feed_each(&mut guard(), &[0.1, 0.8, 0.1, 0.8, 0.1, 0.8]).is_empty());
    }

    #[test]
    fn the_plain_rule_is_unchanged_by_the_hysteresis_options() {
        // The normal turn segmenter has neither option: a dip below 0.5 starts the count over.
        let mut segmenter = segmenter();
        assert!(feed_each(&mut segmenter, &[0.9, 0.9, 0.4, 0.9, 0.9]).is_empty());
        assert_eq!(
            feed_each(&mut segmenter, &[0.9]),
            vec![(0, SegmentEvent::SpeechStarted { frames_ago: 2 })]
        );
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
