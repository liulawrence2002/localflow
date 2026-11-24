//! The turn-taking state machine for conversation mode.
//!
//! A pure function from (state, event) to (state, effects). It owns no devices, threads or
//! clocks, so every transition, including the awkward interleavings, is unit-tested. The
//! coordinator feeds it events and carries out the effects it returns.
//!
//! Two counters make stale work harmless. `conversation` changes every time the mode is
//! switched on, and `turn` changes for every user turn and every interruption. Events carry
//! the counter they belong to; one that no longer matches is ignored, so a slow model or
//! voice finishing late can never speak into a newer turn or a closed conversation.
//!
//! Speech while the assistant has the floor (thinking or speaking) does not cancel its reply
//! at once. The reply is *held*: its audio paused, its words still being produced. What the
//! user said then either confirms the interruption, which cancels the reply exactly as the
//! hotkey does, or shows it was a false one ("mm-hmm", an echo, a noise), which lets the reply
//! carry on where it stopped. The speaker is paused exactly while `held` is set.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Conversation mode is off. The microphone is closed.
    Off,
    /// Opening the microphone and loading models.
    Starting,
    /// Waiting for the user to speak.
    Listening,
    /// The user is speaking.
    UserSpeaking,
    /// Transcribing the turn and waiting for the reply to begin.
    Thinking,
    /// The reply is being spoken.
    Speaking,
}

impl Phase {
    /// Whether the user has the floor: the assistant is waiting for them or hearing them.
    pub fn user_has_floor(self) -> bool {
        matches!(self, Self::Listening | Self::UserSpeaking)
    }

    /// Whether microphone audio is listened to for the user. Always while they have the
    /// floor, and while a reply is being prepared, so that carrying on speaking holds the
    /// reply instead of being lost. While a reply is spoken the microphone is only watched
    /// for the user talking over it, and only when the assistant's own voice can be removed.
    pub fn listens(self) -> bool {
        matches!(self, Self::Listening | Self::UserSpeaking | Self::Thinking)
    }

    pub fn is_active(self) -> bool {
        self != Self::Off
    }
}

/// Why a conversation ended. Shown to the user and logged; never contains speech.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The hotkey, the tray, or app exit.
    User,
    /// The user said goodbye.
    Goodbye,
    /// Nobody spoke for the idle limit.
    Idle,
    /// The conversation reached its maximum length.
    MaxDuration,
    /// The microphone or the models could not be started.
    StartFailed,
    /// The microphone stopped delivering audio.
    DeviceLost,
    /// A coding agent was opened in a terminal tab, and the conversation stepped aside so the
    /// task can be dictated into the agent's own prompt.
    AgentOpened,
}

