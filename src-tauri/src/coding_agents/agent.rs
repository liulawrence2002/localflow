//! Coding agents behind one interface, and carrying out a confirmed command.
//!
//! [`CodingAgent`] is what Codex and the Cursor Agent will implement once their command-line
//! tools are installed; today they are [`NotSetUp`]. Visible launches go through a
//! [`Launcher`] so tests can see exactly what would be started without starting it.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::Serialize;

use super::{
    args::{check_no_permission_skipping, cursor_args, terminal_args},
    ask::{ask_question, QuestionRequest, QUESTION_TIME_LIMIT},
    detect::{
        file_exists, find_claude, find_cursor, find_windows_terminal, probe_claude, ClaudeProbe,
        ClaudeSource, SearchPaths, MAX_TURNS_SINCE,
    },
    grammar::CommandContext,
    projects::{validate_folder, FolderProblem, ProjectIndex},
    AgentAvailability, AgentCommand, AgentKind, CodingProject, Unavailability,
};
use crate::cancel::CancelFlag;

/// Why a program could not be started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchFailure {
    Unavailable(Unavailability),
    /// The project folder no longer exists or is no longer a folder.
    FolderUnusable,
    /// A path contains a character that cannot be passed safely.
    UnsafePath,
    /// The argument list failed the permission check. Never expected; a guard.
    Refused,
    /// The operating system did not start the program.
    Spawn,
}

/// How a background question ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    /// The answer, to be spoken after "Claude says:" and never acted on.
    Answer(String),
    AgentError(AgentErrorKind),
    TimedOut,
    Cancelled,
    /// The program ended without a result LocalFlow could read.
    Unreadable,
    CouldNotStart(LaunchFailure),
}

impl AskOutcome {
    /// A label safe to log: never the answer.
    pub fn log_kind(&self) -> &'static str {
        match self {
            Self::Answer(_) => "answered",
            Self::AgentError(AgentErrorKind::NotSignedIn) => "not_signed_in",
            Self::AgentError(AgentErrorKind::MaxTurns) => "max_turns",
            Self::AgentError(AgentErrorKind::DuringExecution) => "error_during_execution",
            Self::AgentError(AgentErrorKind::Other) => "agent_error",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
            Self::Unreadable => "unreadable",
            Self::CouldNotStart(_) => "could_not_start",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentErrorKind {
    /// The agent needs the user to sign in first.
    NotSignedIn,
    /// It used up its turns before answering (`error_max_turns`).
    MaxTurns,
    /// `error_during_execution`.
    DuringExecution,
    Other,
}

/// What carrying out a confirmed command did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionOutcome {
    Opened,
    OpenFailed(LaunchFailure),
    Answered(AskOutcome),
}

impl ExecutionOutcome {
    /// After Claude Code opens, the conversation ends so the user can dictate the task into
    /// its prompt with the normal dictation hotkey.
    pub fn ends_conversation(&self, command: &AgentCommand) -> bool {
        matches!(self, Self::Opened) && command.agent() == AgentKind::ClaudeCode
    }

    fn log_kind(&self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::OpenFailed(_) => "open_failed",
            Self::Answered(answer) => answer.log_kind(),
        }
    }
}

/// What was found for one agent, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub agent: AgentKind,
    pub name: &'static str,
    pub found: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub can_open: bool,
    pub can_ask: bool,
    /// One sentence about what works and what does not.
    pub detail: String,
    #[serde(skip)]
    pub open: Result<(), Unavailability>,
    #[serde(skip)]
    pub ask: Result<(), Unavailability>,
}

impl AgentStatus {
    fn not_set_up(agent: AgentKind) -> Self {
        Self {
            agent,
            name: agent.display_name(),
            found: false,
            path: None,
            version: None,
            can_open: false,
            can_ask: false,
            detail: format!(
                "{} isn't set up for voice commands yet.",
                agent.display_name()
            ),
            open: Err(Unavailability::NotSetUpYet),
            ask: Err(Unavailability::NotSetUpYet),
        }
    }
}

