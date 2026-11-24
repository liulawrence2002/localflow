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
//! - When a turn model is in use (Smart Turn, which hears the voice rather than the words), it
//!   is asked about each pause at the same time, and its answer moves the wait: see
//!   [`combined_readiness`]. Its answer belongs to that one pause; if the user speaks again it
//!   is thrown away, and a late one never holds the turn up.
//!
//! It is pure. Time is counted in audio samples, not read from a clock; the caller feeds it
//! what the speech detector saw and what the recogniser and the turn model returned, and
//! carries out the actions it gives back. That makes every interleaving of pauses,
//! resumptions and late answers unit-testable.

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

/// Judges a transcript. Injected so tests can script it. It reads words only; what the turn
/// model heard is the second input to the decision, applied by [`combined_readiness`].
pub type ReadinessJudge = Arc<dyn Fn(&str) -> Readiness + Send + Sync>;

/// Above this, the turn model's probability means the speaker sounds finished: Pipecat's own
/// threshold, and the one measured in `docs/benchmarks/TURN_JUDGE_BENCHMARK.md`.
pub const TURN_MODEL_THRESHOLD: f32 = 0.5;

/// What the turn model has said about the latest pause.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TurnSignal {
    /// There is no model, or it failed on this pause: the words decide alone.
    Unavailable,
    /// The model is still listening to this pause.
    Pending,
    /// The model's probability, from 0 to 1, that the turn is finished.
    Heard(f32),
}

