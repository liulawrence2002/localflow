//! A read-only question to Claude Code, run in the background.
//!
//! `claude.exe -p` runs in the project folder with only the read and search tools, in
//! don't-ask permission mode (anything needing permission is refused, never prompted), with
//! no MCP servers, no project or local settings, no hooks and no saved session. The
//! question goes in on standard input as UTF-8 and the single JSON result comes out on
//! standard output. The process tree runs in a kill-on-close job with a time limit, and is
//! killed when the question is cancelled.

use std::{path::Path, time::Duration};

use serde::Deserialize;

use super::{
    agent::{AgentErrorKind, AskOutcome, LaunchFailure},
    args::{check_no_permission_skipping, question_args},
    run::{run_captured, RunEnd, RunLimits, RunResult},
};
use crate::cancel::CancelFlag;

/// Wall-clock limit for one question, including a retry.
pub const QUESTION_TIME_LIMIT: Duration = Duration::from_secs(120);
/// The JSON result is small; anything beyond this is not kept.
const MAX_STDOUT: usize = 2 * 1024 * 1024;
const MAX_STDERR: usize = 64 * 1024;

#[derive(Debug, Clone)]
pub struct QuestionRequest<'a> {
    pub exe: &'a Path,
    pub folder: &'a Path,
    pub question: &'a str,
    /// Pass `--max-turns`; see `detect::MAX_TURNS_SINCE`.
    pub use_max_turns: bool,
    pub time_limit: Duration,
}

/// Ask, wait for the answer, and say what happened.
pub fn ask_question(request: &QuestionRequest<'_>, cancel: &CancelFlag) -> AskOutcome {
    let started = std::time::Instant::now();
    let first = run_once(request, request.use_max_turns, request.time_limit, cancel);
    match first {
        Attempt::UnknownMaxTurns if request.use_max_turns && !cancel.is_cancelled() => {
            let remaining = request.time_limit.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return AskOutcome::TimedOut;
            }
            match run_once(request, false, remaining, cancel) {
                Attempt::Done(outcome) => outcome,
                Attempt::UnknownMaxTurns => AskOutcome::Unreadable,
            }
        }
        Attempt::Done(outcome) => outcome,
        Attempt::UnknownMaxTurns => AskOutcome::Unreadable,
    }
}

enum Attempt {
    Done(AskOutcome),
    /// The program said `--max-turns` is an unknown option; worth one retry without it.
    UnknownMaxTurns,
}

fn run_once(
    request: &QuestionRequest<'_>,
    use_max_turns: bool,
    time_limit: Duration,
    cancel: &CancelFlag,
) -> Attempt {
    let args = question_args(use_max_turns);
    if check_no_permission_skipping(&args).is_err() {
        return Attempt::Done(AskOutcome::CouldNotStart(LaunchFailure::Refused));
    }
    let limits = RunLimits {
        time_limit,
        max_stdout: MAX_STDOUT,
        max_stderr: MAX_STDERR,
    };
    match run_captured(
        request.exe,
        &args,
        Some(request.folder),
        Some(request.question),
        limits,
        cancel,
    ) {
        Ok(result) => interpret(&result),
        Err(_) => Attempt::Done(AskOutcome::CouldNotStart(LaunchFailure::Spawn)),
    }
}

fn interpret(result: &RunResult) -> Attempt {
    match result.end {
        RunEnd::Cancelled => return Attempt::Done(AskOutcome::Cancelled),
        RunEnd::TimedOut => return Attempt::Done(AskOutcome::TimedOut),
        RunEnd::Exited(_) => {}
    }
    if let Some(message) = parse_result(&result.stdout) {
        return Attempt::Done(outcome_of(&message));
    }
    let stderr = result.stderr.to_ascii_lowercase();
    if stderr.contains("unknown option") && stderr.contains("max-turns") {
        return Attempt::UnknownMaxTurns;
    }
    if looks_like_sign_in(&stderr) || looks_like_sign_in(&result.stdout.to_ascii_lowercase()) {
        return Attempt::Done(AskOutcome::AgentError(AgentErrorKind::NotSignedIn));
    }
    Attempt::Done(AskOutcome::Unreadable)
}