/// A coding agent LocalFlow can drive.
pub trait CodingAgent: Send + Sync {
    fn kind(&self) -> AgentKind;
    /// Look for the agent on this computer and remember what was found. May run the agent's
    /// `--version` and `--help`, and nothing else.
    fn detect(&self) -> AgentStatus;
    /// Open the agent visibly in `folder`, with no task.
    fn open_in_folder(&self, folder: &Path) -> Result<(), LaunchFailure>;
    /// Ask a read-only question in `folder` and wait for the answer.
    fn ask(&self, folder: &Path, question: &str, cancel: &CancelFlag) -> AskOutcome;
}

/// A program to start visibly: never through a shell, one argument per element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    /// Environment variables not passed on.
    pub remove_env: Vec<&'static str>,
}

pub trait Launcher: Send + Sync {
    fn launch(&self, launch: &Launch) -> io::Result<()>;
}

/// Starts the program and lets it run on its own.
pub struct SystemLauncher;

impl Launcher for SystemLauncher {
    fn launch(&self, launch: &Launch) -> io::Result<()> {
        // The last check before anything starts, whoever built the list.
        if check_no_permission_skipping(&launch.args).is_err() {
            return Err(io::Error::other(
                "refused an argument list that skips permissions",
            ));
        }
        let mut command = Command::new(&launch.program);
        command
            .args(&launch.args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for name in &launch.remove_env {
            command.env_remove(name);
        }
        // Not waited for: `wt.exe` hands the tab to Windows Terminal and exits, and the
        // editor keeps running after LocalFlow.
        command.spawn().map(drop)
    }
}

/// Records launches instead of starting anything. For tests.
#[derive(Default)]
pub struct RecordingLauncher {
    pub launches: Mutex<Vec<Launch>>,
}

impl Launcher for RecordingLauncher {
    fn launch(&self, launch: &Launch) -> io::Result<()> {
        self.launches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(launch.clone());
        Ok(())
    }
}

/// How a program is located: the real search, or a fixed answer in tests.
pub trait Locator: Send + Sync {
    fn claude(&self) -> Option<(PathBuf, ClaudeSource)>;
    fn windows_terminal(&self) -> Option<PathBuf>;
    fn cursor(&self) -> Option<PathBuf>;
    fn probe_claude(&self, exe: &Path) -> ClaudeProbe;
}

/// Looks in `PATH` and the usual install folders, and runs `--version` and `--help`.
pub struct SystemLocator;

impl Locator for SystemLocator {
    fn claude(&self) -> Option<(PathBuf, ClaudeSource)> {
        find_claude(&SearchPaths::from_environment(), file_exists)
    }

    fn windows_terminal(&self) -> Option<PathBuf> {
        find_windows_terminal(&SearchPaths::from_environment(), file_exists)
    }

    fn cursor(&self) -> Option<PathBuf> {
        find_cursor(&SearchPaths::from_environment(), file_exists)
    }

    fn probe_claude(&self, exe: &Path) -> ClaudeProbe {
        probe_claude(exe)
    }
}

#[derive(Debug, Clone)]
struct ClaudeInstall {
    exe: PathBuf,
    probe: ClaudeProbe,
    terminal: Option<PathBuf>,
}

/// Claude Code: a visible tab in Windows Terminal, or a background question.
pub struct ClaudeCode {
    locator: Arc<dyn Locator>,
    launcher: Arc<dyn Launcher>,
    found: Mutex<Option<ClaudeInstall>>,
    question_time_limit: Duration,
}

impl ClaudeCode {
    pub fn new(locator: Arc<dyn Locator>, launcher: Arc<dyn Launcher>) -> Self {
        Self {
            locator,
            launcher,
            found: Mutex::new(None),
            question_time_limit: QUESTION_TIME_LIMIT,
        }
    }

    pub fn with_question_time_limit(mut self, limit: Duration) -> Self {
        self.question_time_limit = limit;
        self
    }

