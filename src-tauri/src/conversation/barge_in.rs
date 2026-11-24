//! Tells a real interruption from "mm-hmm", and decides how long a held reply waits.
//!
//! When the user starts to speak while the assistant has the floor, the reply is held: its
//! audio is paused, not thrown away, and it goes on being written (LiveKit Agents'
//! `resume_false_interruption`). What the user said is then judged on its words, as soon as
//! the first of them are transcribed:
//!
//! - **real**: "stop", "wait" or "no", any control phrase ("never mind", "say that again",
//!   "goodbye"), or at least two words that are neither fillers nor the assistant's own. The
//!   reply is cancelled and what the user is saying becomes their next turn.
//! - **backchannel**: nothing, fillers, or a listener's acknowledgement ("mm-hmm", "yeah",
//!   "okay", "got it"). The reply carries on where it stopped.
//! - **echo**: the assistant's own words heard back through the microphone. It carries on.
//!
//! The two-word minimum applies only while the reply is being heard, as in Pipecat. Speech
//! that begins before any of the reply has played (while it is still being prepared) is the
//! user carrying on with their own turn, and one word of substance is enough. Once a reply has
//! ended none of this applies: the user has the floor, and "yeah" is an answer.
//!
//! Pure. Time is counted in samples on the caller's audio clock, so every timing is tested.

use std::time::Duration;

use super::{
    echo_guard::{is_echo_of, is_fragment_of},
    router::{normalize, route_turn, ReadWhat, TurnIntent},
    turn_completion::FILLERS,
};
use crate::coding_agents::{
    dialog::{classify_reply, Reply},
    UserSpeech,
};

/// What the words heard over a held reply amount to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interruption {
    /// The user is taking the floor: stop the reply.
    Real,
    /// Nothing, fillers, or a listener's acknowledgement: the reply carries on.
    Backchannel,
    /// The assistant's own words, heard back: the reply carries on.
    Echo,
    /// One word of substance over a reply being heard: too little to stop it for, but it may
    /// be the start of more. Decided by what follows, or by the user going quiet.
    TooShort,
}

/// Words of substance needed to stop a reply that is being heard.
const MIN_INTERRUPTION_WORDS: usize = 2;

/// Words that stop a reply on their own.
const STOP_WORDS: &[&str] = &[
    "stop", "wait", "no", "nope", "pause", "hold", "halt", "quiet", "shh", "shush", "enough",
    "cancel",
];

/// Sounds that carry no words, beyond the hesitations in [`FILLERS`].
const MORE_FILLERS: &[&str] = &[
    "mmm", "mhm", "mmhm", "mmhmm", "mhmm", "uhhuh", "ahh", "aha", "oh", "ooh", "huh",
];

/// What a listener says to show they are following, not to take the floor. Matched as
/// whole runs of words, longest first, so "I got it" is one acknowledgement and not "I" and
/// "got it".
const BACKCHANNELS: &[&[&str]] = &[
    &["that", "makes", "sense"],
    &["i", "got", "it"],
    &["got", "it"],
    &["i", "see"],
    &["i", "know"],
    &["makes", "sense"],
    &["fair", "enough"],
    &["of", "course"],
    &["sounds", "good"],
    &["all", "right"],
    &["go", "on"],
    &["go", "ahead"],
    &["keep", "going"],
    &["carry", "on"],
    &["keep", "reading"],
    &["yeah"],
    &["yea"],
    &["yep"],
    &["yup"],
    &["yes"],
    &["okay"],
    &["ok"],
    &["alright"],
    &["right"],
    &["sure"],
    &["cool"],
    &["nice"],
    &["great"],
    &["wow"],
    &["exactly"],
    &["true"],
    &["totally"],
    &["absolutely"],
    &["definitely"],
    &["indeed"],
    &["interesting"],
    &["really"],
    &["gotcha"],
    &["continue"],
];

