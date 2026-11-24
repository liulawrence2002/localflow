//! Voice commands for coding agents, inside a conversation (milestone 5).
//!
//! The command rules, the clarify-and-confirm dialog and every sentence live in
//! [`crate::coding_agents`]. This module is the conversation's side of it:
//!
//! - [`AgentRuntime`] is what a host supplies: the projects, what is installed, and a way to
//!   carry a confirmed command out. Tests use a fake; hosts without agents use [`NoAgents`].
//! - [`AgentDialog`] wraps the pure dialog with the one thing it cannot know: whether the
//!   question it asked was heard. An answer counts only when the read-back (or "which
//!   project?") was spoken to its end, and the answer was begun by the user afterwards, in
//!   their own turn, not over the assistant's voice. The read-backs also contain no stretch
//!   of words that counts as a yes (tested in `coding_agents::speech`), so the assistant's
//!   own voice, heard back, cannot confirm anything either way.
//! - [`AgentJob`] is a background question in progress, which the ear can stop or ask to be
//!   reminded about while the user talks.
//!
//! Only the user's own microphone transcript is ever given to the dialog. Page text, model
//! replies and the assistant's own words go to the voice and nowhere else.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use crate::{
    cancel::CancelFlag,
    coding_agents::{
        agent::{AskOutcome, LaunchFailure},
        dialog::{self, DialogEffect, DialogEvent, DialogState},
        projects::ProjectIndex,
        AgentAvailability, AgentCommand, CommandContext, ExecutionOutcome, Unavailability,
        UserSpeech,
    },
};

/// The coding agents, as a conversation uses them.
pub trait AgentRuntime: Send + Sync {
    /// Look for the agents. Called once as a conversation starts, on a worker thread; may run
    /// the agents' `--version` and `--help`.
    fn prepare(&self) {}
    /// The user's projects and what can run, for recognising a command.
    fn context(&self) -> CommandContext;
    /// Carry out a confirmed command. Blocks: an open returns at once, a question waits for
    /// its answer or for `cancel`.
    fn execute(&self, command: &AgentCommand, cancel: &CancelFlag) -> ExecutionOutcome;
}

/// For hosts with no coding agents: no projects, nothing installed.
pub struct NoAgents;

impl AgentRuntime for NoAgents {
    fn context(&self) -> CommandContext {
        CommandContext {
            projects: ProjectIndex::new(&[]),
            availability: AgentAvailability::none(),
        }
    }

    fn execute(&self, command: &AgentCommand, _cancel: &CancelFlag) -> ExecutionOutcome {
        let failure = LaunchFailure::Unavailable(Unavailability::NotInstalled);
        match command {
            AgentCommand::Open { .. } => ExecutionOutcome::OpenFailed(failure),
            AgentCommand::Ask { .. } => {
                ExecutionOutcome::Answered(AskOutcome::CouldNotStart(failure))
            }
        }
    }
}

/// What to do with a user turn, as far as coding agents are concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnPlan {
    /// Not about a command: answer it as usual.
    PassOn,
    /// Say this, and nothing else. Not part of what the model is told.
    Say(String),
    /// The user confirmed: carry this out.
    Execute(AgentCommand),
    /// An answer that cannot count (see [`AgentDialog`]): do nothing, keep waiting.
    Ignore,
}

/// How a user turn was heard, which decides whether it may answer the dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HeardTurn {
    /// How long after the floor was given back to the user their speech began. `None` when
    /// it began while the assistant had the floor: over a reply, or as it was prepared.
    pub began_after_floor: Option<Duration>,
}

/// The conversation's coding-agent exchange: the dialog, and whether its question was heard.
#[derive(Debug, Default)]
pub struct AgentDialog {
    state: DialogState,
    /// The turn whose reply asked what the dialog waits on.
    asked_in: Option<u64>,
    /// That reply was spoken to its end, so it can be answered now.
    heard_to_the_end: bool,
}

impl AgentDialog {
    /// Whether the dialog waits for an answer: every turn goes to it first.
    pub fn is_waiting(&self) -> bool {
        self.state.is_waiting()
    }

    /// Whether the question was heard and is waiting for its answer, so the confirmation
    /// time limit runs.
    pub fn awaiting_answer(&self) -> bool {
        self.is_waiting() && self.heard_to_the_end
    }

    /// `turn`'s reply was spoken to its end.
    pub fn reply_finished(&mut self, turn: u64) {
        if self.asked_in == Some(turn) {
            self.heard_to_the_end = true;
        }
    }

    /// `turn`'s reply was cut off or failed. A question it was asking was not heard, so it is
    /// dropped, silently.
    pub fn reply_cut(&mut self, turn: u64) {
        if self.asked_in == Some(turn) && !self.heard_to_the_end {
            self.reset();
        }
    }

    /// The conversation ended or moved on: drop anything pending, silently.
    pub fn reset(&mut self) {
        self.state = DialogState::Idle;
        self.asked_in = None;
        self.heard_to_the_end = false;
    }