    fn install(&self) -> Option<ClaudeInstall> {
        let cached = self
            .found
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if cached.is_some() {
            return cached;
        }
        self.detect();
        self.found
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl CodingAgent for ClaudeCode {
    fn kind(&self) -> AgentKind {
        AgentKind::ClaudeCode
    }

    fn detect(&self) -> AgentStatus {
        let agent = AgentKind::ClaudeCode;
        let Some((exe, _source)) = self.locator.claude() else {
            *self.found.lock().unwrap_or_else(|p| p.into_inner()) = None;
            return AgentStatus {
                agent,
                name: agent.display_name(),
                found: false,
                path: None,
                version: None,
                can_open: false,
                can_ask: false,
                detail: "Claude Code was not found. Install it with npm or the native \
                         installer, then check again."
                    .to_string(),
                open: Err(Unavailability::NotInstalled),
                ask: Err(Unavailability::NotInstalled),
            };
        };
        let probe = self.locator.probe_claude(&exe);
        let terminal = self.locator.windows_terminal();
        let open = match (&terminal, probe.help.normal_permission_mode()) {
            (None, _) => Err(Unavailability::NoTerminal),
            (_, None) => Err(Unavailability::MissingOption),
            _ => Ok(()),
        };
        let missing = probe.help.missing_for_questions();
        let ask = if missing.is_empty() {
            Ok(())
        } else {
            Err(Unavailability::MissingOption)
        };
        let detail = match (open, ask) {
            (Ok(()), Ok(())) => "Can open in Windows Terminal and answer questions.".to_string(),
            (Err(Unavailability::NoTerminal), Ok(())) => {
                "Can answer questions. Opening it needs Windows Terminal, which was not found."
                    .to_string()
            }
            (Err(_), Ok(())) => {
                "Can answer questions. This version has no normal permission mode to open in."
                    .to_string()
            }
            (Ok(()), Err(_)) => format!(
                "Can open in Windows Terminal. Questions need options this version lacks: {}.",
                missing.join(", ")
            ),
            (Err(_), Err(_)) => format!(
                "Found, but this version lacks options LocalFlow needs: {}.",
                missing.join(", ")
            ),
        };
        let status = AgentStatus {
            agent,
            name: agent.display_name(),
            found: true,
            path: Some(exe.to_string_lossy().into_owned()),
            version: probe.version.map(|version| version.to_string()),
            can_open: open.is_ok(),
            can_ask: ask.is_ok(),
            detail,
            open,
            ask,
        };
        *self.found.lock().unwrap_or_else(|p| p.into_inner()) = Some(ClaudeInstall {
            exe,
            probe,
            terminal,
        });
        status
    }

    fn open_in_folder(&self, folder: &Path) -> Result<(), LaunchFailure> {
        let install = self
            .install()
            .ok_or(LaunchFailure::Unavailable(Unavailability::NotInstalled))?;
        let terminal = install
            .terminal
            .ok_or(LaunchFailure::Unavailable(Unavailability::NoTerminal))?;
        let mode = install
            .probe
            .help
            .normal_permission_mode()
            .ok_or(LaunchFailure::Unavailable(Unavailability::MissingOption))?;
        let args = terminal_args(folder, &install.exe, mode)?;
        self.launcher
            .launch(&Launch {
                program: terminal,
                args,
                remove_env: super::run::REMOVED_ENV.to_vec(),
            })
            .map_err(|_| LaunchFailure::Spawn)
    }

    fn ask(&self, folder: &Path, question: &str, cancel: &CancelFlag) -> AskOutcome {
        let Some(install) = self.install() else {
            return AskOutcome::CouldNotStart(LaunchFailure::Unavailable(
                Unavailability::NotInstalled,
            ));
        };
        if !install.probe.help.missing_for_questions().is_empty() {
            return AskOutcome::CouldNotStart(LaunchFailure::Unavailable(
                Unavailability::MissingOption,
            ));
        }
        let use_max_turns = install
            .probe
            .version
            .is_some_and(|version| version >= MAX_TURNS_SINCE);
        ask_question(
            &QuestionRequest {
                exe: &install.exe,
                folder,
                question,
                use_max_turns,
                time_limit: self.question_time_limit,
            },
            cancel,
        )
    }
}

/// The Cursor editor: opens a folder, nothing more. Its agent CLI is a separate program.
pub struct CursorEditor {
    locator: Arc<dyn Locator>,
    launcher: Arc<dyn Launcher>,
    found: Mutex<Option<PathBuf>>,
}

impl CursorEditor {
    pub fn new(locator: Arc<dyn Locator>, launcher: Arc<dyn Launcher>) -> Self {
        Self {
            locator,
            launcher,
            found: Mutex::new(None),
        }
    }
}

impl CodingAgent for CursorEditor {
    fn kind(&self) -> AgentKind {
        AgentKind::Cursor
    }

    fn detect(&self) -> AgentStatus {
        let agent = AgentKind::Cursor;
        let exe = self.locator.cursor();
        *self.found.lock().unwrap_or_else(|p| p.into_inner()) = exe.clone();
        let found = exe.is_some();
        AgentStatus {
            agent,
            name: agent.display_name(),
            found,
            path: exe.map(|exe| exe.to_string_lossy().into_owned()),
            version: None,
            can_open: found,
            can_ask: false,
            detail: if found {
                "Can open a project folder. It is not given a task.".to_string()
            } else {
                "Cursor was not found.".to_string()
            },
            open: if found {
                Ok(())
            } else {
                Err(Unavailability::NotInstalled)
            },
            ask: Err(Unavailability::NotSetUpYet),
        }
    }

    fn open_in_folder(&self, folder: &Path) -> Result<(), LaunchFailure> {
        let cached = self.found.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let exe = match cached {
            Some(exe) => exe,
            None => {
                self.detect();
                self.found
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .clone()
                    .ok_or(LaunchFailure::Unavailable(Unavailability::NotInstalled))?
            }
        };
        let args = cursor_args(folder)?;
        let mut remove_env = super::run::REMOVED_ENV.to_vec();
        // Would make the editor run as a plain Node.js process instead of opening a window.
        remove_env.push("ELECTRON_RUN_AS_NODE");
        self.launcher
            .launch(&Launch {
                program: exe,
                args,
                remove_env,
            })
            .map_err(|_| LaunchFailure::Spawn)
    }

    fn ask(&self, _folder: &Path, _question: &str, _cancel: &CancelFlag) -> AskOutcome {
        AskOutcome::CouldNotStart(LaunchFailure::Unavailable(Unavailability::NotSetUpYet))
    }
}

/// An agent LocalFlow knows the name of but cannot drive yet.
pub struct NotSetUp(pub AgentKind);

impl CodingAgent for NotSetUp {
    fn kind(&self) -> AgentKind {
        self.0
    }

    fn detect(&self) -> AgentStatus {
        AgentStatus::not_set_up(self.0)
    }

    fn open_in_folder(&self, _folder: &Path) -> Result<(), LaunchFailure> {
        Err(LaunchFailure::Unavailable(Unavailability::NotSetUpYet))
    }

    fn ask(&self, _folder: &Path, _question: &str, _cancel: &CancelFlag) -> AskOutcome {
        AskOutcome::CouldNotStart(LaunchFailure::Unavailable(Unavailability::NotSetUpYet))
    }
}

/// What was found on this computer, for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingAgentsStatus {
    pub agents: Vec<AgentStatus>,
    pub windows_terminal: Option<String>,
}

/// All agents, and what each could do at the last detection.
pub struct CodingAgents {
    agents: Vec<Arc<dyn CodingAgent>>,
    locator: Arc<dyn Locator>,
    availability: Mutex<Option<AgentAvailability>>,
}

impl CodingAgents {
    /// The real agents, found on this computer when first needed.
    pub fn system() -> Self {
        let locator: Arc<dyn Locator> = Arc::new(SystemLocator);
        let launcher: Arc<dyn Launcher> = Arc::new(SystemLauncher);
        Self::new(
            vec![
                Arc::new(ClaudeCode::new(locator.clone(), launcher.clone())),
                Arc::new(CursorEditor::new(locator.clone(), launcher)),
                Arc::new(NotSetUp(AgentKind::Codex)),
                Arc::new(NotSetUp(AgentKind::CursorAgent)),
            ],
            locator,
        )
    }