/// Judge what the user said while the assistant had the floor.
///
/// `heard` is everything transcribed since they started; `speaking` is the text of the reply
/// being spoken. `over_reply` is true when the reply was being heard as they started, which
/// is when the two-word minimum and the echo checks apply; false when it was still being
/// prepared.
pub fn judge_interruption(heard: &str, speaking: &str, over_reply: bool) -> Interruption {
    let normalized = normalize(heard);
    if normalized.is_empty() {
        return Interruption::Backchannel;
    }
    if over_reply && is_echo_of(heard, speaking) {
        return Interruption::Echo;
    }
    if is_command(heard) {
        return Interruption::Real;
    }

    let words: Vec<&str> = normalized.split(' ').collect();
    let content = content_words(&words);
    if content.is_empty() {
        return Interruption::Backchannel;
    }
    if content.iter().all(|word| STOP_WORDS.contains(word)) {
        return Interruption::Real;
    }
    if !over_reply {
        return Interruption::Real;
    }
    let phrase = content.join(" ");
    if is_fragment_of(&phrase, speaking) || is_echo_of(&phrase, speaking) {
        return Interruption::Echo;
    }
    if content.len() >= MIN_INTERRUPTION_WORDS {
        Interruption::Real
    } else {
        Interruption::TooShort
    }
}

/// What speech while a coding agent works on a question amounts to. Only asking to stop it
/// counts: the user was told "say stop that to cancel", and a question that takes a minute
/// must not be lost to a remark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhileAgentWorks {
    /// "Stop that", "cancel", "never mind": stop the agent.
    Stop,
    /// Anything else with words in it: the agent carries on, and the user is reminded how to
    /// stop it.
    Other,
    /// Nothing, a noise, an acknowledgement, or the assistant's own voice.
    Nothing,
}

/// Judge what the user said while a coding agent works. `over_reply` when the assistant was
/// speaking as they started (saying "Claude is still working"), so its own words are checked
/// for first: it must never be able to stop the agent with its own voice.
pub fn judge_while_agent_works(heard: &str, speaking: &str, over_reply: bool) -> WhileAgentWorks {
    let normalized = normalize(heard);
    if normalized.is_empty() || (over_reply && is_echo_of(heard, speaking)) {
        return WhileAgentWorks::Nothing;
    }
    let words: Vec<&str> = normalized.split(' ').collect();
    let content = content_words(&words);
    if content.is_empty() {
        return WhileAgentWorks::Nothing;
    }
    let phrase = content.join(" ");
    if over_reply && (is_fragment_of(&phrase, speaking) || is_echo_of(&phrase, speaking)) {
        return WhileAgentWorks::Nothing;
    }
    let speech = UserSpeech::from_user_transcript(heard);
    if classify_reply(speech) == Reply::No || route_turn(heard) == TurnIntent::Quiet {
        WhileAgentWorks::Stop
    } else {
        WhileAgentWorks::Other
    }
}

/// A control phrase that means "stop what you are doing": quiet, goodbye, repeat, or a move
/// around the page being read. "Go on" and "keep reading" mean the opposite.
fn is_command(heard: &str) -> bool {
    matches!(
        route_turn(heard),
        TurnIntent::End
            | TurnIntent::Quiet
            | TurnIntent::Repeat
            | TurnIntent::Read(
                ReadWhat::FromStart | ReadWhat::Next | ReadWhat::Previous | ReadWhat::Again
            )
    )
}

/// The words left once fillers and acknowledgements are taken out.
fn content_words<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut content = Vec::new();
    let mut index = 0;
    while index < words.len() {
        let rest = &words[index..];
        if let Some(phrase) = BACKCHANNELS.iter().find(|phrase| rest.starts_with(phrase)) {
            index += phrase.len();
            continue;
        }
        let word = words[index];
        if !FILLERS.contains(&word) && !MORE_FILLERS.contains(&word) {
            content.push(word);
        }
        index += 1;
    }
    content
}

/// Timings of a hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HoldConfig {
    /// Sample rate of the audio clock.
    pub sample_rate: u32,
    /// Silence after the user stops that, with nothing decided, lets the reply carry on.
    pub false_interruption_timeout: Duration,
    /// A hold never lasts longer than this. Someone still talking after it is interrupting.
    pub max_hold: Duration,
}

impl HoldConfig {
    fn samples(&self, duration: Duration) -> u64 {
        (duration.as_secs_f64() * self.sample_rate as f64) as u64
    }
}

/// What to do with a held reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldDecision {
    /// Keep holding: nothing is decided yet.
    Wait,
    /// The user is interrupting: cancel the reply.
    Interrupt,
    /// A false interruption: let the reply carry on where it stopped.
    Resume,
    /// While a coding agent works: the reply carries on, and the agent is to be stopped.
    StopAgent,
    /// While a coding agent works: the reply carries on, and the user is to be told the agent
    /// is still working and how to stop it.
    RemindAgentBusy,
}