/// The fields of Claude Code's `--output-format json` result that are used.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ResultMessage {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub subtype: Option<String>,
    pub is_error: Option<bool>,
    pub result: Option<String>,
}

/// The result object: the whole output, or failing that the last line that is one.
pub fn parse_result(stdout: &str) -> Option<ResultMessage> {
    let is_result = |message: &ResultMessage| {
        message.kind.as_deref().is_none_or(|kind| kind == "result")
            && (message.subtype.is_some() || message.result.is_some() || message.is_error.is_some())
    };
    if let Ok(message) = serde_json::from_str::<ResultMessage>(stdout.trim()) {
        if is_result(&message) {
            return Some(message);
        }
    }
    stdout.lines().rev().find_map(|line| {
        let line = line.trim();
        if !line.starts_with('{') {
            return None;
        }
        serde_json::from_str::<ResultMessage>(line)
            .ok()
            .filter(|message| is_result(message))
    })
}

pub fn outcome_of(message: &ResultMessage) -> AskOutcome {
    let text = message.result.as_deref().unwrap_or("").trim();
    let subtype = message.subtype.as_deref().unwrap_or("");
    if message.is_error == Some(true) {
        if looks_like_sign_in(&text.to_ascii_lowercase()) {
            return AskOutcome::AgentError(AgentErrorKind::NotSignedIn);
        }
        return AskOutcome::AgentError(error_kind(subtype));
    }
    match subtype {
        "success" if !text.is_empty() => AskOutcome::Answer(text.to_string()),
        "success" => AskOutcome::AgentError(AgentErrorKind::Other),
        other => AskOutcome::AgentError(error_kind(other)),
    }
}

fn error_kind(subtype: &str) -> AgentErrorKind {
    match subtype {
        "error_max_turns" => AgentErrorKind::MaxTurns,
        "error_during_execution" => AgentErrorKind::DuringExecution,
        _ => AgentErrorKind::Other,
    }
}