    pub fn new(agents: Vec<Arc<dyn CodingAgent>>, locator: Arc<dyn Locator>) -> Self {
        Self {
            agents,
            locator,
            availability: Mutex::new(None),
        }
    }

    pub fn agent(&self, kind: AgentKind) -> Option<&Arc<dyn CodingAgent>> {
        self.agents.iter().find(|agent| agent.kind() == kind)
    }

    /// Look for every agent again. Runs `claude --version` and `--help`; call it off the UI
    /// thread.
    pub fn detect_all(&self) -> CodingAgentsStatus {
        let agents: Vec<AgentStatus> = self.agents.iter().map(|agent| agent.detect()).collect();
        let status_of = |kind| agents.iter().find(|status| status.agent == kind);
        let availability = AgentAvailability {
            open_claude: status_of(AgentKind::ClaudeCode)
                .map_or(Err(Unavailability::NotInstalled), |s| s.open),
            ask_claude: status_of(AgentKind::ClaudeCode)
                .map_or(Err(Unavailability::NotInstalled), |s| s.ask),
            open_cursor: status_of(AgentKind::Cursor)
                .map_or(Err(Unavailability::NotInstalled), |s| s.open),
        };
        *self.availability.lock().unwrap_or_else(|p| p.into_inner()) = Some(availability);
        CodingAgentsStatus {
            agents,
            windows_terminal: self
                .locator
                .windows_terminal()
                .map(|path| path.to_string_lossy().into_owned()),
        }
    }