/// One held reply: what has been heard over it, and for how long.
#[derive(Debug, Clone)]
pub struct HeldReply {
    config: HoldConfig,
    over_reply: bool,
    /// The reply is waiting for a coding agent's answer; see [`judge_while_agent_works`].
    agent_working: bool,
    /// Where on the audio clock the hold began.
    began: u64,
    /// Where the user's speech last ended, while they are quiet.
    quiet_since: Option<u64>,
    verdict: Option<Interruption>,
    agent_verdict: Option<WhileAgentWorks>,
}

impl HeldReply {
    /// A hold beginning at `began`. `over_reply` as for [`judge_interruption`].
    pub fn new(config: HoldConfig, began: u64, over_reply: bool) -> Self {
        Self {
            config,
            over_reply,
            agent_working: false,
            began,
            quiet_since: None,
            verdict: None,
            agent_verdict: None,
        }
    }

    /// A hold while the reply waits for a coding agent. Only a request to stop the agent
    /// does anything; it is never cancelled as an interruption.
    pub fn while_agent_works(config: HoldConfig, began: u64, over_reply: bool) -> Self {
        Self {
            agent_working: true,
            ..Self::new(config, began, over_reply)
        }
    }

    /// Whether the reply was being heard when the user started.
    pub fn over_reply(&self) -> bool {
        self.over_reply
    }

    /// Whether this hold is over a reply waiting for a coding agent.
    pub fn agent_working(&self) -> bool {
        self.agent_working
    }

    /// The coding agent has answered while the user was talking: its answer is about to be
    /// spoken, so from now on what they say is judged as over any reply. "Stop that" then
    /// stops the answer being spoken, where it would only have stopped an agent that had
    /// already finished.
    pub fn agent_finished(&mut self) {
        self.agent_working = false;
    }

    /// The latest judgment of what was said, for diagnostics. Never contains any words.
    pub fn verdict(&self) -> Option<Interruption> {
        self.verdict
    }

    /// The latest judgment while an agent works, for diagnostics. Never contains any words.
    pub fn agent_verdict(&self) -> Option<WhileAgentWorks> {
        self.agent_verdict
    }

    fn judge_agent_speech(&mut self, heard: &str, speaking: &str) -> WhileAgentWorks {
        let verdict = judge_while_agent_works(heard, speaking, self.over_reply);
        self.agent_verdict = Some(verdict);
        verdict
    }

    /// The user stopped speaking at `speech_end`.
    pub fn user_paused(&mut self, speech_end: u64) {
        self.quiet_since = Some(speech_end);
    }

    /// The user is speaking again.
    pub fn user_resumed(&mut self) {
        self.quiet_since = None;
    }

    /// Everything said since the hold began has been transcribed as far as `heard`.
    /// `more_coming` is true when speech after it is still being transcribed.
    pub fn heard(&mut self, heard: &str, speaking: &str, more_coming: bool) -> HoldDecision {
        let settled = self.quiet_since.is_some() && !more_coming;
        if self.agent_working {
            return match self.judge_agent_speech(heard, speaking) {
                WhileAgentWorks::Stop => HoldDecision::StopAgent,
                WhileAgentWorks::Other if settled => HoldDecision::RemindAgentBusy,
                WhileAgentWorks::Nothing if settled => HoldDecision::Resume,
                _ => HoldDecision::Wait,
            };
        }
        let verdict = judge_interruption(heard, speaking, self.over_reply);
        self.verdict = Some(verdict);
        match verdict {
            Interruption::Real => HoldDecision::Interrupt,
            // Carry on at once, unless the user is still talking or has said more since.
            Interruption::Backchannel | Interruption::Echo
                if self.quiet_since.is_some() && !more_coming =>
            {
                HoldDecision::Resume
            }
            _ => HoldDecision::Wait,
        }
    }

    /// The user's speech has ended for good with `heard`: decide now.
    pub fn finished(&mut self, heard: &str, speaking: &str) -> HoldDecision {
        if self.agent_working {
            return match self.judge_agent_speech(heard, speaking) {
                WhileAgentWorks::Stop => HoldDecision::StopAgent,
                WhileAgentWorks::Other => HoldDecision::RemindAgentBusy,
                WhileAgentWorks::Nothing => HoldDecision::Resume,
            };
        }
        let verdict = judge_interruption(heard, speaking, self.over_reply);
        self.verdict = Some(verdict);
        if verdict == Interruption::Real {
            HoldDecision::Interrupt
        } else {
            HoldDecision::Resume
        }
    }

