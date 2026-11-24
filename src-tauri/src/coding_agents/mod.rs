//! Voice commands for coding agents (milestone 5).
//!
//! In a conversation the user can say "open Claude Code in localflow", "ask Claude what the
//! overlay code in localflow does" or "open localflow in Cursor". This module recognises
//! those phrases with fixed rules (no language model), asks about anything unclear, reads the
//! command back for a spoken "yes", and then carries it out:
//!
//! - [`grammar`] turns the user's transcript into an [`AgentCommand`] or a question back.
//! - [`dialog`] is the explicit state machine for clarifying and confirming.
//! - [`speech`] has every sentence spoken, including the read-back.
//! - [`agent`] holds the [`agent::CodingAgent`] trait and the Claude Code, Cursor and
//!   not-yet-supported agents behind it.
//! - [`args`] builds every argument list and refuses permission-skipping options.
//! - [`ask`] runs a read-only background question in a kill-on-close job.
//! - [`detect`] finds the installed programs.
//! - [`projects`] and [`spoken_forms`] hold the user's project folders and their names.
//!
//! Privacy: a task is never sent anywhere (the user dictates it into the agent's own prompt);
//! a question is sent to the agent only after it was read back and confirmed. Logs name the
//! agent and the project's spoken name, never the task, the question or the answer.

pub mod agent;
pub mod args;
pub mod ask;
pub mod commands;
pub mod detect;
pub mod dialog;
pub mod grammar;
pub mod phrases;
pub mod projects;
pub mod run;
pub mod speech;
pub mod spoken_forms;

use std::path::PathBuf;

use serde::Serialize;

pub use agent::{CodingAgents, ExecutionOutcome};
pub use commands::CodingAgentsRuntime;
pub use dialog::{DialogEffect, DialogEvent, DialogState, CONFIRMATION_TIMEOUT};
pub use grammar::{recognize, CommandContext, Recognition, UserSpeech};
pub use projects::{CodingAgentSettings, CodingProject};

/// The coding agents a user can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentKind {
    ClaudeCode,
    /// The Cursor editor, which can only open a folder.
    Cursor,
    Codex,
    CursorAgent,
}

impl AgentKind {
    /// The name as spoken and shown: "Claude Code".
    pub fn display_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Cursor => "Cursor",
            Self::Codex => "Codex",
            Self::CursorAgent => "Cursor Agent",
        }
    }

    /// The name used for the agent answering a question: "Claude".
    pub fn short_name(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude",
            other => other.display_name(),
        }
    }
}

/// A project as it was when the command was recognised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    pub id: String,
    pub path: PathBuf,
    /// How the user says it; used in every spoken sentence.
    pub spoken_name: String,
    /// The name of the folder the project is in, to tell two same-named projects apart.
    pub parent_name: String,
}

/// A recognised command, complete and ready to be read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentCommand {
    /// Open the agent visibly in the project folder. A task, if one was said, is not passed
    /// on; the user dictates it into the agent's own prompt.
    Open {
        agent: AgentKind,
        project: ProjectRef,
        task_was_said: bool,
    },
    /// Ask a read-only question in the background and speak the answer.
    Ask {
        agent: AgentKind,
        project: ProjectRef,
        /// The user's words, verbatim.
        question: String,
    },
}

impl AgentCommand {
    pub fn agent(&self) -> AgentKind {
        match self {
            Self::Open { agent, .. } | Self::Ask { agent, .. } => *agent,
        }
    }

    pub fn project(&self) -> &ProjectRef {
        match self {
            Self::Open { project, .. } | Self::Ask { project, .. } => project,
        }
    }

    /// A label safe to log: the kind of command, never its text.
    pub fn log_kind(&self) -> &'static str {
        match self {
            Self::Open { .. } => "open",
            Self::Ask { .. } => "ask",
        }
    }
}

/// A command still missing its project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandDraft {
    Open {
        agent: AgentKind,
        task_was_said: bool,
    },
    Ask {
        agent: AgentKind,
        question: String,
    },
}

impl CommandDraft {
    pub fn agent(&self) -> AgentKind {
        match self {
            Self::Open { agent, .. } | Self::Ask { agent, .. } => *agent,
        }
    }

    pub fn complete(self, project: ProjectRef) -> AgentCommand {
        match self {
            Self::Open {
                agent,
                task_was_said,
            } => AgentCommand::Open {
                agent,
                project,
                task_was_said,
            },
            Self::Ask { agent, question } => AgentCommand::Ask {
                agent,
                project,
                question,
            },
        }
    }
}

/// Why an agent cannot do what was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unavailability {
    /// LocalFlow has no support for this agent yet (Codex, Cursor Agent).
    NotSetUpYet,
    /// The program was not found on this computer.
    NotInstalled,
    /// Opening a visible tab needs Windows Terminal, which was not found.
    NoTerminal,
    /// The installed version lacks an option LocalFlow relies on.
    MissingOption,
}

/// What each agent can do on this computer, from the last detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentAvailability {
    pub open_claude: Result<(), Unavailability>,
    pub ask_claude: Result<(), Unavailability>,
    pub open_cursor: Result<(), Unavailability>,
}

impl AgentAvailability {
    /// Everything available; for tests and previews.
    pub fn all() -> Self {
        Self {
            open_claude: Ok(()),
            ask_claude: Ok(()),
            open_cursor: Ok(()),
        }
    }

    /// Nothing found yet.
    pub fn none() -> Self {
        Self {
            open_claude: Err(Unavailability::NotInstalled),
            ask_claude: Err(Unavailability::NotInstalled),
            open_cursor: Err(Unavailability::NotInstalled),
        }
    }
}
