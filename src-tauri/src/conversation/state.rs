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
    /// Whether microphone audio is acted on. While the assistant thinks or speaks it is
    /// ignored, so the assistant can never hear itself and take that for the user.
    pub fn listens(self) -> bool {
        matches!(self, Self::Listening | Self::UserSpeaking)
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
    SpeechEnded {
        conversation: u64,
    },
    TurnRouted {
        turn: u64,
        outcome: TurnOutcome,
    },
    /// The first audio of the reply reached the speaker queue.
    ReplyAudioStarted {
        turn: u64,
    },
    /// The reply has been spoken to the end.
    ReplyFinished {
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
    /// Cancel the model and the voice for this turn and silence the speaker.
    CancelTurn { turn: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationState {
    pub phase: Phase,
    pub conversation: u64,
    pub turn: u64,
}

impl Default for ConversationState {
    fn default() -> Self {
        Self {
            phase: Phase::Off,
            conversation: 0,
            turn: 0,
        }
    }
}

pub fn transition(state: ConversationState, event: Event) -> (ConversationState, Vec<Effect>) {
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
    let current_turn = |turn: u64| matches!(state.phase, Thinking | Speaking) && turn == state.turn;

    match event {
        Event::Toggle => match state.phase {
            Off => {
                let conversation = state.conversation + 1;
                (
                    ConversationState {
                        phase: Starting,
                        conversation,
                        turn: state.turn,
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
            if state.phase == UserSpeaking && conversation == state.conversation {
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
    fn the_microphone_is_only_acted_on_while_waiting_for_or_hearing_the_user() {
        assert!(Phase::Listening.listens());
        assert!(Phase::UserSpeaking.listens());
        for phase in [
            Phase::Off,
            Phase::Starting,
            Phase::Thinking,
            Phase::Speaking,
        ] {
            assert!(!phase.listens(), "{phase:?}");
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
            for prefix in [vec![Event::Toggle], listening(), thinking(), speaking] {
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
        ] {
            let (state, effects) = transition(second, stale);
            assert_eq!(state, second, "{stale:?}");
            assert!(effects.is_empty(), "{stale:?}");
        }
    }

    #[test]
    fn turn_numbers_never_repeat_across_interruptions_and_conversations() {
        let mut events = thinking(); // turn 1
        events.push(Event::Toggle); // interrupt
        events.push(Event::SpeechStarted { conversation: 1 });
        events.push(Event::SpeechEnded { conversation: 1 }); // a new turn
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

        assert_eq!(turns.len(), 3);
        assert!(turns.windows(2).all(|pair| pair[1] > pair[0]), "{turns:?}");
    }
}