    /// It is now `now` on the audio clock.
    pub fn tick(&mut self, now: u64) -> HoldDecision {
        if now.saturating_sub(self.began) >= self.config.samples(self.config.max_hold) {
            // Long talk is not a reason to throw away an agent's answer.
            return if self.agent_working {
                HoldDecision::Resume
            } else {
                HoldDecision::Interrupt
            };
        }
        match self.quiet_since {
            Some(quiet) => {
                let waited = now.saturating_sub(quiet);
                if waited >= self.config.samples(self.config.false_interruption_timeout) {
                    HoldDecision::Resume
                } else {
                    HoldDecision::Wait
                }
            }
            None => HoldDecision::Wait,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = "A mutex is a lock that lets only one thread touch a piece of data \
        at a time, which keeps two threads from changing it at once.";

    fn over_reply(heard: &str) -> Interruption {
        judge_interruption(heard, REPLY, true)
    }

    #[test]
    fn a_single_stop_word_or_a_control_phrase_is_enough_to_interrupt() {
        for heard in [
            "Stop.",
            "Wait!",
            "No.",
            "no no",
            "Hold on.",
            "Um, wait.",
            "Okay, stop.",
            "Never mind.",
            "That's enough.",
            "Say that again?",
            "Pardon?",
            "Goodbye.",
            "Next paragraph.",
            "Skip.",
            "Go back.",
        ] {
            assert_eq!(over_reply(heard), Interruption::Real, "{heard}");
        }
    }

    #[test]
    fn two_words_of_substance_interrupt() {
        for heard in [
            "What about deadlocks?",
            "Actually, what time is it?",
            "Yeah, but why?",
            "Okay so tell me more about semaphores.",
            "I see what you mean, but no.",
            "Thank you.",
        ] {
            assert_eq!(over_reply(heard), Interruption::Real, "{heard}");
        }
    }

    #[test]
    fn listening_noises_and_acknowledgements_are_backchannels() {
        for heard in [
            "",
            "   ",
            "...",
            "Mm-hmm.",
            "Mhm.",
            "Uh-huh.",
            "Hmm.",
            "Um, uh.",
            "Yeah.",
            "Yeah, yeah.",
            "Yep.",
            "Okay.",
            "OK.",
            "Oh, okay.",
            "Right.",
            "Sure.",
            "Got it.",
            "I got it.",
            "I see.",
            "Makes sense.",
            "Go on.",
            "Keep going.",
            "Continue.",
        ] {
            assert_eq!(over_reply(heard), Interruption::Backchannel, "{heard:?}");
        }
    }

    #[test]
    fn the_assistants_own_words_are_echo_however_few() {
        assert_eq!(
            over_reply("lets only one thread touch a piece"),
            Interruption::Echo
        );
        // Too short for the full check, but side by side in the reply.
        assert_eq!(over_reply("a lock"), Interruption::Echo);
        assert_eq!(over_reply("Mutex."), Interruption::Echo);
        // Fillers around an echo do not hide it.
        assert_eq!(over_reply("um, two threads"), Interruption::Echo);
    }

    #[test]
    fn the_users_stop_wins_even_when_the_reply_says_it_too() {
        let reply = "No, you cannot stop a thread that holds the lock. Wait for it instead.";
        assert_eq!(judge_interruption("stop", reply, true), Interruption::Real);
        assert_eq!(judge_interruption("No.", reply, true), Interruption::Real);
        assert_eq!(judge_interruption("Wait.", reply, true), Interruption::Real);
    }

    #[test]
    fn one_other_word_over_a_reply_is_too_short_to_stop_it() {
        for heard in ["Actually,", "So", "Paris?", "Thanks."] {
            assert_eq!(over_reply(heard), Interruption::TooShort, "{heard}");
        }
    }

    #[test]
    fn before_the_reply_is_heard_one_word_of_substance_is_enough() {
        // The user carrying on while the reply is being prepared: no minimum, no echo check.
        for heard in ["France?", "Actually,", "a lock", "Please."] {
            assert_eq!(
                judge_interruption(heard, REPLY, false),
                Interruption::Real,
                "{heard}"
            );
        }
        // But noises and acknowledgements still are not words.
        for heard in ["", "Um.", "Mm-hmm.", "Okay."] {
            assert_eq!(
                judge_interruption(heard, REPLY, false),
                Interruption::Backchannel,
                "{heard:?}"
            );
        }
    }

    #[test]
    fn nothing_is_echo_when_nothing_was_being_said() {
        assert_eq!(judge_interruption("a lock", "", true), Interruption::Real);
        assert_eq!(judge_interruption("lock", "", true), Interruption::TooShort);
    }

    const RATE: u64 = 16_000;

    fn ms(milliseconds: u64) -> u64 {
        milliseconds * RATE / 1_000
    }

    fn held(over_reply: bool) -> HeldReply {
        HeldReply::new(
            HoldConfig {
                sample_rate: RATE as u32,
                false_interruption_timeout: Duration::from_millis(1_500),
                max_hold: Duration::from_secs(6),
            },
            ms(10_000),
            over_reply,
        )
    }

    #[test]
    fn a_real_interruption_cancels_the_reply_as_soon_as_its_words_arrive() {
        let mut hold = held(true);
        // Even while the user is still talking.
        assert_eq!(
            hold.heard("Wait, what about", REPLY, true),
            HoldDecision::Interrupt
        );
        assert_eq!(hold.verdict(), Some(Interruption::Real));
    }

    #[test]
    fn a_backchannel_resumes_at_once_once_the_user_is_quiet() {
        let mut hold = held(true);
        hold.user_paused(ms(10_600));
        assert_eq!(hold.heard("Mm-hmm.", REPLY, false), HoldDecision::Resume);

        let mut hold = held(true);
        hold.user_paused(ms(10_600));
        assert_eq!(hold.heard("a lock", REPLY, false), HoldDecision::Resume);
    }

    #[test]
    fn a_backchannel_waits_while_the_user_is_still_talking_or_has_said_more() {
        let mut hold = held(true);
        // They paused, then carried on before the first part was transcribed.
        hold.user_paused(ms(10_400));
        hold.user_resumed();
        assert_eq!(hold.heard("Yeah,", REPLY, false), HoldDecision::Wait);

        // They paused again, but that part is still being transcribed.
        hold.user_paused(ms(11_200));
        assert_eq!(hold.heard("Yeah,", REPLY, true), HoldDecision::Wait);
        // The rest arrives and decides it.
        assert_eq!(
            hold.heard("Yeah, but what about deadlocks?", REPLY, false),
            HoldDecision::Interrupt
        );
    }

    #[test]
    fn a_single_word_waits_for_more_then_resumes_after_the_timeout() {
        let mut hold = held(true);
        hold.user_paused(ms(10_500));
        assert_eq!(hold.heard("Actually,", REPLY, false), HoldDecision::Wait);

        assert_eq!(hold.tick(ms(10_500 + 1_499)), HoldDecision::Wait);
        assert_eq!(hold.tick(ms(10_500 + 1_500)), HoldDecision::Resume);
    }

    #[test]
    fn with_no_words_at_all_the_reply_resumes_after_the_timeout_from_going_quiet() {
        // The hold began at 10 s.
        let mut hold = held(true);
        // Talking: no timeout runs, however long it takes (up to the cap).
        assert_eq!(hold.tick(ms(11_000)), HoldDecision::Wait);
        hold.user_paused(ms(11_000));
        assert_eq!(hold.tick(ms(12_000)), HoldDecision::Wait);
        // Speaking again restarts it.
        hold.user_resumed();
        assert_eq!(hold.tick(ms(12_200)), HoldDecision::Wait);
        hold.user_paused(ms(12_200));
        assert_eq!(hold.tick(ms(13_000)), HoldDecision::Wait);
        assert_eq!(hold.tick(ms(12_200 + 1_500)), HoldDecision::Resume);
    }

    #[test]
    fn a_hold_that_lasts_too_long_becomes_an_interruption() {
        let mut hold = held(true);
        assert_eq!(hold.tick(ms(10_000 + 5_999)), HoldDecision::Wait);
        assert_eq!(hold.tick(ms(10_000 + 6_000)), HoldDecision::Interrupt);
    }

    #[test]
    fn when_the_speech_is_over_anything_but_a_real_interruption_resumes() {
        for (heard, decision) in [
            ("Okay.", HoldDecision::Resume),
            ("Actually,", HoldDecision::Resume),
            ("a lock", HoldDecision::Resume),
            ("", HoldDecision::Resume),
            ("Hang on a second.", HoldDecision::Interrupt),
        ] {
            assert_eq!(held(true).finished(heard, REPLY), decision, "{heard:?}");
        }
        // Before the reply was heard, one word is enough.
        assert_eq!(
            held(false).finished("France?", REPLY),
            HoldDecision::Interrupt
        );
    }

    #[test]
    fn while_an_agent_works_only_asking_to_stop_it_counts() {
        let judge = |heard| judge_while_agent_works(heard, "", false);
        for heard in [
            "Stop that.",
            "Cancel.",
            "Never mind.",
            "No.",
            "Stop.",
            "Be quiet.",
        ] {
            assert_eq!(judge(heard), WhileAgentWorks::Stop, "{heard}");
        }
        for heard in [
            "What's the weather like?",
            "Open Cursor in localflow",
            "Goodbye.",
            "Thanks.",
        ] {
            assert_eq!(judge(heard), WhileAgentWorks::Other, "{heard}");
        }
        for heard in ["", "Mm-hmm.", "Okay.", "Um."] {
            assert_eq!(judge(heard), WhileAgentWorks::Nothing, "{heard:?}");
        }
    }

    #[test]
    fn the_assistants_own_reminder_cannot_stop_the_agent() {
        let reminder = "Claude is still working. Say stop that to cancel.";
        assert_eq!(
            judge_while_agent_works("stop that", reminder, true),
            WhileAgentWorks::Nothing
        );
        assert_eq!(
            judge_while_agent_works("Say stop that to cancel.", reminder, true),
            WhileAgentWorks::Nothing
        );
        // Said once the reminder is over, it does.
        assert_eq!(
            judge_while_agent_works("stop that", reminder, false),
            WhileAgentWorks::Stop
        );
    }

    #[test]
    fn a_hold_while_an_agent_works_never_cancels_the_reply() {
        let config = HoldConfig {
            sample_rate: RATE as u32,
            false_interruption_timeout: Duration::from_millis(1_500),
            max_hold: Duration::from_secs(6),
        };
        let mut hold = HeldReply::while_agent_works(config, ms(10_000), false);
        assert!(hold.agent_working());
        // "Stop that" stops the agent at once, even mid-sentence.
        assert_eq!(hold.heard("Stop that", "", true), HoldDecision::StopAgent);

        let mut hold = HeldReply::while_agent_works(config, ms(10_000), false);
        assert_eq!(
            hold.heard("What's the time?", "", false),
            HoldDecision::Wait,
            "still talking"
        );
        hold.user_paused(ms(11_000));
        assert_eq!(
            hold.heard("What's the time?", "", false),
            HoldDecision::RemindAgentBusy
        );
        assert_eq!(hold.agent_verdict(), Some(WhileAgentWorks::Other));
        assert_eq!(hold.heard("Okay.", "", false), HoldDecision::Resume);
        assert_eq!(
            hold.finished("Tell me a joke", ""),
            HoldDecision::RemindAgentBusy
        );
        assert_eq!(hold.finished("Cancel that.", ""), HoldDecision::StopAgent);
        assert_eq!(hold.finished("", ""), HoldDecision::Resume);

        // Talking on and on lets the agent carry on rather than cutting it off.
        let mut hold = HeldReply::while_agent_works(config, ms(10_000), false);
        assert_eq!(hold.tick(ms(16_000)), HoldDecision::Resume);
    }

    #[test]
    fn once_the_agent_has_answered_the_hold_is_judged_as_over_any_reply() {
        let config = HoldConfig {
            sample_rate: RATE as u32,
            false_interruption_timeout: Duration::from_millis(1_500),
            max_hold: Duration::from_secs(6),
        };
        // "Stop that" now stops the answer being spoken: a real interruption.
        let mut hold = HeldReply::while_agent_works(config, ms(10_000), false);
        hold.agent_finished();
        assert!(!hold.agent_working());
        assert_eq!(hold.heard("Stop that.", "", true), HoldDecision::Interrupt);
        assert_eq!(hold.finished("Stop that.", ""), HoldDecision::Interrupt);
        // Something else is a question again, and long talk cancels as over any reply.
        let mut hold = HeldReply::while_agent_works(config, ms(10_000), false);
        hold.agent_finished();
        assert_eq!(
            hold.finished("What's the weather?", ""),
            HoldDecision::Interrupt
        );
        assert_eq!(hold.tick(ms(16_000)), HoldDecision::Interrupt);
        let mut hold = HeldReply::while_agent_works(config, ms(10_000), false);
        hold.agent_finished();
        hold.user_paused(ms(10_500));
        assert_eq!(hold.heard("Mm-hmm.", "", false), HoldDecision::Resume);
    }

    #[test]
    fn verdicts_name_a_kind_never_the_words() {
        let mut hold = held(true);
        hold.user_paused(ms(10_400));
        hold.heard("my password is hunter2", REPLY, false);
        let shown = format!("{:?} {:?}", hold.verdict(), hold);
        assert!(!shown.contains("hunter2"));
    }
}