/// What a finished user turn turned out to be, as far as the state machine cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnOutcome {
    /// Nothing to answer: silence, noise, or "never mind".
    NoReply,
    /// A reply is being produced.
    Reply,
    /// The user ended the conversation.
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The conversation hotkey was pressed.
    Toggle,
    /// End now, whatever is happening (tray item, app exit).
    EndRequested,
    Ready {
        conversation: u64,
    },
    StartFailed {
        conversation: u64,
    },
    SpeechStarted {
        conversation: u64,
    },
    /// The user finished a turn. Accepted while waiting for them too: the start of their
    /// speech may have been heard while the assistant still had the floor, which it then gave
    /// back as they talked.
    SpeechEnded {
        conversation: u64,
    },
    /// The user started speaking while the assistant had the floor: while it prepared a reply
    /// or spoke one. The reply is held, not cancelled, until their words show whether they are
    /// taking the floor.
    BargeIn {
        conversation: u64,
    },
    /// What the user said over the held reply is a real interruption. The reply is cancelled
    /// and the user has the floor: what they are saying is their next turn.
    InterruptionConfirmed {
        conversation: u64,
    },
    /// It was only "mm-hmm", the assistant's own voice, or a noise. The held reply carries on
    /// where it stopped.
    FalseInterruption {
        conversation: u64,
    },
    /// What sounded like the start of a turn held no words (a cough, a noise).
    TurnAbandoned {
        conversation: u64,
    },
    TurnRouted {
        turn: u64,
        outcome: TurnOutcome,
    },
    /// The first audio of the reply reached the speaker queue. Also the first audio after a
    /// [`Event::ReplyWaiting`].
    ReplyAudioStarted {
        turn: u64,
    },
    /// What the reply had to say so far has been said, and the rest is not ready: a coding
    /// agent is still working on the question. The assistant keeps the floor, but the
    /// microphone is listened to as while it prepares any reply.
    ReplyWaiting {
        turn: u64,
    },
    /// The reply has been spoken to the end.
    ReplyFinished {
        turn: u64,
    },
    /// The reply has been spoken to the end, and it opened a coding agent: the conversation
    /// ends so the task can be dictated into the agent.
    HandedToAgent {
        turn: u64,
    },
    /// The reply could not be produced or spoken.
    ReplyFailed {
        turn: u64,
    },
    IdleTimeout {
        conversation: u64,
    },
    MaxDuration {
        conversation: u64,
    },
    DeviceLost {
        conversation: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Open the microphone and warm the models for this conversation.
    Start { conversation: u64 },
    /// Close the microphone, cancel any turn in progress and silence the speaker.
    Stop { reason: StopReason },
    /// Transcribe the captured utterance and answer it.
    BeginTurn { turn: u64 },
    /// Cancel the model and the voice for this turn and silence the speaker, discarding
    /// what was queued.
    CancelTurn { turn: u64 },
    /// Pause the speaker, keeping what is queued; the reply goes on being produced.
    PauseReply { turn: u64 },
    /// Let the speaker play again after [`Effect::PauseReply`]. After a
    /// [`Effect::CancelTurn`] in the same transition, so nothing cancelled is heard.
    ResumeReply { turn: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationState {
    pub phase: Phase,
    pub conversation: u64,
    pub turn: u64,
    /// The user started speaking while the assistant had the floor, so its reply is held:
    /// the speaker paused, the reply still being produced, until their words are judged.
    /// Only ever set while thinking or speaking.
    pub held: bool,
}

impl Default for ConversationState {
    fn default() -> Self {
        Self {
            phase: Phase::Off,
            conversation: 0,
            turn: 0,
            held: false,
        }
    }
}

pub fn transition(state: ConversationState, event: Event) -> (ConversationState, Vec<Effect>) {
    let (mut next, mut effects) = step(state, event);
    // Only a reply that is being prepared or spoken can be held.
    if !matches!(next.phase, Phase::Thinking | Phase::Speaking) {
        next.held = false;
    }
    // However a hold ends, the speaker must not stay paused: the next reply would never be
    // heard. A cancel comes first, so what was cancelled is discarded before it could play.
    if state.held
        && !next.held
        && !effects
            .iter()
            .any(|effect| matches!(effect, Effect::ResumeReply { .. }))
    {
        let after_cancel = effects
            .iter()
            .position(|effect| matches!(effect, Effect::CancelTurn { .. }))
            .map_or(effects.len(), |cancel| cancel + 1);
        effects.insert(after_cancel, Effect::ResumeReply { turn: state.turn });
    }
    (next, effects)
}

fn step(state: ConversationState, event: Event) -> (ConversationState, Vec<Effect>) {
    use Phase::*;

    let stop = |state: ConversationState, reason| {
        let mut effects = Vec::new();
        if matches!(state.phase, Thinking | Speaking) {
            effects.push(Effect::CancelTurn { turn: state.turn });
        }
        effects.push(Effect::Stop { reason });
        (
            ConversationState {
                phase: Off,
                // Invalidate everything still in flight for the conversation that just ended.
                turn: state.turn + 1,
                ..state
            },
            effects,
        )
    };
    let stay = (state, Vec::new());
    let current_conversation =
        |conversation: u64| state.phase.is_active() && conversation == state.conversation;
    let assistant_has_floor = matches!(state.phase, Thinking | Speaking);
    let current_turn = |turn: u64| assistant_has_floor && turn == state.turn;

    match event {
        Event::Toggle => match state.phase {
            Off => {
                let conversation = state.conversation + 1;
                (
                    ConversationState {
                        phase: Starting,
                        conversation,
                        turn: state.turn,
                        held: false,
                    },
                    vec![Effect::Start { conversation }],
                )
            }
            // While the assistant is thinking or speaking the hotkey interrupts it and hands
            // the floor back to the user; otherwise it ends the conversation.
            Thinking | Speaking => (
                ConversationState {
                    phase: Listening,
                    turn: state.turn + 1,
                    ..state
                },
                vec![Effect::CancelTurn { turn: state.turn }],
            ),
            Starting | Listening | UserSpeaking => stop(state, StopReason::User),
        },
        Event::EndRequested => {
            if state.phase.is_active() {
                stop(state, StopReason::User)
            } else {
                stay
            }
        }
        Event::Ready { conversation } => {
            if state.phase == Starting && conversation == state.conversation {
                (
                    ConversationState {
                        phase: Listening,
                        ..state
                    },
                    Vec::new(),
                )
            } else {
                stay
            }
        }
        Event::StartFailed { conversation } => {
            if state.phase == Starting && conversation == state.conversation {
                stop(state, StopReason::StartFailed)
            } else {
                stay
            }
        }
        Event::SpeechStarted { conversation } => {
            if state.phase == Listening && conversation == state.conversation {
                (
                    ConversationState {
                        phase: UserSpeaking,
                        ..state
                    },
                    Vec::new(),
                )
            } else {
                stay
            }
        }
        Event::SpeechEnded { conversation } => {
            if matches!(state.phase, Listening | UserSpeaking) && conversation == state.conversation
            {
                let turn = state.turn + 1;
                (
                    ConversationState {
                        phase: Thinking,
                        turn,
                        ..state
                    },
                    vec![Effect::BeginTurn { turn }],
                )
            } else {
                stay
            }
        }
        Event::BargeIn { conversation } => {
            // Only a reply that is being prepared or spoken can be talked over. It is held,
            // not cancelled: the user may only be saying "mm-hmm".
            if assistant_has_floor && conversation == state.conversation && !state.held {
                (
                    ConversationState {
                        held: true,
                        ..state
                    },
                    vec![Effect::PauseReply { turn: state.turn }],
                )
            } else {
                stay
            }
        }
        Event::InterruptionConfirmed { conversation } => {
            // The user keeps the floor: what they are saying is the start of their next turn.
            if assistant_has_floor && conversation == state.conversation && state.held {
                (
                    ConversationState {
                        phase: UserSpeaking,
                        turn: state.turn + 1,
                        held: false,
                        ..state
                    },
                    vec![Effect::CancelTurn { turn: state.turn }],
                )
            } else {
                stay
            }
        }
        Event::FalseInterruption { conversation } => {
            if assistant_has_floor && conversation == state.conversation && state.held {
                (
                    ConversationState {
                        held: false,
                        ..state
                    },
                    vec![Effect::ResumeReply { turn: state.turn }],
                )
            } else {
                stay
            }
        }
        Event::TurnAbandoned { conversation } => {
            if state.phase == UserSpeaking && conversation == state.conversation {
                (
                    ConversationState {
                        phase: Listening,
                        ..state
                    },
                    Vec::new(),
                )
            } else {
                stay
            }
        }
        Event::TurnRouted { turn, outcome } => {
            if state.phase != Thinking || turn != state.turn {
                return stay;
            }
            match outcome {
                TurnOutcome::Reply => stay,
                TurnOutcome::NoReply => (
                    ConversationState {
                        phase: Listening,
                        ..state
                    },
                    Vec::new(),
                ),
                TurnOutcome::End => stop(state, StopReason::Goodbye),
            }
        }
        Event::ReplyAudioStarted { turn } => {
            if state.phase == Thinking && turn == state.turn {
                (
                    ConversationState {
                        phase: Speaking,
                        ..state
                    },
                    Vec::new(),
                )
            } else {
                stay
            }
        }
        Event::ReplyWaiting { turn } => {
            if state.phase == Speaking && turn == state.turn {
                (
                    ConversationState {
                        phase: Thinking,
                        ..state
                    },
                    Vec::new(),
                )
            } else {
                stay
            }
        }
        Event::ReplyFinished { turn } | Event::ReplyFailed { turn } => {
            if current_turn(turn) {
                (
                    ConversationState {
                        phase: Listening,
                        ..state
                    },
                    Vec::new(),
                )
            } else {
                stay
            }
        }
        Event::HandedToAgent { turn } => {
            if current_turn(turn) {
                stop(state, StopReason::AgentOpened)
            } else {
                stay
            }
        }
        Event::IdleTimeout { conversation } => {
            // Only silence while waiting counts as idle; a long reply is not idleness.
            if state.phase == Listening && conversation == state.conversation {
                stop(state, StopReason::Idle)
            } else {
                stay
            }
        }
        Event::MaxDuration { conversation } => {
            if current_conversation(conversation) {
                stop(state, StopReason::MaxDuration)
            } else {
                stay
            }
        }
        Event::DeviceLost { conversation } => {
            if current_conversation(conversation) {
                stop(state, StopReason::DeviceLost)
            } else {
                stay
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run events in order, returning the final state and every effect produced.
    fn run(events: &[Event]) -> (ConversationState, Vec<Effect>) {
        let mut state = ConversationState::default();
        let mut effects = Vec::new();
        for event in events {
            let (next, produced) = transition(state, *event);
            state = next;
            effects.extend(produced);
        }
        (state, effects)
    }

    /// A conversation that is on and waiting for the user.
    fn listening() -> Vec<Event> {
        vec![Event::Toggle, Event::Ready { conversation: 1 }]
    }

    /// A conversation in which the user has just finished speaking turn 1.
    fn thinking() -> Vec<Event> {
        let mut events = listening();
        events.push(Event::SpeechStarted { conversation: 1 });
        events.push(Event::SpeechEnded { conversation: 1 });
        events
    }

    fn after(mut events: Vec<Event>, more: &[Event]) -> (ConversationState, Vec<Effect>) {
        events.extend_from_slice(more);
        run(&events)
    }

    #[test]
    fn a_normal_turn_visits_every_phase_once() {
        let mut state = ConversationState::default();
        let mut phases = vec![state.phase];
        for event in [
            Event::Toggle,
            Event::Ready { conversation: 1 },
            Event::SpeechStarted { conversation: 1 },
            Event::SpeechEnded { conversation: 1 },
            Event::TurnRouted {
                turn: 1,
                outcome: TurnOutcome::Reply,
            },
            Event::ReplyAudioStarted { turn: 1 },
            Event::ReplyFinished { turn: 1 },
        ] {
            let (next, _) = transition(state, event);
            if next.phase != state.phase {
                phases.push(next.phase);
            }
            state = next;
        }

        assert_eq!(
            phases,
            vec![
                Phase::Off,
                Phase::Starting,
                Phase::Listening,
                Phase::UserSpeaking,
                Phase::Thinking,
                Phase::Speaking,
                Phase::Listening
            ]
        );
    }

    #[test]
    fn switching_on_opens_the_microphone_and_each_turn_is_started_once() {
        let (state, effects) = run(&thinking());

        assert_eq!(state.phase, Phase::Thinking);
        assert_eq!(
            effects,
            vec![
                Effect::Start { conversation: 1 },
                Effect::BeginTurn { turn: 1 }
            ]
        );
    }

    #[test]
    fn the_microphone_is_listened_to_while_the_user_has_the_floor_or_a_reply_is_prepared() {
        // Changed by design: thinking used to be deaf, which lost whatever the user said
        // after a pause that ended their turn too soon.
        for phase in [Phase::Listening, Phase::UserSpeaking, Phase::Thinking] {
            assert!(phase.listens(), "{phase:?}");
        }
        for phase in [Phase::Off, Phase::Starting, Phase::Speaking] {
            assert!(!phase.listens(), "{phase:?}");
        }
        // The floor is the user's only while waiting for or hearing them.
        assert!(Phase::Listening.user_has_floor());
        assert!(Phase::UserSpeaking.user_has_floor());
        for phase in [
            Phase::Off,
            Phase::Starting,
            Phase::Thinking,
            Phase::Speaking,
        ] {
            assert!(!phase.user_has_floor(), "{phase:?}");
        }
    }

    #[test]
    fn the_assistant_cannot_start_a_turn_by_hearing_itself() {
        // Speech detected while thinking or speaking changes nothing and starts no turn.
        for prefix in [thinking(), {
            let mut events = thinking();
            events.push(Event::ReplyAudioStarted { turn: 1 });
            events
        }] {
            let (before, _) = run(&prefix);
            let (state, effects) = after(
                prefix,
                &[
                    Event::SpeechStarted { conversation: 1 },
                    Event::SpeechEnded { conversation: 1 },
                ],
            );
            assert_eq!(state, before);
            assert_eq!(
                effects
                    .iter()
                    .filter(|e| matches!(e, Effect::BeginTurn { .. }))
                    .count(),
                1
            );
        }
    }

    #[test]
    fn the_hotkey_interrupts_a_reply_and_hands_the_floor_back() {
        let (state, effects) = after(
            thinking(),
            &[Event::ReplyAudioStarted { turn: 1 }, Event::Toggle],
        );

        assert_eq!(state.phase, Phase::Listening);
        assert_eq!(effects.last(), Some(&Effect::CancelTurn { turn: 1 }));
        // The conversation itself goes on: nothing was stopped.
        assert!(!effects.iter().any(|e| matches!(e, Effect::Stop { .. })));
    }

    #[test]
    fn a_reply_that_arrives_after_being_interrupted_is_ignored() {
        let (interrupted, _) = after(thinking(), &[Event::Toggle]);
        assert_eq!(interrupted.phase, Phase::Listening);

        // The cancelled turn's worker reports in late.
        for late in [
            Event::TurnRouted {
                turn: 1,
                outcome: TurnOutcome::Reply,
            },
            Event::ReplyAudioStarted { turn: 1 },
            Event::ReplyFinished { turn: 1 },
            Event::ReplyFailed { turn: 1 },
            Event::TurnRouted {
                turn: 1,
                outcome: TurnOutcome::End,
            },
        ] {
            let (state, effects) = transition(interrupted, late);
            assert_eq!(state, interrupted, "{late:?}");
            assert!(effects.is_empty(), "{late:?}");
        }
    }

    #[test]
    fn the_hotkey_ends_the_conversation_when_the_assistant_is_not_talking() {
        for prefix in [vec![Event::Toggle], listening(), {
            let mut events = listening();
            events.push(Event::SpeechStarted { conversation: 1 });
            events
        }] {
            let (state, effects) = after(prefix, &[Event::Toggle]);
            assert_eq!(state.phase, Phase::Off);
            assert_eq!(
                effects.last(),
                Some(&Effect::Stop {
                    reason: StopReason::User
                })
            );
        }
    }

    #[test]
    fn hammering_the_hotkey_never_leaves_two_conversations_or_an_open_microphone() {
        let mut state = ConversationState::default();
        let mut open = 0i32;
        for _ in 0..25 {
            let (next, effects) = transition(state, Event::Toggle);
            for effect in effects {
                match effect {
                    Effect::Start { .. } => open += 1,
                    Effect::Stop { .. } => open -= 1,
                    _ => {}
                }
            }
            assert!((0..=1).contains(&open));
            assert_eq!(open == 1, next.phase.is_active());
            state = next;
        }
    }

    #[test]
    fn every_way_of_ending_stops_exactly_once_and_cancels_a_turn_in_progress() {
        let endings = [
            (Event::EndRequested, StopReason::User),
            (
                Event::MaxDuration { conversation: 1 },
                StopReason::MaxDuration,
            ),
            (
                Event::DeviceLost { conversation: 1 },
                StopReason::DeviceLost,
            ),
        ];
        for (event, reason) in endings {
            let mut speaking = thinking();
            speaking.push(Event::ReplyAudioStarted { turn: 1 });
            let mut held = speaking.clone();
            held.push(Event::BargeIn { conversation: 1 });
            for prefix in [vec![Event::Toggle], listening(), thinking(), speaking, held] {
                let (before, earlier) = run(&prefix);
                let (state, effects) = after(prefix, &[event]);
                let new = &effects[earlier.len()..];

                assert_eq!(state.phase, Phase::Off, "{event:?} from {:?}", before.phase);
                assert_eq!(new.last(), Some(&Effect::Stop { reason }));
                assert_eq!(
                    new.iter()
                        .filter(|e| matches!(e, Effect::Stop { .. }))
                        .count(),
                    1
                );
                let cancels = new.contains(&Effect::CancelTurn { turn: before.turn });
                assert_eq!(
                    cancels,
                    matches!(before.phase, Phase::Thinking | Phase::Speaking)
                );
            }
        }
    }

    #[test]
    fn idle_ends_a_waiting_conversation_but_not_one_that_is_busy() {
        let (state, effects) = after(listening(), &[Event::IdleTimeout { conversation: 1 }]);
        assert_eq!(state.phase, Phase::Off);
        assert_eq!(
            effects.last(),
            Some(&Effect::Stop {
                reason: StopReason::Idle
            })
        );

        let (state, _) = after(thinking(), &[Event::IdleTimeout { conversation: 1 }]);
        assert_eq!(state.phase, Phase::Thinking);
    }

    #[test]
    fn goodbye_ends_and_an_empty_turn_goes_back_to_listening() {
        let (state, effects) = after(
            thinking(),
            &[Event::TurnRouted {
                turn: 1,
                outcome: TurnOutcome::End,
            }],
        );
        assert_eq!(state.phase, Phase::Off);
        assert_eq!(
            effects.last(),
            Some(&Effect::Stop {
                reason: StopReason::Goodbye
            })
        );

        let (state, effects) = after(
            thinking(),
            &[Event::TurnRouted {
                turn: 1,
                outcome: TurnOutcome::NoReply,
            }],
        );
        assert_eq!(state.phase, Phase::Listening);
        assert_eq!(effects.len(), 2, "no cancel or stop for an empty turn");
    }

    #[test]
    fn a_failed_reply_returns_to_listening_without_ending_the_conversation() {
        let (state, effects) = after(thinking(), &[Event::ReplyFailed { turn: 1 }]);

        assert_eq!(state.phase, Phase::Listening);
        assert!(!effects.iter().any(|e| matches!(e, Effect::Stop { .. })));
    }

    #[test]
    fn a_noise_that_was_not_speech_goes_back_to_listening_without_a_turn() {
        let mut events = listening();
        events.push(Event::SpeechStarted { conversation: 1 });
        let (state, effects) = after(events, &[Event::TurnAbandoned { conversation: 1 }]);

        assert_eq!(state.phase, Phase::Listening);
        assert_eq!(effects, vec![Effect::Start { conversation: 1 }]);
        // It means nothing in any other phase.
        let (thinking_state, _) = run(&thinking());
        let (state, effects) = transition(thinking_state, Event::TurnAbandoned { conversation: 1 });
        assert_eq!(state, thinking_state);
        assert!(effects.is_empty());
    }

    /// A conversation in which turn 1's reply is being spoken.
    fn speaking() -> Vec<Event> {
        let mut events = thinking();
        events.push(Event::ReplyAudioStarted { turn: 1 });
        events
    }

    /// Turn 1's reply is being spoken and the user has started talking over it.
    fn held_while_speaking() -> Vec<Event> {
        let mut events = speaking();
        events.push(Event::BargeIn { conversation: 1 });
        events
    }

    /// The user carried on speaking while turn 1's reply was being prepared.
    fn held_while_thinking() -> Vec<Event> {
        let mut events = thinking();
        events.push(Event::BargeIn { conversation: 1 });
        events
    }

    #[test]
    fn speaking_over_a_reply_holds_it_instead_of_cutting_it_off() {
        // Changed by design: this used to cancel the reply on the spot.
        let (before, _) = run(&speaking());
        let (state, effects) = transition(before, Event::BargeIn { conversation: 1 });

        assert_eq!(state.phase, Phase::Speaking);
        assert!(state.held);
        assert_eq!(state.turn, 1, "the turn goes on");
        assert_eq!(effects, vec![Effect::PauseReply { turn: 1 }]);
    }

    #[test]
    fn a_confirmed_interruption_cuts_the_reply_off_and_starts_the_users_turn() {
        let (state, effects) = after(
            held_while_speaking(),
            &[Event::InterruptionConfirmed { conversation: 1 }],
        );

        assert_eq!(state.phase, Phase::UserSpeaking);
        assert!(!state.held);
        // Cancelled (and the queue discarded) before the speaker is let go again.
        assert_eq!(
            effects[effects.len() - 2..],
            [
                Effect::CancelTurn { turn: 1 },
                Effect::ResumeReply { turn: 1 }
            ]
        );
        assert!(!effects.iter().any(|e| matches!(e, Effect::Stop { .. })));

        // The interrupted reply reporting in late changes nothing, and the user's words
        // become the next turn.
        let (late, none) = transition(state, Event::ReplyFinished { turn: 1 });
        assert_eq!(late, state);
        assert!(none.is_empty());
        let (next, effects) = transition(state, Event::SpeechEnded { conversation: 1 });
        assert_eq!(next.phase, Phase::Thinking);
        assert_eq!(effects, vec![Effect::BeginTurn { turn: 3 }]);
    }

    #[test]
    fn a_false_interruption_lets_the_reply_carry_on_in_the_same_turn() {
        let (state, effects) = after(
            held_while_speaking(),
            &[Event::FalseInterruption { conversation: 1 }],
        );

        assert_eq!(state.phase, Phase::Speaking);
        assert!(!state.held);
        assert_eq!(effects.last(), Some(&Effect::ResumeReply { turn: 1 }));
        assert!(!effects
            .iter()
            .any(|e| matches!(e, Effect::CancelTurn { .. })));
        // The same reply can still finish, and be talked over again.
        let (again, effects) = transition(state, Event::BargeIn { conversation: 1 });
        assert!(again.held);
        assert_eq!(effects, vec![Effect::PauseReply { turn: 1 }]);
        let (done, _) = transition(state, Event::ReplyFinished { turn: 1 });
        assert_eq!(done.phase, Phase::Listening);
    }

    #[test]
    fn speech_while_the_reply_is_prepared_holds_it_before_any_of_it_plays() {
        let (state, effects) = run(&held_while_thinking());
        assert_eq!((state.phase, state.held), (Phase::Thinking, true));
        assert_eq!(effects.last(), Some(&Effect::PauseReply { turn: 1 }));

        // Its first audio arrives into the paused speaker: still held.
        let (state, effects) = transition(state, Event::ReplyAudioStarted { turn: 1 });
        assert_eq!((state.phase, state.held), (Phase::Speaking, true));
        assert!(effects.is_empty());

        // The user was carrying on with their question: it is cancelled for the longer one.
        let (state, effects) = transition(state, Event::InterruptionConfirmed { conversation: 1 });
        assert_eq!(state.phase, Phase::UserSpeaking);
        assert_eq!(
            effects,
            vec![
                Effect::CancelTurn { turn: 1 },
                Effect::ResumeReply { turn: 1 }
            ]
        );
    }

    #[test]
    fn a_noise_while_the_reply_is_prepared_releases_it() {
        let (state, effects) = after(
            held_while_thinking(),
            &[Event::FalseInterruption { conversation: 1 }],
        );
        assert_eq!((state.phase, state.held), (Phase::Thinking, false));
        assert_eq!(effects.last(), Some(&Effect::ResumeReply { turn: 1 }));
        // The reply goes on as if nothing had happened.
        let (state, _) = transition(state, Event::ReplyAudioStarted { turn: 1 });
        assert_eq!(state.phase, Phase::Speaking);
    }

    #[test]
    fn only_a_reply_being_prepared_or_spoken_can_be_talked_over() {
        // Changed by design: a reply being prepared can now be held too.
        for prefix in [vec![Event::Toggle], listening(), {
            let mut events = listening();
            events.push(Event::SpeechStarted { conversation: 1 });
            events
        }] {
            let (before, _) = run(&prefix);
            for event in [
                Event::BargeIn { conversation: 1 },
                Event::InterruptionConfirmed { conversation: 1 },
                Event::FalseInterruption { conversation: 1 },
            ] {
                let (state, effects) = transition(before, event);
                assert_eq!(state, before, "{event:?} in {:?}", before.phase);
                assert!(effects.is_empty());
            }
        }
        // And not from an earlier conversation.
        for prefix in [thinking(), speaking(), held_while_speaking()] {
            let (before, _) = run(&prefix);
            for event in [
                Event::BargeIn { conversation: 7 },
                Event::InterruptionConfirmed { conversation: 7 },
                Event::FalseInterruption { conversation: 7 },
            ] {
                let (state, effects) = transition(before, event);
                assert_eq!(state, before, "{event:?}");
                assert!(effects.is_empty());
            }
        }
    }

    #[test]
    fn a_verdict_needs_a_hold_and_a_hold_is_not_taken_twice() {
        for prefix in [thinking(), speaking()] {
            let (before, _) = run(&prefix);
            for event in [
                Event::InterruptionConfirmed { conversation: 1 },
                Event::FalseInterruption { conversation: 1 },
            ] {
                let (state, effects) = transition(before, event);
                assert_eq!(state, before, "{event:?}");
                assert!(effects.is_empty());
            }
        }
        let (held, _) = run(&held_while_speaking());
        let (state, effects) = transition(held, Event::BargeIn { conversation: 1 });
        assert_eq!(state, held);
        assert!(effects.is_empty(), "the speaker is paused once");
    }

    #[test]
    fn the_hotkey_stops_a_held_reply_at_once() {
        for prefix in [held_while_speaking(), held_while_thinking()] {
            let (state, effects) = after(prefix, &[Event::Toggle]);
            assert_eq!(state.phase, Phase::Listening);
            assert!(!state.held);
            assert_eq!(
                effects[effects.len() - 2..],
                [
                    Effect::CancelTurn { turn: 1 },
                    Effect::ResumeReply { turn: 1 }
                ]
            );
        }
    }

    #[test]
    fn a_reply_that_ends_while_held_releases_the_speaker() {
        for (prefix, ending) in [
            (held_while_thinking(), Event::ReplyFailed { turn: 1 }),
            (held_while_thinking(), Event::ReplyFinished { turn: 1 }),
            (
                held_while_thinking(),
                Event::TurnRouted {
                    turn: 1,
                    outcome: TurnOutcome::NoReply,
                },
            ),
            (held_while_speaking(), Event::ReplyFinished { turn: 1 }),
        ] {
            let (state, effects) = after(prefix, &[ending]);
            assert_eq!(state.phase, Phase::Listening, "{ending:?}");
            assert!(!state.held);
            assert_eq!(effects.last(), Some(&Effect::ResumeReply { turn: 1 }));
            // A verdict that arrives afterwards is too late to matter.
            for late in [
                Event::InterruptionConfirmed { conversation: 1 },
                Event::FalseInterruption { conversation: 1 },
            ] {
                let (unchanged, none) = transition(state, late);
                assert_eq!(unchanged, state);
                assert!(none.is_empty());
            }
        }
    }

    #[test]
    fn a_turn_whose_start_was_missed_still_begins_when_it_ends() {
        // The user started talking as the reply ended, so the start was heard while the
        // assistant still had the floor; then the floor came back.
        let (state, effects) = after(listening(), &[Event::SpeechEnded { conversation: 1 }]);
        assert_eq!(state.phase, Phase::Thinking);
        assert_eq!(effects.last(), Some(&Effect::BeginTurn { turn: 1 }));
    }

    #[test]
    fn a_reply_waiting_for_an_agent_listens_without_giving_up_the_floor() {
        let (before, _) = run(&speaking());
        let (state, effects) = transition(before, Event::ReplyWaiting { turn: 1 });
        assert_eq!(state.phase, Phase::Thinking);
        assert_eq!(state.turn, 1, "the same reply");
        assert!(effects.is_empty());
        // The rest of the reply arrives and is spoken.
        let (state, _) = transition(state, Event::ReplyAudioStarted { turn: 1 });
        assert_eq!(state.phase, Phase::Speaking);
        // Late or out of place, it changes nothing.
        for prefix in [listening(), thinking()] {
            let (before, _) = run(&prefix);
            assert_eq!(
                transition(before, Event::ReplyWaiting { turn: 1 }).0,
                before
            );
        }
        let (interrupted, _) = after(speaking(), &[Event::Toggle]);
        assert_eq!(
            transition(interrupted, Event::ReplyWaiting { turn: 1 }).0,
            interrupted
        );
    }

    #[test]
    fn opening_an_agent_ends_the_conversation_once_its_reply_is_spoken() {
        let (state, effects) = after(speaking(), &[Event::HandedToAgent { turn: 1 }]);
        assert_eq!(state.phase, Phase::Off);
        assert_eq!(
            effects.last(),
            Some(&Effect::Stop {
                reason: StopReason::AgentOpened
            })
        );
        // Not from a reply that was already interrupted.
        let (interrupted, _) = after(speaking(), &[Event::Toggle]);
        let (state, effects) = transition(interrupted, Event::HandedToAgent { turn: 1 });
        assert_eq!(state, interrupted);
        assert!(effects.is_empty());
    }

    /// A small deterministic generator, so the walk below is the same on every run.
    fn shuffled(seed: &mut u64, len: usize) -> usize {
        *seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((*seed >> 33) as usize) % len
    }

    #[test]
    fn whatever_happens_the_speaker_is_paused_exactly_while_a_reply_is_held() {
        let mut seed = 7u64;
        for _ in 0..200 {
            let mut state = ConversationState::default();
            let mut paused = false;
            let mut open = false;
            for _ in 0..60 {
                let conversation = state.conversation.max(1);
                let turn = state.turn;
                let events = [
                    Event::Toggle,
                    Event::EndRequested,
                    Event::Ready { conversation },
                    Event::SpeechStarted { conversation },
                    Event::SpeechEnded { conversation },
                    Event::BargeIn { conversation },
                    Event::InterruptionConfirmed { conversation },
                    Event::FalseInterruption { conversation },
                    Event::TurnAbandoned { conversation },
                    Event::TurnRouted {
                        turn,
                        outcome: TurnOutcome::Reply,
                    },
                    Event::TurnRouted {
                        turn,
                        outcome: TurnOutcome::NoReply,
                    },
                    Event::ReplyAudioStarted { turn },
                    Event::ReplyWaiting { turn },
                    Event::ReplyFinished { turn },
                    Event::ReplyFailed { turn },
                    Event::HandedToAgent { turn },
                    Event::ReplyFinished {
                        turn: turn.saturating_sub(1),
                    },
                    Event::IdleTimeout { conversation },
                ];
                let event = events[shuffled(&mut seed, events.len())];
                let (next, effects) = transition(state, event);
                for effect in &effects {
                    match effect {
                        Effect::Start { .. } => open = true,
                        Effect::PauseReply { .. } => {
                            assert!(!paused, "paused twice by {event:?}");
                            paused = true;
                        }
                        Effect::ResumeReply { .. } => paused = false,
                        Effect::Stop { .. } => {
                            open = false;
                            paused = false;
                        }
                        _ => {}
                    }
                }
                assert_eq!(paused, next.held, "after {event:?} from {state:?}");
                assert!(
                    !next.held || matches!(next.phase, Phase::Thinking | Phase::Speaking),
                    "held in {:?}",
                    next.phase
                );
                assert_eq!(open, next.phase.is_active());
                state = next;
            }
        }
    }

    #[test]
    fn a_failed_start_closes_everything() {
        let (state, effects) = run(&[Event::Toggle, Event::StartFailed { conversation: 1 }]);

        assert_eq!(state.phase, Phase::Off);
        assert_eq!(
            effects.last(),
            Some(&Effect::Stop {
                reason: StopReason::StartFailed
            })
        );
    }

    #[test]
    fn events_from_an_earlier_conversation_cannot_touch_a_new_one() {
        // Conversation 1 ends while a turn is in flight; conversation 2 starts.
        let mut events = thinking();
        events.extend([
            Event::EndRequested,
            Event::Toggle,
            Event::Ready { conversation: 2 },
        ]);
        let (second, _) = run(&events);
        assert_eq!((second.phase, second.conversation), (Phase::Listening, 2));

        for stale in [
            Event::Ready { conversation: 1 },
            Event::SpeechStarted { conversation: 1 },
            Event::SpeechEnded { conversation: 1 },
            Event::TurnAbandoned { conversation: 1 },
            Event::IdleTimeout { conversation: 1 },
            Event::MaxDuration { conversation: 1 },
            Event::DeviceLost { conversation: 1 },
            Event::StartFailed { conversation: 1 },
            Event::ReplyAudioStarted { turn: 1 },
            Event::ReplyFinished { turn: 1 },
            Event::TurnRouted {
                turn: 1,
                outcome: TurnOutcome::End,
            },
            Event::BargeIn { conversation: 1 },
            Event::InterruptionConfirmed { conversation: 1 },
            Event::FalseInterruption { conversation: 1 },
        ] {
            let (state, effects) = transition(second, stale);
            assert_eq!(state, second, "{stale:?}");
            assert!(effects.is_empty(), "{stale:?}");
        }
        // Nor while the new conversation is busy with a held reply of its own.
        let mut busy = events.clone();
        busy.extend([
            Event::SpeechStarted { conversation: 2 },
            Event::SpeechEnded { conversation: 2 },
            Event::BargeIn { conversation: 2 },
        ]);
        let (held, _) = run(&busy);
        assert!(held.held);
        for stale in [
            Event::InterruptionConfirmed { conversation: 1 },
            Event::FalseInterruption { conversation: 1 },
            Event::ReplyFinished { turn: 1 },
        ] {
            let (state, effects) = transition(held, stale);
            assert_eq!(state, held, "{stale:?}");
            assert!(effects.is_empty(), "{stale:?}");
        }
    }

    #[test]
    fn turn_numbers_never_repeat_across_interruptions_and_conversations() {
        let mut events = thinking(); // turn 1
        events.push(Event::Toggle); // interrupt
        events.push(Event::SpeechStarted { conversation: 1 });
        events.push(Event::SpeechEnded { conversation: 1 }); // a new turn
        events.push(Event::BargeIn { conversation: 1 }); // the user carries on...
        events.push(Event::InterruptionConfirmed { conversation: 1 }); // ...for real
        events.push(Event::SpeechEnded { conversation: 1 }); // one longer turn
        events.push(Event::EndRequested);
        events.extend([Event::Toggle, Event::Ready { conversation: 2 }]);
        events.push(Event::SpeechStarted { conversation: 2 });
        events.push(Event::SpeechEnded { conversation: 2 });

        let (_, effects) = run(&events);
        let turns: Vec<u64> = effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::BeginTurn { turn } => Some(*turn),
                _ => None,
            })
            .collect();

        assert_eq!(turns.len(), 4);
        assert!(turns.windows(2).all(|pair| pair[1] > pair[0]), "{turns:?}");
    }
}
