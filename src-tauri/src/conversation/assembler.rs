//! Decides when a user's turn has ended and what they said, while they are still speaking.
//!
//! The first version of conversation mode waited for 700 ms of silence, and only then began
//! transcribing: every reply paid for the full wait plus the full transcription. This does
//! both jobs during the turn instead:
//!
//! - At every short pause (about a quarter of a second) the speech so far is sent for
//!   transcription. If the user carries on, that text is kept and only what follows is
//!   transcribed at the next pause, so a long turn costs no more than its last phrase.
//! - Once the transcript of everything said is in hand, how long to wait before answering
//!   depends on how finished it reads: briefly after a complete question, the ordinary time
//!   when unsure, and noticeably longer after something that trails off mid-thought.
//!
//! It is pure. Time is counted in audio samples, not read from a clock; the caller feeds it
//! what the speech detector saw and what the recogniser returned, and carries out the
//! actions it gives back. That makes every interleaving of pauses, resumptions and late
//! transcripts unit-testable.

use std::{sync::Arc, time::Duration};

/// How finished the words so far read. Decides how long a silence must last to end the turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Readiness {
    /// A finished question, statement or command.
    Ready,
    /// Could be finished or not.
    Unsure,
    /// Trails off on something that needs more after it.
    NotReady,
}

/// Judges a transcript. Injected so tests can script it.
pub type ReadinessJudge = Arc<dyn Fn(&str) -> Readiness + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssemblerConfig {
    /// Sample rate of the audio clock.
    pub sample_rate: u32,
    /// Audio kept from before speech was detected, so the first word is not clipped.
    pub pre_roll: Duration,
    /// Audio kept after the last word of a phrase.
    pub tail: Duration,
    /// Silence needed to end a turn that reads as finished.
    pub ready_wait: Duration,
    /// Silence needed when it is unclear whether the turn is finished.
    pub unsure_wait: Duration,
    /// Silence needed when the turn reads as unfinished.
    pub not_ready_wait: Duration,
    /// A turn is cut at this length even if the user never pauses.
    pub max_turn: Duration,
}

impl AssemblerConfig {
    fn samples(&self, duration: Duration) -> u64 {
        (duration.as_secs_f64() * self.sample_rate as f64) as u64
    }
}

/// What the caller must do. Sample positions are absolute on the caller's audio clock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// The user has started a turn.
    TurnStarted,
    /// Transcribe the audio in `from..to` and report back with this `id`.
    Transcribe { id: u64, from: u64, to: u64 },
    /// The turn is over: this is everything the user said.
    Commit {
        text: String,
        /// Where the user's last word ended.
        speech_end: u64,
        /// Silence that had passed when the turn was ended.
        waited: Duration,
    },
    /// The turn held no words (a cough, a noise). Go back to waiting.
    Abandon,
    /// Audio before this position will not be asked for again.
    Release { before: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle,
    Speaking,
    /// The user stopped at `speech_end`. `forced` when the turn hit its maximum length.
    Paused {
        speech_end: u64,
        forced: bool,
    },
}

pub struct TurnAssembler {
    config: AssemblerConfig,
    judge: ReadinessJudge,
    phase: Phase,
    turn_start: u64,
    /// Start of the audio not yet transcribed.
    segment_start: u64,
    /// Everything transcribed so far in this turn.
    text: String,
    /// The transcription in flight: its id and the range it covers.
    pending: Option<(u64, u64, u64)>,
    next_id: u64,
}

impl TurnAssembler {
    pub fn new(config: AssemblerConfig, judge: ReadinessJudge) -> Self {
        Self {
            config,
            judge,
            phase: Phase::Idle,
            turn_start: 0,
            segment_start: 0,
            text: String::new(),
            pending: None,
            next_id: 1,
        }
    }

    pub fn in_turn(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Forget the turn in progress. A transcription still in flight will be ignored.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.text.clear();
        self.pending = None;
    }

    /// The detector heard speech beginning at `speech_start`.
    pub fn speech_started(&mut self, speech_start: u64) -> Vec<Action> {
        match self.phase {
            Phase::Idle => {
                self.phase = Phase::Speaking;
                self.segment_start =
                    speech_start.saturating_sub(self.config.samples(self.config.pre_roll));
                self.turn_start = self.segment_start;
                self.text.clear();
                vec![Action::TurnStarted]
            }
            // The user carried on after a pause: the turn simply continues.
            Phase::Paused { .. } => {
                self.phase = Phase::Speaking;
                Vec::new()
            }
            Phase::Speaking => Vec::new(),
        }
    }

    /// The detector heard the user stop at `speech_end`; it is now `now`.
    pub fn pause_started(&mut self, speech_end: u64, now: u64) -> Vec<Action> {
        if self.phase != Phase::Speaking {
            return Vec::new();
        }
        self.phase = Phase::Paused {
            speech_end,
            forced: false,
        };
        let mut actions = self.request_transcription(now);
        actions.extend(self.try_commit(now));
        actions
    }