    /// What each agent can do, detecting first if that has not happened yet.
    pub fn availability(&self) -> AgentAvailability {
        let known = *self.availability.lock().unwrap_or_else(|p| p.into_inner());
        match known {
            Some(availability) => availability,
            None => {
                self.detect_all();
                self.availability
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .unwrap_or_else(AgentAvailability::none)
            }
        }
    }

    /// What the command matcher needs: the user's projects and what can run.
    pub fn context(&self, projects: &[CodingProject]) -> CommandContext {
        CommandContext {
            projects: ProjectIndex::new(projects),
            availability: self.availability(),
        }
    }

    /// Carry out a command the user confirmed. Blocks: an open returns at once, a question
    /// waits for the answer (up to the time limit) or for `cancel`. Logs which agent and which
    /// project, never the question or the answer.
    pub fn execute(&self, command: &AgentCommand, cancel: &CancelFlag) -> ExecutionOutcome {
        let agent_kind = command.agent();
        let project = command.project();
        let outcome = match self.agent(agent_kind) {
            None => match command {
                AgentCommand::Open { .. } => ExecutionOutcome::OpenFailed(
                    LaunchFailure::Unavailable(Unavailability::NotSetUpYet),
                ),
                AgentCommand::Ask { .. } => ExecutionOutcome::Answered(AskOutcome::CouldNotStart(
                    LaunchFailure::Unavailable(Unavailability::NotSetUpYet),
                )),
            },
            Some(agent) => {
                // The folder is checked again: it may have moved since it was added.
                match project.path.to_str().map(validate_folder) {
                    Some(Ok(folder)) => match command {
                        AgentCommand::Open { .. } => match agent.open_in_folder(&folder) {
                            Ok(()) => ExecutionOutcome::Opened,
                            Err(failure) => ExecutionOutcome::OpenFailed(failure),
                        },
                        AgentCommand::Ask { question, .. } => {
                            ExecutionOutcome::Answered(agent.ask(&folder, question, cancel))
                        }
                    },
                    other => {
                        let failure = match other {
                            Some(Err(FolderProblem::TerminalCharacter(_))) => {
                                LaunchFailure::UnsafePath
                            }
                            _ => LaunchFailure::FolderUnusable,
                        };
                        match command {
                            AgentCommand::Open { .. } => ExecutionOutcome::OpenFailed(failure),
                            AgentCommand::Ask { .. } => {
                                ExecutionOutcome::Answered(AskOutcome::CouldNotStart(failure))
                            }
                        }
                    }
                }
            }
        };
        tracing::info!(
            agent = agent_kind.display_name(),
            project = %project.spoken_name,
            command = command.log_kind(),
            outcome = outcome.log_kind(),
            "coding agent command"
        );
        outcome
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::coding_agents::{
        detect::{parse_help, Version},
        ProjectRef,
    };

    const HELP: &str = include_str!("testdata/claude_help_2_1_170.txt");
    pub(crate) const CLAUDE_EXE: &str =
        r"C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe";
    const WT: &str = r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\wt.exe";
    const CURSOR: &str = r"C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe";

    /// A computer with fixed programs on it.
    pub(crate) struct FakeLocator {
        pub claude: Option<PathBuf>,
        pub terminal: Option<PathBuf>,
        pub cursor: Option<PathBuf>,
        pub help: String,
        pub version: Option<Version>,
    }

    impl FakeLocator {
        pub(crate) fn everything() -> Self {
            Self {
                claude: Some(PathBuf::from(CLAUDE_EXE)),
                terminal: Some(PathBuf::from(WT)),
                cursor: Some(PathBuf::from(CURSOR)),
                help: HELP.to_string(),
                version: Some(Version::new(2, 1, 170)),
            }
        }
    }

    impl Locator for FakeLocator {
        fn claude(&self) -> Option<(PathBuf, ClaudeSource)> {
            self.claude
                .clone()
                .map(|exe| (exe, ClaudeSource::NpmPackage))
        }
        fn windows_terminal(&self) -> Option<PathBuf> {
            self.terminal.clone()
        }
        fn cursor(&self) -> Option<PathBuf> {
            self.cursor.clone()
        }
        fn probe_claude(&self, _exe: &Path) -> ClaudeProbe {
            ClaudeProbe {
                version: self.version,
                help: parse_help(&self.help),
            }
        }
    }

    fn agents_with(locator: FakeLocator) -> (CodingAgents, Arc<RecordingLauncher>) {
        let locator: Arc<dyn Locator> = Arc::new(locator);
        let launcher = Arc::new(RecordingLauncher::default());
        let as_launcher: Arc<dyn Launcher> = launcher.clone();
        let agents = CodingAgents::new(
            vec![
                Arc::new(ClaudeCode::new(locator.clone(), as_launcher.clone())),
                Arc::new(CursorEditor::new(locator.clone(), as_launcher)),
                Arc::new(NotSetUp(AgentKind::Codex)),
                Arc::new(NotSetUp(AgentKind::CursorAgent)),
            ],
            locator,
        );
        (agents, launcher)
    }

    fn this_folder() -> ProjectRef {
        ProjectRef {
            id: "here".into(),
            path: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            spoken_name: "localflow".into(),
            parent_name: "localflow".into(),
        }
    }

    fn open(agent: AgentKind, project: ProjectRef) -> AgentCommand {
        AgentCommand::Open {
            agent,
            project,
            task_was_said: true,
        }
    }

    #[test]
    fn opening_claude_code_starts_one_terminal_tab_with_no_task() {
        let (agents, launcher) = agents_with(FakeLocator::everything());
        let outcome = agents.execute(
            &open(AgentKind::ClaudeCode, this_folder()),
            &CancelFlag::default(),
        );
        assert_eq!(outcome, ExecutionOutcome::Opened);
        assert!(outcome.ends_conversation(&open(AgentKind::ClaudeCode, this_folder())));

        let launches = launcher.launches.lock().unwrap();
        assert_eq!(launches.len(), 1);
        let launch = &launches[0];
        assert_eq!(launch.program, PathBuf::from(WT));
        let args: Vec<String> = launch
            .args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-w",
                "0",
                "new-tab",
                "-d",
                env!("CARGO_MANIFEST_DIR"),
                CLAUDE_EXE,
                "--permission-mode",
                "default"
            ]
        );
        assert!(check_no_permission_skipping(&launch.args).is_ok());
    }