    /// The user finished `turn`, saying `transcript` (their own words, from the microphone).
    /// `answer_guard` is how long after the floor came back an answer must begin: anything
    /// earlier could be the end of the assistant's own question ("Shall I?"), still coming
    /// out of the speaker or echoing in the room, which would otherwise be taken as a "no".
    pub fn user_said(
        &mut self,
        turn: u64,
        transcript: &str,
        heard: HeardTurn,
        answer_guard: Duration,
        context: &CommandContext,
    ) -> TurnPlan {
        if self.is_waiting() {
            let in_own_turn = heard
                .began_after_floor
                .is_some_and(|began| began >= answer_guard);
            if !self.heard_to_the_end || !in_own_turn {
                tracing::info!("coding agent answer not taken: said over or with the question");
                return TurnPlan::Ignore;
            }
        }
        let speech = UserSpeech::from_user_transcript(transcript);
        self.apply(turn, DialogEvent::UserSaid(speech), context)
    }

    /// Nothing was said for the confirmation time limit after the question was heard.
    /// `turn` is the turn that will say so.
    pub fn timed_out(&mut self, turn: u64, context: &CommandContext) -> TurnPlan {
        if !self.awaiting_answer() {
            return TurnPlan::Ignore;
        }
        self.apply(turn, DialogEvent::TimedOut, context)
    }

    fn apply(&mut self, turn: u64, event: DialogEvent<'_>, context: &CommandContext) -> TurnPlan {
        let was_waiting = self.is_waiting();
        let (state, effects) = dialog::step(std::mem::take(&mut self.state), event, context);
        self.state = state;
        let plan = match effects.into_iter().next() {
            Some(DialogEffect::PassOn) => TurnPlan::PassOn,
            Some(DialogEffect::Say(text)) => TurnPlan::Say(text),
            Some(DialogEffect::Execute(command)) => TurnPlan::Execute(command),
            None => TurnPlan::Ignore,
        };
        if self.is_waiting() {
            // This turn's reply asks the question; it can be answered once it is heard.
            self.asked_in = Some(turn);
            self.heard_to_the_end = false;
        } else {
            self.asked_in = None;
            self.heard_to_the_end = false;
        }
        match (&plan, was_waiting, self.is_waiting()) {
            (TurnPlan::Execute(command), _, _) => tracing::info!(
                agent = command.agent().display_name(),
                project = %command.project().spoken_name,
                command = command.log_kind(),
                "coding agent command confirmed"
            ),
            (_, false, true) => tracing::info!("coding agent command heard; asking"),
            (_, true, false) => tracing::info!("coding agent command not confirmed"),
            _ => {}
        }
        plan
    }
}

/// A background question in progress. The ear holds a copy while the user talks over it.
#[derive(Debug, Clone)]
pub struct AgentJob {
    /// The turn whose reply is waiting for the answer.
    pub turn: u64,
    stop: CancelFlag,
    remind: Arc<AtomicBool>,
}

impl AgentJob {
    pub fn new(turn: u64) -> Self {
        Self {
            turn,
            stop: CancelFlag::default(),
            remind: Arc::new(AtomicBool::new(false)),
        }
    }

    /// What the agent watches: set to stop it.
    pub fn cancel_flag(&self) -> &CancelFlag {
        &self.stop
    }

    /// The user asked to stop the agent.
    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// The user said something else meanwhile: remind them the agent is still working.
    pub fn ask_for_reminder(&self) {
        self.remind.store(true, Ordering::SeqCst);
    }

    /// Whether a reminder was asked for since the last call.
    pub fn take_reminder(&self) -> bool {
        self.remind.swap(false, Ordering::SeqCst)
    }
}