    /// Time has passed with nothing new from the detector.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        match self.phase {
            Phase::Idle => Vec::new(),
            Phase::Speaking => {
                if now.saturating_sub(self.turn_start) < self.config.samples(self.config.max_turn) {
                    return Vec::new();
                }
                // Talking without a break for the maximum length: end the turn here.
                self.phase = Phase::Paused {
                    speech_end: now,
                    forced: true,
                };
                let mut actions = self.request_transcription(now);
                actions.extend(self.try_commit(now));
                actions
            }
            Phase::Paused { .. } => self.try_commit(now),
        }
    }

    /// The recogniser returned `text` for request `id`.
    pub fn transcribed(&mut self, id: u64, text: &str, now: u64) -> Vec<Action> {
        let Some((pending, _, to)) = self.pending else {
            return Vec::new();
        };
        if pending != id {
            return Vec::new();
        }
        self.pending = None;

        let text = text.trim();
        if !text.is_empty() {
            if !self.text.is_empty() {
                self.text.push(' ');
            }
            self.text.push_str(text);
        }
        self.segment_start = to;
        let mut actions = vec![Action::Release { before: to }];
        // The user may have spoken again and paused again while this was being transcribed.
        actions.extend(self.request_transcription(now));
        actions.extend(self.try_commit(now));
        actions
    }

    /// The recogniser failed on request `id`: the turn cannot be completed.
    pub fn transcription_failed(&mut self, id: u64) -> Vec<Action> {
        if self.pending.is_some_and(|(pending, _, _)| pending == id) {
            self.reset();
            vec![Action::Abandon]
        } else {
            Vec::new()
        }
    }

    /// Ask for the untranscribed speech up to the current pause, unless a request is already
    /// in flight or there is nothing new.
    fn request_transcription(&mut self, now: u64) -> Vec<Action> {
        let Phase::Paused { speech_end, .. } = self.phase else {
            return Vec::new();
        };
        if self.pending.is_some() || speech_end <= self.segment_start {
            return Vec::new();
        }
        // A little silence after the last word helps the recogniser; never past the present.
        let to = (speech_end + self.config.samples(self.config.tail)).min(now.max(speech_end));
        let id = self.next_id;
        self.next_id += 1;
        self.pending = Some((id, self.segment_start, to));
        vec![Action::Transcribe {
            id,
            from: self.segment_start,
            to,
        }]
    }

    /// End the turn if everything said has been transcribed and the silence has lasted long
    /// enough for how finished it reads.
    fn try_commit(&mut self, now: u64) -> Vec<Action> {
        let Phase::Paused { speech_end, forced } = self.phase else {
            return Vec::new();
        };
        if self.pending.is_some() || self.segment_start < speech_end {
            return Vec::new();
        }
        let silence = now.saturating_sub(speech_end);

        if self.text.trim().is_empty() {
            if forced || silence >= self.config.samples(self.config.unsure_wait) {
                self.reset();
                return vec![Action::Abandon];
            }
            return Vec::new();
        }

        let wait = if forced {
            Duration::ZERO
        } else {
            match (self.judge)(&self.text) {
                Readiness::Ready => self.config.ready_wait,
                Readiness::Unsure => self.config.unsure_wait,
                Readiness::NotReady => self.config.not_ready_wait,
            }
        };
        if silence < self.config.samples(wait) {
            return Vec::new();
        }

        let text = std::mem::take(&mut self.text);
        let waited = Duration::from_secs_f64(silence as f64 / self.config.sample_rate as f64);
        self.reset();
        vec![Action::Commit {
            text,
            speech_end,
            waited,
        }]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u64 = 16_000;

    fn ms(milliseconds: u64) -> u64 {
        milliseconds * RATE / 1_000
    }

    fn config() -> AssemblerConfig {
        AssemblerConfig {
            sample_rate: RATE as u32,
            pre_roll: Duration::from_millis(400),
            tail: Duration::from_millis(200),
            ready_wait: Duration::from_millis(400),
            unsure_wait: Duration::from_millis(700),
            not_ready_wait: Duration::from_millis(1_400),
            max_turn: Duration::from_secs(60),
        }
    }

    /// Reads a transcript ending in "?" or "." as finished, one ending in "and" as not.
    fn judge() -> ReadinessJudge {
        Arc::new(|text: &str| {
            if text.ends_with('?') || text.ends_with('.') {
                Readiness::Ready
            } else if text.ends_with("and") {
                Readiness::NotReady
            } else {
                Readiness::Unsure
            }
        })
    }

    fn assembler() -> TurnAssembler {
        TurnAssembler::new(config(), judge())
    }

    fn request(actions: &[Action]) -> (u64, u64, u64) {
        actions
            .iter()
            .find_map(|action| match action {
                Action::Transcribe { id, from, to } => Some((*id, *from, *to)),
                _ => None,
            })
            .expect("a transcription request")
    }

    fn commit(actions: &[Action]) -> Option<(&str, Duration)> {
        actions.iter().find_map(|action| match action {
            Action::Commit { text, waited, .. } => Some((text.as_str(), *waited)),
            _ => None,
        })
    }

    #[test]
    fn a_finished_question_is_answered_after_a_short_silence() {
        let mut turn = assembler();
        // Speech from 1.0 s to 3.0 s; the detector reports the pause 250 ms later.
        assert_eq!(turn.speech_started(ms(1_000)), vec![Action::TurnStarted]);
        let actions = turn.pause_started(ms(3_000), ms(3_250));
        let (id, from, to) = request(&actions);
        // Transcription starts at the pause, with the lead-in and a short tail.
        assert_eq!((from, to), (ms(600), ms(3_200)));
        assert!(commit(&actions).is_none());

        // The transcript arrives 300 ms into the silence: not long enough yet.
        let actions = turn.transcribed(id, "What is the capital of France?", ms(3_300));
        assert!(commit(&actions).is_none());
        assert!(commit(&turn.tick(ms(3_390))).is_none());
        // At 400 ms of silence the turn ends.
        let actions = turn.tick(ms(3_400));
        let (text, waited) = commit(&actions).expect("the turn should end");
        assert_eq!(text, "What is the capital of France?");
        assert_eq!(waited, Duration::from_millis(400));
        assert!(!turn.in_turn());
    }

    #[test]
    fn a_slow_transcript_ends_the_turn_the_moment_it_arrives() {
        let mut turn = assembler();
        turn.speech_started(ms(1_000));
        let (id, _, _) = request(&turn.pause_started(ms(3_000), ms(3_250)));
        // Nothing can be decided without the words, however long the silence.
        assert!(turn.tick(ms(3_900)).is_empty());

        let actions = turn.transcribed(id, "Tell me a joke.", ms(3_950));

        let (text, waited) = commit(&actions).expect("ends when the words arrive");
        assert_eq!(text, "Tell me a joke.");
        assert_eq!(waited, Duration::from_millis(950));
    }

    #[test]
    fn something_that_trails_off_is_given_much_longer() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        let (id, _, _) = request(&turn.pause_started(ms(2_000), ms(2_250)));
        turn.transcribed(id, "I went to the shop and", ms(2_500));

        assert!(
            turn.tick(ms(2_000 + 700)).is_empty(),
            "ordinary wait is not enough"
        );
        assert!(turn.tick(ms(2_000 + 1_399)).is_empty());
        let actions = turn.tick(ms(2_000 + 1_400));
        assert_eq!(
            commit(&actions).map(|(text, _)| text),
            Some("I went to the shop and")
        );
    }

    #[test]
    fn an_unclear_ending_gets_the_ordinary_wait() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        let (id, _, _) = request(&turn.pause_started(ms(2_000), ms(2_250)));
        turn.transcribed(id, "maybe the blue one", ms(2_400));

        assert!(turn.tick(ms(2_699)).is_empty());
        assert!(commit(&turn.tick(ms(2_700))).is_some());
    }

    #[test]
    fn carrying_on_after_a_pause_keeps_the_turn_open_and_only_the_new_part_is_transcribed() {
        let mut turn = assembler();
        turn.speech_started(ms(1_000));
        let (first, from, to) = request(&turn.pause_started(ms(4_000), ms(4_250)));
        assert_eq!((from, to), (ms(600), ms(4_200)));
        // The user speaks again before the first transcript is back.
        assert!(
            turn.speech_started(ms(4_300)).is_empty(),
            "no second turn is started"
        );
        let actions = turn.transcribed(first, "I went to the shop and", ms(4_600));
        assert_eq!(actions, vec![Action::Release { before: ms(4_200) }]);

        // Second pause: only the audio after the first phrase is sent.
        let actions = turn.pause_started(ms(7_000), ms(7_250));
        let (second, from, to) = request(&actions);
        assert_eq!((from, to), (ms(4_200), ms(7_200)));
        let actions = turn.transcribed(second, "bought some bread.", ms(7_450));

        let (text, _) = commit(&actions).expect("the whole turn");
        assert_eq!(text, "I went to the shop and bought some bread.");
    }

    #[test]
    fn a_second_pause_during_a_transcription_is_picked_up_when_it_returns() {
        let mut turn = assembler();
        turn.speech_started(ms(1_000));
        let (first, _, first_to) = request(&turn.pause_started(ms(2_000), ms(2_250)));
        // Speaks again and pauses again while the first request is still running.
        turn.speech_started(ms(2_400));
        let actions = turn.pause_started(ms(3_500), ms(3_750));
        assert!(actions.is_empty(), "one transcription at a time");

        let actions = turn.transcribed(first, "First part", ms(3_800));
        let (second, from, to) = request(&actions);
        assert_eq!((from, to), (first_to, ms(3_700)));
        assert!(
            commit(&actions).is_none(),
            "the second part is not transcribed yet"
        );

        let actions = turn.transcribed(second, "second part.", ms(4_000));
        assert_eq!(
            commit(&actions).map(|(text, _)| text),
            Some("First part second part.")
        );
    }

    #[test]
    fn the_wait_restarts_from_the_latest_pause() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        let (first, _, _) = request(&turn.pause_started(ms(1_000), ms(1_250)));
        turn.transcribed(first, "Hang on.", ms(1_300));
        // Speaks again at 1.35 s, just before the first pause would have ended the turn.
        turn.speech_started(ms(1_350));
        assert!(
            turn.tick(ms(1_500)).is_empty(),
            "the turn is open while the user speaks"
        );

        let (second, _, _) = request(&turn.pause_started(ms(2_000), ms(2_250)));
        let actions = turn.transcribed(second, "What time is it?", ms(2_300));
        assert!(
            commit(&actions).is_none(),
            "only 300 ms since the latest pause"
        );
        assert!(commit(&turn.tick(ms(2_400))).is_some());
    }

    #[test]
    fn noise_with_no_words_is_abandoned_without_a_reply() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        let (id, _, _) = request(&turn.pause_started(ms(500), ms(750)));

        let actions = turn.transcribed(id, "  ", ms(900));
        assert!(
            !actions.contains(&Action::Abandon),
            "wait in case words follow"
        );
        assert_eq!(turn.tick(ms(500 + 700)), vec![Action::Abandon]);
        assert!(!turn.in_turn());
    }

    #[test]
    fn a_failed_transcription_abandons_the_turn() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        let (id, _, _) = request(&turn.pause_started(ms(500), ms(750)));

        assert_eq!(turn.transcription_failed(id), vec![Action::Abandon]);
        assert!(!turn.in_turn());
        // A stale failure or result afterwards does nothing.
        assert!(turn.transcription_failed(id).is_empty());
        assert!(turn.transcribed(id, "late", ms(2_000)).is_empty());
    }

    #[test]
    fn a_transcript_from_before_a_reset_cannot_leak_into_the_next_turn() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        let (stale, _, _) = request(&turn.pause_started(ms(500), ms(750)));
        // The assistant was interrupted or the conversation moved on.
        turn.reset();

        turn.speech_started(ms(5_000));
        let (fresh, _, _) = request(&turn.pause_started(ms(6_000), ms(6_250)));
        assert_ne!(stale, fresh);
        assert!(turn.transcribed(stale, "old words.", ms(6_300)).is_empty());

        let actions = turn.transcribed(fresh, "New words.", ms(6_500));
        assert_eq!(commit(&actions).map(|(text, _)| text), Some("New words."));
    }

    #[test]
    fn talking_without_a_break_is_cut_at_the_maximum_length() {
        let mut turn = TurnAssembler::new(
            AssemblerConfig {
                max_turn: Duration::from_secs(5),
                ..config()
            },
            judge(),
        );
        turn.speech_started(ms(400));
        assert!(turn.tick(ms(4_000)).is_empty());

        let actions = turn.tick(ms(5_000));
        let (id, from, to) = request(&actions);
        assert_eq!((from, to), (0, ms(5_000)));
        // A forced end does not wait for silence or for the words to read as finished.
        let actions = turn.transcribed(id, "and so on and", ms(5_600));
        assert_eq!(
            commit(&actions).map(|(text, _)| text),
            Some("and so on and")
        );
    }

    #[test]
    fn released_audio_is_never_asked_for_again() {
        let mut turn = assembler();
        turn.speech_started(ms(1_000));
        let mut released = 0;
        let mut now = ms(1_000);
        for phrase in 0..5 {
            now += ms(1_500);
            let actions = turn.pause_started(now, now + ms(250));
            let (id, from, _) = request(&actions);
            assert!(
                from >= released,
                "phrase {phrase} asked for audio already released"
            );
            for action in turn.transcribed(id, "words", now + ms(300)) {
                if let Action::Release { before } = action {
                    released = before;
                }
            }
            turn.speech_started(now + ms(320));
        }
        assert!(released > ms(7_000));
    }

    #[test]
    fn events_outside_a_turn_do_nothing() {
        let mut turn = assembler();

        assert!(turn.pause_started(ms(100), ms(350)).is_empty());
        assert!(turn.tick(ms(10_000)).is_empty());
        assert!(turn.transcribed(1, "words", ms(500)).is_empty());
        assert!(!turn.in_turn());
    }
}