    #[test]
    fn opening_cursor_passes_only_the_folder_to_the_real_executable() {
        let (agents, launcher) = agents_with(FakeLocator::everything());
        let command = open(AgentKind::Cursor, this_folder());
        let outcome = agents.execute(&command, &CancelFlag::default());
        assert_eq!(outcome, ExecutionOutcome::Opened);
        assert!(!outcome.ends_conversation(&command));
        let launches = launcher.launches.lock().unwrap();
        assert_eq!(launches[0].program, PathBuf::from(CURSOR));
        assert_eq!(
            launches[0].args,
            [OsString::from(env!("CARGO_MANIFEST_DIR"))]
        );
        assert!(launches[0].remove_env.contains(&"ELECTRON_RUN_AS_NODE"));
    }

    #[test]
    fn nothing_is_launched_without_windows_terminal() {
        let (agents, launcher) = agents_with(FakeLocator {
            terminal: None,
            ..FakeLocator::everything()
        });
        assert_eq!(
            agents.execute(
                &open(AgentKind::ClaudeCode, this_folder()),
                &CancelFlag::default()
            ),
            ExecutionOutcome::OpenFailed(LaunchFailure::Unavailable(Unavailability::NoTerminal))
        );
        assert!(launcher.launches.lock().unwrap().is_empty());
        assert_eq!(
            agents.availability().open_claude,
            Err(Unavailability::NoTerminal)
        );
        assert_eq!(agents.availability().ask_claude, Ok(()));
    }