/// How finished the turn is, from what its words say and what its voice sounded like.
///
/// The voice decides and the words move its answer one step. This is the rule the benchmark
/// found best: cut-offs fell from 28 % to 4.5 % of unfinished turns, and finished turns were
/// answered a little sooner.
///
/// - It sounds finished (above [`TURN_MODEL_THRESHOLD`]): ready, or only unsure when the
///   words trail off.
/// - It does not: not ready, or only unsure when the words read as finished.
/// - No model, or it failed: the words alone, exactly as without one.
/// - Still listening: the words alone, except that words that read as finished get the
///   unsure wait instead of the short one. That is the call the model most often overturns
///   (a full stop the recogniser put where the speaker only took a breath), and it bounds the
///   wait at what an unsure turn waits anyway. If the answer arrives meanwhile it decides from
///   then on; if not, the turn ends on the words, and nothing waits longer for the model.
pub fn combined_readiness(words: Readiness, signal: TurnSignal) -> Readiness {
    use Readiness::*;
    match signal {
        TurnSignal::Unavailable => words,
        TurnSignal::Heard(probability) if probability > TURN_MODEL_THRESHOLD => {
            if words == NotReady {
                Unsure
            } else {
                Ready
            }
        }
        TurnSignal::Heard(_) => {
            if words == Ready {
                Unsure
            } else {
                NotReady
            }
        }
        TurnSignal::Pending => {
            if words == Ready {
                Unsure
            } else {
                words
            }
        }
    }
}

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
    /// Whether a turn model is listening, so each pause is also sent to it.
    pub turn_model: bool,
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
    /// Ask the turn model whether the turn in `from..to` (everything so far, ending at this
    /// pause) sounds finished, and report back with this `pause`.
    JudgeTurn { pause: u64, from: u64, to: u64 },
    /// The turn is over: this is everything the user said.
    Commit {
        text: String,
        /// Where the user's last word ended.
        speech_end: u64,
        /// Silence that had passed when the turn was ended.
        waited: Duration,
        /// Whether the turn model heard the last pause as finished; `None` when it had not
        /// answered or there is none. For diagnostics only.
        sounded_finished: Option<bool>,
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
    /// Number of the latest pause; a turn model's answer counts only for it.
    pause: u64,
    /// What the turn model said about that pause.
    signal: TurnSignal,
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
            pause: 0,
            signal: TurnSignal::Unavailable,
        }
    }

    pub fn in_turn(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// Everything transcribed in the turn so far: what the user has said up to the last
    /// pause whose transcript is back. Empty outside a turn.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Forget the turn in progress. A transcription still in flight will be ignored.
    pub fn reset(&mut self) {
        self.phase = Phase::Idle;
        self.text.clear();
        self.pending = None;
        self.signal = TurnSignal::Unavailable;
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
            // The user carried on after a pause: the turn simply continues, and what the turn
            // model said about that pause no longer applies.
            Phase::Paused { .. } => {
                self.phase = Phase::Speaking;
                self.signal = TurnSignal::Unavailable;
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
        self.pause += 1;
        let mut actions = Vec::new();
        if self.config.turn_model {
            // Heard alongside the transcription: everything since the turn began, ending
            // where the recogniser's phrase ends.
            self.signal = TurnSignal::Pending;
            actions.push(Action::JudgeTurn {
                pause: self.pause,
                from: self.turn_start,
                to: self.phrase_end(speech_end, now),
            });
        } else {
            self.signal = TurnSignal::Unavailable;
        }
        actions.extend(self.request_transcription(now));
        actions.extend(self.try_commit(now));
        actions
    }

    /// The turn model answered for `pause`: the probability that the turn is finished, or
    /// `None` when it could not judge it. An answer for any pause but the latest, or after the
    /// user spoke again, is ignored.
    pub fn turn_judged(&mut self, pause: u64, probability: Option<f32>, now: u64) -> Vec<Action> {
        let current = pause == self.pause
            && matches!(self.phase, Phase::Paused { forced: false, .. })
            && self.signal == TurnSignal::Pending;
        if !current {
            return Vec::new();
        }
        self.signal = probability.map_or(TurnSignal::Unavailable, TurnSignal::Heard);
        self.try_commit(now)
    }

    /// Where a phrase that paused at `speech_end` ends: a little silence after the last word
    /// helps the recogniser, but never past the present.
    fn phrase_end(&self, speech_end: u64, now: u64) -> u64 {
        (speech_end + self.config.samples(self.config.tail)).min(now.max(speech_end))
    }

    /// Time has passed with nothing new from the detector.
    pub fn tick(&mut self, now: u64) -> Vec<Action> {
        match self.phase {
            Phase::Idle => Vec::new(),
            Phase::Speaking => {
                if now.saturating_sub(self.turn_start) < self.config.samples(self.config.max_turn) {
                    return Vec::new();
                }
                // Talking without a break for the maximum length: end the turn here, without
                // waiting for anything.
                self.phase = Phase::Paused {
                    speech_end: now,
                    forced: true,
                };
                self.signal = TurnSignal::Unavailable;
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
        let to = self.phrase_end(speech_end, now);
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
            match combined_readiness((self.judge)(&self.text), self.signal) {
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
        let sounded_finished = match self.signal {
            TurnSignal::Heard(probability) => Some(probability > TURN_MODEL_THRESHOLD),
            TurnSignal::Unavailable | TurnSignal::Pending => None,
        };
        self.reset();
        vec![Action::Commit {
            text,
            speech_end,
            waited,
            sounded_finished,
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
            turn_model: false,
        }
    }

    /// An assembler whose pauses are also sent to a turn model.
    fn listening_assembler() -> TurnAssembler {
        TurnAssembler::new(
            AssemblerConfig {
                turn_model: true,
                ..config()
            },
            judge(),
        )
    }

    /// The turn-model request among `actions`: its pause number and range.
    fn judge_request(actions: &[Action]) -> (u64, u64, u64) {
        actions
            .iter()
            .find_map(|action| match action {
                Action::JudgeTurn { pause, from, to } => Some((*pause, *from, *to)),
                _ => None,
            })
            .expect("a turn-model request")
    }

    fn sounded_finished(actions: &[Action]) -> Option<Option<bool>> {
        actions.iter().find_map(|action| match action {
            Action::Commit {
                sounded_finished, ..
            } => Some(*sounded_finished),
            _ => None,
        })
    }

    #[test]
    fn the_voice_decides_and_the_words_move_it_one_step() {
        use Readiness::*;
        let finished = TurnSignal::Heard(0.9);
        let unfinished = TurnSignal::Heard(0.1);
        for (words, signal, expected) in [
            // It sounds finished.
            (Ready, finished, Ready),
            (Unsure, finished, Ready),
            (NotReady, finished, Unsure),
            // It does not.
            (Ready, unfinished, Unsure),
            (Unsure, unfinished, NotReady),
            (NotReady, unfinished, NotReady),
            // No model, or it failed: the words alone.
            (Ready, TurnSignal::Unavailable, Ready),
            (Unsure, TurnSignal::Unavailable, Unsure),
            (NotReady, TurnSignal::Unavailable, NotReady),
            // Still listening: a finished reading waits as an unsure one, nothing else moves.
            (Ready, TurnSignal::Pending, Unsure),
            (Unsure, TurnSignal::Pending, Unsure),
            (NotReady, TurnSignal::Pending, NotReady),
        ] {
            assert_eq!(
                combined_readiness(words, signal),
                expected,
                "{words:?} with {signal:?}"
            );
        }
        // The threshold itself counts as not finished, as in the benchmark.
        assert_eq!(
            combined_readiness(Unsure, TurnSignal::Heard(TURN_MODEL_THRESHOLD)),
            NotReady
        );
        assert_eq!(
            combined_readiness(Unsure, TurnSignal::Heard(0.500_1)),
            Ready
        );
    }

    #[test]
    fn each_pause_is_sent_to_the_turn_model_with_the_whole_turn_so_far() {
        let mut turn = listening_assembler();
        turn.speech_started(ms(1_000));
        let actions = turn.pause_started(ms(3_000), ms(3_250));
        // From the turn's start (lead-in included) to where the phrase sent for
        // transcription ends.
        assert_eq!(judge_request(&actions), (1, ms(600), ms(3_200)));
        assert_eq!(request(&actions).2, ms(3_200));

        let (id, _, _) = request(&actions);
        turn.transcribed(id, "And then", ms(3_300));
        turn.speech_started(ms(3_400));
        let actions = turn.pause_started(ms(5_000), ms(5_250));
        assert_eq!(judge_request(&actions), (2, ms(600), ms(5_200)));
        // Without a turn model nothing is sent.
        let mut plain = assembler();
        plain.speech_started(ms(1_000));
        assert!(!plain
            .pause_started(ms(3_000), ms(3_250))
            .iter()
            .any(|action| matches!(action, Action::JudgeTurn { .. })));
    }

    #[test]
    fn a_voice_that_sounds_finished_ends_an_unsure_turn_at_the_short_wait() {
        let mut turn = listening_assembler();
        turn.speech_started(ms(0));
        let actions = turn.pause_started(ms(2_000), ms(2_250));
        let (pause, _, _) = judge_request(&actions);
        let (id, _, _) = request(&actions);
        turn.transcribed(id, "maybe the blue one", ms(2_300));
        assert!(commit(&turn.tick(ms(2_399))).is_none());

        // The answer arrives late, after the short wait has passed: the turn ends at once.
        let actions = turn.turn_judged(pause, Some(0.97), ms(2_420));
        assert_eq!(
            commit(&actions).map(|(text, _)| text),
            Some("maybe the blue one")
        );
        assert_eq!(sounded_finished(&actions), Some(Some(true)));
    }

    #[test]
    fn a_voice_that_sounds_unfinished_holds_a_finished_reading_to_the_unsure_wait() {
        let mut turn = listening_assembler();
        turn.speech_started(ms(0));
        let actions = turn.pause_started(ms(2_000), ms(2_250));
        let (pause, _, _) = judge_request(&actions);
        let (id, _, _) = request(&actions);
        assert!(turn.turn_judged(pause, Some(0.02), ms(2_260)).is_empty());
        // The recogniser put a full stop where the speaker only took a breath.
        turn.transcribed(id, "I think we should.", ms(2_300));

        assert!(commit(&turn.tick(ms(2_400))).is_none(), "the short wait");
        assert!(commit(&turn.tick(ms(2_699))).is_none());
        let actions = turn.tick(ms(2_700));
        assert!(commit(&actions).is_some());
        assert_eq!(sounded_finished(&actions), Some(Some(false)));
    }

    #[test]
    fn an_answer_for_an_earlier_pause_is_thrown_away() {
        let mut turn = listening_assembler();
        turn.speech_started(ms(0));
        let first = turn.pause_started(ms(1_000), ms(1_250));
        let (old_pause, _, _) = judge_request(&first);
        let (id, _, _) = request(&first);
        turn.transcribed(id, "What time", ms(1_300));
        // The user carries on; the answer about the first pause arrives while they talk.
        turn.speech_started(ms(1_350));
        assert!(turn
            .turn_judged(old_pause, Some(0.99), ms(1_400))
            .is_empty());

        let second = turn.pause_started(ms(2_000), ms(2_250));
        let (pause, _, _) = judge_request(&second);
        assert_ne!(pause, old_pause);
        let (id, _, _) = request(&second);
        turn.transcribed(id, "is it?", ms(2_300));
        // Still about the first pause, arriving even later: ignored again.
        assert!(turn
            .turn_judged(old_pause, Some(0.99), ms(2_420))
            .is_empty());
        // With the second pause's answer still out, the finished reading waits the unsure time.
        assert!(commit(&turn.tick(ms(2_420))).is_none());
        assert!(commit(&turn.turn_judged(pause, Some(0.05), ms(2_500))).is_none());
        assert!(commit(&turn.tick(ms(2_699))).is_none());
        assert!(commit(&turn.tick(ms(2_700))).is_some());
    }

    #[test]
    fn a_late_or_missing_answer_never_holds_the_turn_up() {
        // No answer at all: the words decide, a finished reading at the unsure wait.
        let mut turn = listening_assembler();
        turn.speech_started(ms(0));
        let actions = turn.pause_started(ms(2_000), ms(2_250));
        let (pause, _, _) = judge_request(&actions);
        let (id, _, _) = request(&actions);
        turn.transcribed(id, "What time is it?", ms(2_300));
        let actions = turn.tick(ms(2_700));
        assert!(commit(&actions).is_some());
        assert_eq!(sounded_finished(&actions), Some(None));
        // The answer turning up afterwards changes nothing.
        assert!(turn.turn_judged(pause, Some(0.0), ms(2_800)).is_empty());

        // An unclear reading waits no longer than without a model.
        let mut turn = listening_assembler();
        turn.speech_started(ms(0));
        let (id, _, _) = request(&turn.pause_started(ms(2_000), ms(2_250)));
        turn.transcribed(id, "maybe the blue one", ms(2_300));
        assert!(commit(&turn.tick(ms(2_700))).is_some());
    }

    #[test]
    fn a_failed_answer_leaves_the_words_to_decide_alone() {
        let mut turn = listening_assembler();
        turn.speech_started(ms(0));
        let actions = turn.pause_started(ms(2_000), ms(2_250));
        let (pause, _, _) = judge_request(&actions);
        let (id, _, _) = request(&actions);
        assert!(turn.turn_judged(pause, None, ms(2_260)).is_empty());
        turn.transcribed(id, "What time is it?", ms(2_300));
        // A finished reading gets the short wait again, as without a model.
        assert!(commit(&turn.tick(ms(2_400))).is_some());
    }

    #[test]
    fn a_turn_cut_at_its_maximum_length_waits_for_nothing() {
        let mut turn = TurnAssembler::new(
            AssemblerConfig {
                max_turn: Duration::from_secs(5),
                turn_model: true,
                ..config()
            },
            judge(),
        );
        turn.speech_started(ms(400));
        let actions = turn.tick(ms(5_000));
        assert!(!actions
            .iter()
            .any(|action| matches!(action, Action::JudgeTurn { .. })));
        let (id, _, _) = request(&actions);
        assert!(commit(&turn.transcribed(id, "and so on and", ms(5_100))).is_some());
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
    fn the_words_so_far_can_be_read_before_the_turn_ends() {
        let mut turn = assembler();
        turn.speech_started(ms(0));
        assert_eq!(turn.text(), "");
        let (first, _, _) = request(&turn.pause_started(ms(1_000), ms(1_250)));
        turn.transcribed(first, "Wait,", ms(1_300));
        assert_eq!(turn.text(), "Wait,");

        turn.speech_started(ms(1_320));
        let (second, _, _) = request(&turn.pause_started(ms(2_000), ms(2_250)));
        let actions = turn.transcribed(second, "what about deadlocks?", ms(2_300));
        assert!(commit(&actions).is_none());
        assert_eq!(turn.text(), "Wait, what about deadlocks?");

        // Once the turn is over there is nothing in progress.
        assert!(commit(&turn.tick(ms(2_400))).is_some());
        assert_eq!(turn.text(), "");
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