/// Said when the user talks while a coding agent is working on their question.
pub const STILL_WORKING: &str = "Claude is still working. Say stop that to cancel.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding_agents::{projects::CodingProject, speech, AgentKind};

    fn context() -> CommandContext {
        CommandContext {
            projects: ProjectIndex::new(&[CodingProject {
                id: "lf".into(),
                path: r"C:\work\localflow".into(),
                spoken_names: vec!["localflow".into()],
            }]),
            availability: AgentAvailability::all(),
        }
    }

    const GUARD: Duration = Duration::from_millis(300);

    fn own_turn() -> HeardTurn {
        HeardTurn {
            began_after_floor: Some(Duration::from_millis(800)),
        }
    }

    /// A dialog that has read "Open Claude Code in localflow" back in turn 1.
    fn asked() -> AgentDialog {
        let mut dialog = AgentDialog::default();
        let plan = dialog.user_said(
            1,
            "Open Claude Code in localflow",
            own_turn(),
            GUARD,
            &context(),
        );
        assert!(matches!(plan, TurnPlan::Say(ref text) if text.ends_with("Shall I?")));
        assert!(dialog.is_waiting());
        dialog
    }

    #[test]
    fn a_yes_after_the_read_back_was_heard_carries_the_command_out() {
        let mut dialog = asked();
        assert!(!dialog.awaiting_answer(), "not until it has been heard");
        dialog.reply_finished(1);
        assert!(dialog.awaiting_answer());

        let plan = dialog.user_said(3, "Yes.", own_turn(), GUARD, &context());
        match plan {
            TurnPlan::Execute(command) => {
                assert_eq!(command.agent(), AgentKind::ClaudeCode);
                assert_eq!(command.project().id, "lf");
            }
            other => panic!("{other:?}"),
        }
        assert!(!dialog.is_waiting());
    }

    #[test]
    fn a_read_back_cut_off_is_forgotten_and_cannot_be_answered() {
        let mut dialog = asked();
        dialog.reply_cut(1);
        assert!(!dialog.is_waiting());
        // A later "yes" is just a turn.
        assert_eq!(
            dialog.user_said(3, "Yes.", own_turn(), GUARD, &context()),
            TurnPlan::PassOn
        );
    }

    #[test]
    fn a_yes_that_began_over_or_right_after_the_read_back_does_not_count() {
        let mut dialog = asked();
        dialog.reply_finished(1);
        // Begun while the assistant had the floor.
        let over = HeardTurn {
            began_after_floor: None,
        };
        assert_eq!(
            dialog.user_said(3, "Yes.", over, GUARD, &context()),
            TurnPlan::Ignore
        );
        // Begun as the last of the read-back was still coming out of the speaker.
        let too_soon = HeardTurn {
            began_after_floor: Some(Duration::from_millis(100)),
        };
        assert_eq!(
            dialog.user_said(5, "Go ahead.", too_soon, GUARD, &context()),
            TurnPlan::Ignore
        );
        // Still waiting: a real answer counts.
        assert!(dialog.awaiting_answer());
        assert!(matches!(
            dialog.user_said(9, "Yes, go ahead.", own_turn(), GUARD, &context()),
            TurnPlan::Execute(_)
        ));
    }

    #[test]
    fn anything_else_declines_and_silence_times_out() {
        let mut dialog = asked();
        dialog.reply_finished(1);
        assert_eq!(
            dialog.user_said(3, "No.", own_turn(), GUARD, &context()),
            TurnPlan::Say(speech::DECLINED.to_string())
        );
        assert!(!dialog.is_waiting());

        let mut dialog = asked();
        // Not heard yet: the time limit does not run.
        assert_eq!(dialog.timed_out(2, &context()), TurnPlan::Ignore);
        dialog.reply_finished(1);
        assert_eq!(
            dialog.timed_out(3, &context()),
            TurnPlan::Say(speech::TIMED_OUT.to_string())
        );
        assert!(!dialog.is_waiting());
        assert_eq!(dialog.timed_out(5, &context()), TurnPlan::Ignore);
    }

    #[test]
    fn a_question_back_is_answered_the_same_way() {
        let mut dialog = AgentDialog::default();
        let plan = dialog.user_said(1, "Open Claude Code", own_turn(), GUARD, &context());
        assert!(matches!(plan, TurnPlan::Say(ref text) if text.starts_with("Which project")));
        dialog.reply_finished(1);
        let plan = dialog.user_said(3, "localflow", own_turn(), GUARD, &context());
        assert!(
            matches!(plan, TurnPlan::Say(ref text) if text.starts_with("Open Claude Code in localflow?"))
        );
        // The read-back is a new question, asked in turn 3.
        assert!(!dialog.awaiting_answer());
        dialog.reply_finished(1);
        assert!(
            !dialog.awaiting_answer(),
            "the old question's end does not count"
        );
        dialog.reply_finished(3);
        assert!(matches!(
            dialog.user_said(5, "yes", own_turn(), GUARD, &context()),
            TurnPlan::Execute(_)
        ));
    }

    #[test]
    fn resetting_drops_everything_silently() {
        let mut dialog = asked();
        dialog.reply_finished(1);
        dialog.reset();
        assert!(!dialog.is_waiting());
        assert_eq!(
            dialog.user_said(3, "yes", own_turn(), GUARD, &context()),
            TurnPlan::PassOn
        );
    }

    #[test]
    fn ordinary_turns_pass_on_whenever_they_begin() {
        let mut dialog = AgentDialog::default();
        for heard in [own_turn(), HeardTurn::default()] {
            assert_eq!(
                dialog.user_said(1, "What's the capital of France?", heard, GUARD, &context()),
                TurnPlan::PassOn
            );
        }
    }

    #[test]
    fn a_job_can_be_stopped_and_asked_for_one_reminder_at_a_time() {
        let job = AgentJob::new(4);
        let seen_by_the_ear = job.clone();
        assert!(!job.take_reminder());
        seen_by_the_ear.ask_for_reminder();
        assert!(job.take_reminder());
        assert!(!job.take_reminder());
        seen_by_the_ear.stop();
        assert!(job.cancel_flag().is_cancelled());
    }
}