    #[test]
    fn nothing_is_launched_without_a_normal_permission_mode() {
        let (agents, launcher) = agents_with(FakeLocator {
            help: HELP.replace("\"default\"", "\"normal\""),
            ..FakeLocator::everything()
        });
        assert_eq!(
            agents.execute(
                &open(AgentKind::ClaudeCode, this_folder()),
                &CancelFlag::default()
            ),
            ExecutionOutcome::OpenFailed(LaunchFailure::Unavailable(Unavailability::MissingOption))
        );
        assert!(launcher.launches.lock().unwrap().is_empty());
    }

    #[test]
    fn a_folder_that_disappeared_is_not_opened() {
        let (agents, launcher) = agents_with(FakeLocator::everything());
        let gone = ProjectRef {
            path: PathBuf::from(r"C:\definitely\gone\now"),
            ..this_folder()
        };
        assert_eq!(
            agents.execute(&open(AgentKind::ClaudeCode, gone), &CancelFlag::default()),
            ExecutionOutcome::OpenFailed(LaunchFailure::FolderUnusable)
        );
        let unsafe_path = ProjectRef {
            path: PathBuf::from(r"C:\work\a;calc"),
            ..this_folder()
        };
        assert_eq!(
            agents.execute(
                &open(AgentKind::Cursor, unsafe_path),
                &CancelFlag::default()
            ),
            ExecutionOutcome::OpenFailed(LaunchFailure::UnsafePath)
        );
        assert!(launcher.launches.lock().unwrap().is_empty());
    }

    #[test]
    fn agents_not_set_up_do_nothing() {
        let (agents, launcher) = agents_with(FakeLocator::everything());
        assert_eq!(
            agents.execute(
                &open(AgentKind::Codex, this_folder()),
                &CancelFlag::default()
            ),
            ExecutionOutcome::OpenFailed(LaunchFailure::Unavailable(Unavailability::NotSetUpYet))
        );
        let ask = AgentCommand::Ask {
            agent: AgentKind::CursorAgent,
            project: this_folder(),
            question: "what is this".into(),
        };
        assert_eq!(
            agents.execute(&ask, &CancelFlag::default()),
            ExecutionOutcome::Answered(AskOutcome::CouldNotStart(LaunchFailure::Unavailable(
                Unavailability::NotSetUpYet
            )))
        );
        assert!(launcher.launches.lock().unwrap().is_empty());
    }

    #[test]
    fn detection_reports_each_agent() {
        let (agents, _) = agents_with(FakeLocator::everything());
        let status = agents.detect_all();
        assert_eq!(status.windows_terminal.as_deref(), Some(WT));
        let claude = &status.agents[0];
        assert_eq!(claude.agent, AgentKind::ClaudeCode);
        assert_eq!(claude.path.as_deref(), Some(CLAUDE_EXE));
        assert_eq!(claude.version.as_deref(), Some("2.1.170"));
        assert!(claude.can_open && claude.can_ask);
        assert!(status.agents[1].can_open && !status.agents[1].can_ask);
        for not_set_up in &status.agents[2..] {
            assert!(!not_set_up.found);
            assert!(not_set_up
                .detail
                .ends_with("isn't set up for voice commands yet."));
        }
        assert_eq!(agents.availability(), AgentAvailability::all());

        let (missing, _) = agents_with(FakeLocator {
            claude: None,
            cursor: None,
            ..FakeLocator::everything()
        });
        assert_eq!(missing.availability(), AgentAvailability::none());
    }

    #[test]
    fn status_serialises_for_the_settings_screen() {
        let (agents, _) = agents_with(FakeLocator::everything());
        let json = serde_json::to_value(agents.detect_all()).unwrap();
        assert_eq!(json["agents"][0]["agent"], "claudeCode");
        assert_eq!(json["agents"][0]["canOpen"], true);
        assert!(json["agents"][0].get("open").is_none());
        assert_eq!(json["windowsTerminal"], WT);
    }

