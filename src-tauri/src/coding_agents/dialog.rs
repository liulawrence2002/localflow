//! The clarify-and-confirm exchange around a coding-agent command, as an explicit state
//! machine.
//!
//! A pure function from (state, event) to (state, effects), like the conversation's own
//! turn-taking machine: it owns no devices, threads or clocks. The conversation feeds it each
//! user turn (only the user's own transcript, see [`UserSpeech`]) and a timeout, speaks what
//! it says, and on [`DialogEffect::Execute`] carries the command out.

use std::time::Duration;

use super::{
    grammar::{recognize, resolve_project_reply, CommandContext, Recognition, UserSpeech},
    phrases::words,
    speech, AgentCommand, CommandDraft, ProjectRef,
};

/// How long a read-back or a "which project?" waits for an answer. Silence declines.
pub const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum DialogState {
    /// No command in progress: every turn is checked for a new command.
    #[default]
    Idle,
    /// A command was heard but its project is unclear, and the user was asked which.
    /// `candidates` is empty when any project may be named.
    ChoosingProject {
        draft: CommandDraft,
        candidates: Vec<ProjectRef>,
    },
    /// The command was read back; only "yes" carries it out.
    Confirming { command: AgentCommand },
}

impl DialogState {
    /// Whether an answer is awaited, so the conversation should run the
    /// [`CONFIRMATION_TIMEOUT`] and send every turn here first.
    pub fn is_waiting(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DialogEvent<'a> {
    /// A finished user turn.
    UserSaid(UserSpeech<'a>),
    /// Nothing was said for [`CONFIRMATION_TIMEOUT`] while waiting.
    TimedOut,
    /// The conversation ended or was interrupted: drop anything pending, silently.
    Reset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialogEffect {
    /// Not about a coding-agent command: route the turn as usual.
    PassOn,
    /// Speak this and keep listening. Nothing else happens.
    Say(String),
    /// The user said yes. Speak [`speech::starting`] if it has a sentence, carry the command
    /// out with `CodingAgents::execute`, then speak [`speech::outcome`].
    Execute(AgentCommand),
}

/// How a reply to the read-back was meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Yes,
    No,
    Other,
}

/// Only these, said on their own (politeness aside), count as yes.
const YES: &[&str] = &[
    "yes",
    "yeah",
    "yep",
    "yup",
    "sure",
    "go ahead",
    "do it",
    "go for it",
    "yes go ahead",
    "yes do it",
    "yes please",
    "yeah go ahead",
    "yes thats right",
    "thats right",
];
const NO: &[&str] = &[
    "no",
    "nope",
    "nah",
    "no thanks",
    "no thank you",
    "cancel",
    "cancel that",
    "stop",
    "stop that",
    "never mind",
    "nevermind",
    "forget it",
    "dont",
    "do not",
    "dont do it",
];
/// Dropped from both ends before a reply is classified: "yes, please", "OK, go ahead".
const REPLY_FILLERS: &[&str] = &["please", "ok", "okay", "thanks", "um", "uh", "oh", "well"];

pub fn classify_reply(reply: UserSpeech<'_>) -> Reply {
    let heard: Vec<String> = words(reply.text())
        .into_iter()
        .map(|word| word.text)
        .collect();
    let mut start = 0;
    let mut end = heard.len();
    while start < end && REPLY_FILLERS.contains(&heard[start].as_str()) {
        start += 1;
    }
    while end > start && REPLY_FILLERS.contains(&heard[end - 1].as_str()) {
        end -= 1;
    }
    let core = heard[start..end].join(" ");
    if YES.contains(&core.as_str()) {
        Reply::Yes
    } else if NO.contains(&core.as_str()) {
        Reply::No
    } else {
        Reply::Other
    }
}

pub fn step(
    state: DialogState,
    event: DialogEvent<'_>,
    context: &CommandContext,
) -> (DialogState, Vec<DialogEffect>) {
    use DialogEffect::*;
    use DialogState::*;

    match (state, event) {
        (_, DialogEvent::Reset) => (Idle, Vec::new()),
        (Idle, DialogEvent::TimedOut) => (Idle, Vec::new()),
        (_, DialogEvent::TimedOut) => (Idle, vec![Say(speech::TIMED_OUT.to_string())]),

        (Confirming { command }, DialogEvent::UserSaid(reply)) => match classify_reply(reply) {
            Reply::Yes => (Idle, vec![Execute(command)]),
            Reply::No | Reply::Other => (Idle, vec![Say(speech::DECLINED.to_string())]),
        },

        (ChoosingProject { draft, candidates }, DialogEvent::UserSaid(reply)) => {
            if classify_reply(reply) == Reply::No {
                return (Idle, vec![Say(speech::DROPPED.to_string())]);
            }
            match resolve_project_reply(reply, &candidates, &context.projects) {
                Some(project) => {
                    let command = draft.complete(project);
                    let readback = speech::readback(&command);
                    (Confirming { command }, vec![Say(readback)])
                }
                None => (Idle, vec![Say(speech::STILL_UNCLEAR.to_string())]),
            }
        }

        (Idle, DialogEvent::UserSaid(speech_heard)) => {
            let recognition = recognize(speech_heard, context);
            let all_projects = context.projects.all_refs();
            let say = |recognition: &Recognition| {
                Say(speech::clarification(recognition, &all_projects).unwrap_or_default())
            };
            match recognition {
                Recognition::NotACommand => (Idle, vec![PassOn]),
                Recognition::Command(command) => {
                    let readback = speech::readback(&command);
                    (Confirming { command }, vec![Say(readback)])
                }
                Recognition::UnknownProject { ref draft, .. }
                | Recognition::MissingProject { ref draft } => {
                    let effects = vec![say(&recognition)];
                    (
                        ChoosingProject {
                            draft: draft.clone(),
                            candidates: Vec::new(),
                        },
                        effects,
                    )
                }
                Recognition::AmbiguousProject {
                    ref draft,
                    ref candidates,
                    ..
                } => {
                    let effects = vec![say(&recognition)];
                    (
                        ChoosingProject {
                            draft: draft.clone(),
                            candidates: candidates.clone(),
                        },
                        effects,
                    )
                }
                Recognition::NoProjects { .. }
                | Recognition::MissingQuestion { .. }
                | Recognition::AgentNotAvailable { .. } => (Idle, vec![say(&recognition)]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding_agents::{
        projects::{CodingProject, ProjectIndex},
        AgentAvailability, AgentKind,
    };

    fn context() -> CommandContext {
        let project = |id: &str, path: &str, names: &[&str]| CodingProject {
            id: id.into(),
            path: path.into(),
            spoken_names: names.iter().map(|n| n.to_string()).collect(),
        };
        CommandContext {
            projects: ProjectIndex::new(&[
                project("lf", r"C:\work\localflow", &["localflow"]),
                project("a1", r"C:\work\app", &["my app"]),
                project("a2", r"C:\personal\app", &["my app"]),
            ]),
            availability: AgentAvailability::all(),
        }
    }

    fn said(text: &str) -> DialogEvent<'_> {
        DialogEvent::UserSaid(UserSpeech::from_user_transcript(text))
    }

    fn run(events: &[DialogEvent<'_>]) -> (DialogState, Vec<Vec<DialogEffect>>) {
        let context = context();
        let mut state = DialogState::Idle;
        let mut all = Vec::new();
        for event in events {
            let (next, effects) = step(state, *event, &context);
            state = next;
            all.push(effects);
        }
        (state, all)
    }

    fn executed(effects: &[DialogEffect]) -> Option<&AgentCommand> {
        effects.iter().find_map(|effect| match effect {
            DialogEffect::Execute(command) => Some(command),
            _ => None,
        })
    }

    #[test]
    fn an_ordinary_turn_passes_through() {
        let (state, effects) = run(&[said("What's the capital of France?")]);
        assert_eq!(state, DialogState::Idle);
        assert_eq!(effects, [vec![DialogEffect::PassOn]]);
    }

    #[test]
    fn a_command_is_read_back_and_runs_only_on_yes() {
        let (state, effects) = run(&[said("Open Claude Code in localflow")]);
        assert!(matches!(state, DialogState::Confirming { .. }));
        assert!(state.is_waiting());
        assert!(
            matches!(&effects[0][..], [DialogEffect::Say(text)] if text.starts_with("Open Claude Code in localflow?"))
        );

        let (state, effects) = run(&[said("Open Claude Code in localflow"), said("Yes, please.")]);
        assert_eq!(state, DialogState::Idle);
        let command = executed(&effects[1]).expect("yes runs it");
        assert_eq!(command.agent(), AgentKind::ClaudeCode);
        assert_eq!(command.project().id, "lf");
    }

    #[test]
    fn anything_but_yes_declines() {
        for reply in [
            "no",
            "cancel",
            "stop that",
            "yes but in the other project",
            "maybe",
            "OK",
            "open it in cursor instead",
            "",
        ] {
            let (state, effects) = run(&[said("open claude code in localflow"), said(reply)]);
            assert_eq!(state, DialogState::Idle, "{reply}");
            assert_eq!(
                effects[1],
                [DialogEffect::Say(speech::DECLINED.to_string())],
                "{reply}"
            );
        }
    }

    #[test]
    fn silence_declines() {
        let (state, effects) = run(&[said("open claude code in localflow"), DialogEvent::TimedOut]);
        assert_eq!(state, DialogState::Idle);
        assert_eq!(
            effects[1],
            [DialogEffect::Say(speech::TIMED_OUT.to_string())]
        );
        // A timeout with nothing pending does nothing.
        let (_, effects) = run(&[DialogEvent::TimedOut]);
        assert!(effects[0].is_empty());
    }

    #[test]
    fn reset_drops_a_pending_command_silently() {
        let (state, effects) = run(&[said("open claude code in localflow"), DialogEvent::Reset]);
        assert_eq!(state, DialogState::Idle);
        assert!(effects[1].is_empty());
    }

    #[test]
    fn an_unclear_project_is_asked_then_confirmed() {
        let (state, effects) = run(&[said("Open Claude Code in my app")]);
        match &state {
            DialogState::ChoosingProject { candidates, .. } => assert_eq!(candidates.len(), 2),
            other => panic!("{other:?}"),
        }
        assert!(
            matches!(&effects[0][..], [DialogEffect::Say(text)] if text.contains("Which one?"))
        );

        let (state, effects) = run(&[
            said("Open Claude Code in my app"),
            said("the one in personal"),
            said("yes"),
        ]);
        assert_eq!(state, DialogState::Idle);
        assert!(
            matches!(&effects[1][..], [DialogEffect::Say(text)] if text.starts_with("Open Claude Code in my app?"))
        );
        assert_eq!(executed(&effects[2]).unwrap().project().id, "a2");
    }

    #[test]
    fn a_missing_project_can_be_given_afterwards() {
        let (_, effects) = run(&[
            said("Ask Claude why the build is slow"),
            said("localflow"),
            said("go ahead"),
        ]);
        match executed(&effects[2]) {
            Some(AgentCommand::Ask {
                project, question, ..
            }) => {
                assert_eq!(project.id, "lf");
                assert_eq!(question, "why the build is slow");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_answer_naming_no_project_drops_the_command() {
        let (state, effects) = run(&[said("Open Claude Code in foobar"), said("the blue one")]);
        assert_eq!(state, DialogState::Idle);
        assert_eq!(
            effects[1],
            [DialogEffect::Say(speech::STILL_UNCLEAR.to_string())]
        );

        let (state, effects) = run(&[said("Open Claude Code"), said("never mind")]);
        assert_eq!(state, DialogState::Idle);
        assert_eq!(effects[1], [DialogEffect::Say(speech::DROPPED.to_string())]);
    }

    #[test]
    fn agents_not_set_up_are_explained_and_nothing_waits() {
        let (state, effects) = run(&[said("Open Codex in localflow")]);
        assert_eq!(state, DialogState::Idle);
        assert_eq!(
            effects[0],
            [DialogEffect::Say(
                "Codex isn't set up for voice commands yet.".to_string()
            )]
        );
    }

    #[test]
    fn a_yes_with_nothing_pending_is_just_a_turn() {
        let (state, effects) = run(&[said("yes")]);
        assert_eq!(state, DialogState::Idle);
        assert_eq!(effects, [vec![DialogEffect::PassOn]]);
    }

    #[test]
    fn replies_are_classified_strictly() {
        let reply = |text| classify_reply(UserSpeech::from_user_transcript(text));
        assert_eq!(reply("Yes."), Reply::Yes);
        assert_eq!(reply("OK, go ahead"), Reply::Yes);
        assert_eq!(reply("do it please"), Reply::Yes);
        assert_eq!(reply("No thanks"), Reply::No);
        assert_eq!(reply("Never mind."), Reply::No);
        assert_eq!(reply("yes and also delete everything"), Reply::Other);
        assert_eq!(reply("I guess"), Reply::Other);
        assert_eq!(reply("ok"), Reply::Other);
    }
}