/// Phrases Claude Code uses when it is not signed in. Checked on lower-cased text.
fn looks_like_sign_in(text: &str) -> bool {
    [
        "/login",
        "not logged in",
        "please log in",
        "log in to",
        "invalid api key",
        "authentication_error",
        "authentication failed",
        "oauth token has expired",
        "oauth token revoked",
    ]
    .iter()
    .any(|phrase| text.contains(phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(stdout: &str, stderr: &str, end: RunEnd) -> RunResult {
        RunResult {
            end,
            stdout: stdout.into(),
            stderr: stderr.into(),
            elapsed: Duration::ZERO,
        }
    }

    fn outcome(stdout: &str, stderr: &str) -> AskOutcome {
        match interpret(&result(stdout, stderr, RunEnd::Exited(Some(0)))) {
            Attempt::Done(outcome) => outcome,
            Attempt::UnknownMaxTurns => panic!("unexpected retry"),
        }
    }

    #[test]
    fn a_successful_result_is_the_answer() {
        let stdout = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":2100,"num_turns":3,"result":"It draws the waveform.","session_id":"x","total_cost_usd":0.01}"#;
        assert_eq!(
            outcome(stdout, ""),
            AskOutcome::Answer("It draws the waveform.".into())
        );
    }

    #[test]
    fn the_result_line_is_found_among_other_output() {
        let stdout = "Warming up...\n{\"type\":\"system\"}\n{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"Yes.\"}\n";
        assert_eq!(outcome(stdout, ""), AskOutcome::Answer("Yes.".into()));
    }

    #[test]
    fn error_subtypes_are_told_apart() {
        let message = |subtype: &str, is_error: bool| {
            format!(
                r#"{{"type":"result","subtype":"{subtype}","is_error":{is_error},"result":""}}"#
            )
        };
        assert_eq!(
            outcome(&message("error_max_turns", true), ""),
            AskOutcome::AgentError(AgentErrorKind::MaxTurns)
        );
        assert_eq!(
            outcome(&message("error_max_turns", false), ""),
            AskOutcome::AgentError(AgentErrorKind::MaxTurns)
        );
        assert_eq!(
            outcome(&message("error_during_execution", true), ""),
            AskOutcome::AgentError(AgentErrorKind::DuringExecution)
        );
        assert_eq!(
            outcome(&message("something_new", true), ""),
            AskOutcome::AgentError(AgentErrorKind::Other)
        );
        assert_eq!(
            outcome(&message("success", false), ""),
            AskOutcome::AgentError(AgentErrorKind::Other),
            "an empty answer is not an answer"
        );
    }

    #[test]
    fn a_login_failure_is_recognised() {
        let stdout = r#"{"type":"result","subtype":"success","is_error":true,"result":"Invalid API key · Please run /login"}"#;
        assert_eq!(
            outcome(stdout, ""),
            AskOutcome::AgentError(AgentErrorKind::NotSignedIn)
        );
        assert_eq!(
            outcome("", "Error: Not logged in. Please run /login"),
            AskOutcome::AgentError(AgentErrorKind::NotSignedIn)
        );
    }

    #[test]
    fn junk_output_is_unreadable() {
        assert_eq!(outcome("not json at all", ""), AskOutcome::Unreadable);
        assert_eq!(
            outcome("{\"type\":\"assistant\"}", ""),
            AskOutcome::Unreadable
        );
        assert_eq!(outcome("", ""), AskOutcome::Unreadable);
    }

    #[test]
    fn an_unknown_max_turns_option_asks_for_a_retry() {
        let attempt = interpret(&result(
            "",
            "error: unknown option '--max-turns'",
            RunEnd::Exited(Some(1)),
        ));
        assert!(matches!(attempt, Attempt::UnknownMaxTurns));
        // Another unknown option is not retried.
        let attempt = interpret(&result(
            "",
            "error: unknown option '--tools'",
            RunEnd::Exited(Some(1)),
        ));
        assert!(matches!(attempt, Attempt::Done(AskOutcome::Unreadable)));
    }

    #[test]
    fn time_outs_and_cancels_win_over_any_output() {
        let stdout = r#"{"type":"result","subtype":"success","is_error":false,"result":"late"}"#;
        assert!(matches!(
            interpret(&result(stdout, "", RunEnd::TimedOut)),
            Attempt::Done(AskOutcome::TimedOut)
        ));
        assert!(matches!(
            interpret(&result(stdout, "", RunEnd::Cancelled)),
            Attempt::Done(AskOutcome::Cancelled)
        ));
    }
}

/// The runner against a real process: `examples/fake_agent.rs`, which behaves as the question
/// tells it to. Build it first:
///
/// ```text
/// cargo build --release --example fake_agent
/// cargo test --release --lib coding_agents -- --ignored
/// ```
#[cfg(all(test, windows))]
pub(crate) mod with_fake_agent {
    use super::*;
    use crate::{coding_agents::args::question_args, platform::process::is_process_running};
    use std::{path::PathBuf, time::Instant};

    pub(crate) fn fake_agent() -> PathBuf {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let exe = target
            .join("release")
            .join("examples")
            .join("fake_agent.exe");
        assert!(
            exe.is_file(),
            "build it first: cargo build --release --example fake_agent ({})",
            exe.display()
        );
        exe
    }

    fn folder() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    fn ask_with(
        question: &str,
        use_max_turns: bool,
        limit: Duration,
        cancel: &CancelFlag,
    ) -> AskOutcome {
        let exe = fake_agent();
        let folder = folder();
        ask_question(
            &QuestionRequest {
                exe: &exe,
                folder: &folder,
                question,
                use_max_turns,
                time_limit: limit,
            },
            cancel,
        )
    }

    fn ask(question: &str) -> AskOutcome {
        ask_with(
            question,
            true,
            Duration::from_secs(30),
            &CancelFlag::default(),
        )
    }

    fn echoed(outcome: AskOutcome) -> serde_json::Value {
        match outcome {
            AskOutcome::Answer(json) => serde_json::from_str(&json).expect("echo is JSON"),
            other => panic!("expected an answer, got {other:?}"),
        }
    }

    fn pid_file(name: &str) -> PathBuf {
        let file = std::env::temp_dir().join(format!(
            "localflow-fake-agent-{name}-{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&file);
        file
    }

    /// The fake's own id and its child's, once it has written them.
    fn read_pids(file: &Path) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(file) {
                let pids: Vec<u32> = text
                    .split_whitespace()
                    .filter_map(|p| p.parse().ok())
                    .collect();
                if pids.len() == 2 {
                    return pids;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the fake agent never wrote its process ids");
    }

    fn assert_all_gone(pids: &[u32]) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while pids.iter().any(|&pid| is_process_running(pid)) {
            assert!(Instant::now() < deadline, "still running: {pids:?}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn a_successful_answer_comes_back() {
        assert_eq!(
            ask("FAKE:answer The overlay draws the waveform."),
            AskOutcome::Answer("The overlay draws the waveform.".into())
        );
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn the_question_goes_on_stdin_and_the_arguments_are_exactly_the_read_only_set() {
        let question = "FAKE:echo — naïve \"quoted\" ; & | %PATH% \n second line ✓";
        let echo = echoed(ask(question));
        let args: Vec<String> = echo["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap().to_string())
            .collect();
        let expected: Vec<String> = question_args(true)
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args, expected,
            "each argument arrives intact, the JSON one included"
        );
        assert_eq!(echo["stdin"], question, "UTF-8 on stdin, unchanged");
        assert_eq!(echo["cwd"], folder().to_string_lossy().as_ref());
        assert!(!args.iter().any(|arg| arg.contains("naïve")));
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn errors_logins_junk_and_silent_exits_are_told_apart() {
        assert_eq!(
            ask("FAKE:error error_during_execution"),
            AskOutcome::AgentError(AgentErrorKind::DuringExecution)
        );
        assert_eq!(
            ask("FAKE:error error_max_turns"),
            AskOutcome::AgentError(AgentErrorKind::MaxTurns)
        );
        assert_eq!(
            ask("FAKE:login"),
            AskOutcome::AgentError(AgentErrorKind::NotSignedIn)
        );
        assert_eq!(ask("FAKE:junk"), AskOutcome::Unreadable);
        assert_eq!(ask("FAKE:exit 2"), AskOutcome::Unreadable);
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn an_unknown_max_turns_is_retried_without_it() {
        let echo = echoed(ask("FAKE:reject-max-turns"));
        let args = echo["args"].as_array().unwrap();
        assert!(!args.iter().any(|arg| arg == "--max-turns"));
        assert!(args.iter().any(|arg| arg == "--permission-mode"));
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn a_time_out_kills_the_whole_tree() {
        let file = pid_file("timeout");
        let started = Instant::now();
        let exe = fake_agent();
        let folder = folder();
        let question = format!("FAKE:tree {}", file.display());
        let handle = {
            let (exe, folder, question) = (exe.clone(), folder.clone(), question.clone());
            std::thread::spawn(move || {
                ask_question(
                    &QuestionRequest {
                        exe: &exe,
                        folder: &folder,
                        question: &question,
                        use_max_turns: true,
                        time_limit: Duration::from_secs(3),
                    },
                    &CancelFlag::default(),
                )
            })
        };
        let pids = read_pids(&file);
        assert!(pids.iter().all(|&pid| is_process_running(pid)));
        assert_eq!(handle.join().unwrap(), AskOutcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(15));
        assert_all_gone(&pids);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn cancelling_kills_the_whole_tree_promptly() {
        let file = pid_file("cancel");
        let cancel = CancelFlag::default();
        let question = format!("FAKE:tree {}", file.display());
        let handle = {
            let cancel = cancel.clone();
            std::thread::spawn(move || ask_with(&question, true, Duration::from_secs(60), &cancel))
        };
        let pids = read_pids(&file);
        let cancelled_at = Instant::now();
        cancel.cancel();
        assert_eq!(handle.join().unwrap(), AskOutcome::Cancelled);
        assert!(cancelled_at.elapsed() < Duration::from_secs(5));
        assert_all_gone(&pids);
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    #[ignore = "needs: cargo build --release --example fake_agent"]
    fn a_slow_answer_inside_the_limit_still_arrives() {
        assert_eq!(
            ask_with(
                "FAKE:sleep 1",
                true,
                Duration::from_secs(20),
                &CancelFlag::default()
            ),
            AskOutcome::Answer("slept".into())
        );
    }
}