    fn ask_fake(version: Option<Version>) -> Vec<String> {
        let (agents, launcher) = agents_with(FakeLocator {
            claude: Some(crate::coding_agents::ask::with_fake_agent::fake_agent()),
            version,
            ..FakeLocator::everything()
        });
        let command = AgentCommand::Ask {
            agent: AgentKind::ClaudeCode,
            project: this_folder(),
            question: "FAKE:echo what is this".into(),
        };
        let outcome = agents.execute(&command, &CancelFlag::default());
        assert!(
            launcher.launches.lock().unwrap().is_empty(),
            "nothing visible"
        );
        let json = match outcome {
            ExecutionOutcome::Answered(AskOutcome::Answer(json)) => json,
            other => panic!("{other:?}"),
        };
        let echo: serde_json::Value = serde_json::from_str(&json).unwrap();
        echo["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap().to_string())
            .collect()
    }

    /// Prints what this computer has. Runs `claude --version` and `--help` and nothing else;
    /// never launches anything.
    #[test]
    #[ignore = "looks at this computer's installs"]
    fn detects_what_is_installed_here() {
        let status = CodingAgents::system().detect_all();
        println!("{status:#?}");
        assert_eq!(status.agents.len(), 4);
    }

    /// The live check of the visible launch, through the real Windows Terminal, with a
    /// stand-in for `claude.exe` that writes down what it received and exits. Opens a
    /// Windows Terminal tab on the screen for a moment. Set:
    /// - `LOCALFLOW_WT_PROBE`: the stand-in program;
    /// - `LOCALFLOW_WT_PROBE_OUT`: the file it writes (`cwd=...`, then `arg<n>=[...]`);
    /// - `LOCALFLOW_WT_PROBE_FOLDER`: the folder the tab should open in (a space in it tests
    ///   Windows Terminal's re-quoting).
    #[test]
    #[ignore = "opens a Windows Terminal tab; needs LOCALFLOW_WT_PROBE and friends"]
    fn real_windows_terminal_passes_the_folder_and_arguments_through() {
        use crate::coding_agents::{
            args::terminal_args,
            detect::{file_exists, find_windows_terminal, SearchPaths},
        };
        let var = |name: &str| PathBuf::from(std::env::var_os(name).expect(name));
        let probe = var("LOCALFLOW_WT_PROBE");
        let out = var("LOCALFLOW_WT_PROBE_OUT");
        let folder = var("LOCALFLOW_WT_PROBE_FOLDER");
        let _ = std::fs::remove_file(&out);

        let wt = find_windows_terminal(&SearchPaths::from_environment(), file_exists)
            .expect("Windows Terminal is installed");
        let args = terminal_args(&folder, &probe, "default").expect("safe paths");
        SystemLauncher
            .launch(&Launch {
                program: wt,
                args,
                remove_env: Vec::new(),
            })
            .expect("wt.exe starts");

        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let received = loop {
            if let Ok(text) = std::fs::read_to_string(&out) {
                if text.lines().count() >= 4 {
                    break text;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the stand-in never ran"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        // Let the stand-in finish writing anything more it was given.
        std::thread::sleep(Duration::from_millis(500));
        let received = std::fs::read_to_string(&out).unwrap_or(received);
        println!("{received}");
        let lines: Vec<&str> = received.lines().collect();
        assert_eq!(lines[0], format!("cwd={}", folder.display()));
        assert_eq!(lines[2], "arg1=[--permission-mode]");
        assert_eq!(lines[3], "arg2=[default]");
        assert_eq!(lines.len(), 4, "nothing else may reach the agent");
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn a_confirmed_question_runs_in_the_background_with_max_turns_gated_by_version() {
        let args = ask_fake(Some(Version::new(2, 1, 170)));
        assert!(args.windows(2).any(|pair| pair == ["--max-turns", "20"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--permission-mode", "dontAsk"]));

        let args = ask_fake(None);
        assert!(
            !args.iter().any(|arg| arg == "--max-turns"),
            "unknown version: left out"
        );
    }
}
