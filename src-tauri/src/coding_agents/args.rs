//! Every argument list LocalFlow passes to a coding agent, and the check that none of them
//! ever skips the agent's permission prompts.
//!
//! Programs are started with `Command::new(<full path>)` and one argument per element, never
//! through a shell, so nothing here is parsed by `cmd.exe` or PowerShell. Windows Terminal
//! does parse its own command line, splitting at `;` even inside quotes and expanding
//! `%NAME%`; paths containing those characters (or `"`) are refused, and no user text (no
//! task, no question) is ever put on a terminal command line.

use std::{ffi::OsString, path::Path};

use super::{agent::LaunchFailure, projects::check_path_text};

/// Appended to Claude Code's system prompt for a spoken answer.
pub const ANSWER_STYLE: &str =
    "Answer in at most three short spoken sentences, without markdown or code.";
/// The only tools a background question may use: reading and searching files.
pub const READ_ONLY_TOOLS: &str = "Read,Grep,Glob";
/// Extra settings for a background question: no hooks run.
pub const QUESTION_SETTINGS: &str = r#"{"disableAllHooks":true}"#;
/// Agentic turns allowed for a background question.
pub const MAX_TURNS: &str = "20";

/// Permission modes LocalFlow may pass. Anything else is refused by
/// [`check_no_permission_skipping`].
const ALLOWED_PERMISSION_MODES: &[&str] = &["default", "manual", "plan", "dontask"];
/// Text that must never appear in any argument, whatever its position.
const FORBIDDEN_TEXT: &[&str] = &["dangerously", "bypasspermissions", "acceptedits"];

/// Arguments for `wt.exe`: a new tab in the current window, in `folder`, running Claude Code
/// interactively in its normal permission mode, with no task.
///
/// `--permission-mode` is always passed: the user's own settings can make a more permissive
/// mode the default, and omitting it would start in that mode. `permission_mode` is the
/// normal mode's name as the installed version lists it (`default`, or `manual`).
pub fn terminal_args(
    folder: &Path,
    claude_exe: &Path,
    permission_mode: &str,
) -> Result<Vec<OsString>, LaunchFailure> {
    check_terminal_path(folder)?;
    check_terminal_path(claude_exe)?;
    if !matches!(permission_mode, "default" | "manual") {
        return Err(LaunchFailure::Refused);
    }
    let args: Vec<OsString> = vec![
        "-w".into(),
        "0".into(),
        "new-tab".into(),
        "-d".into(),
        folder.as_os_str().to_owned(),
        claude_exe.as_os_str().to_owned(),
        "--permission-mode".into(),
        permission_mode.into(),
    ];
    check_no_permission_skipping(&args)?;
    Ok(args)
}

/// Arguments for `claude.exe` answering one question in the background. The question itself
/// goes on standard input, never on the command line.
pub fn question_args(include_max_turns: bool) -> Vec<OsString> {
    let mut args: Vec<&str> = vec![
        "-p",
        "--output-format",
        "json",
        "--permission-mode",
        "dontAsk",
        "--tools",
        READ_ONLY_TOOLS,
        "--strict-mcp-config",
        "--setting-sources",
        "user",
        "--settings",
        QUESTION_SETTINGS,
    ];
    if include_max_turns {
        args.extend(["--max-turns", MAX_TURNS]);
    }
    args.extend([
        "--no-session-persistence",
        "--append-system-prompt",
        ANSWER_STYLE,
    ]);
    args.into_iter().map(OsString::from).collect()
}

/// Arguments for `Cursor.exe`: just the folder.
pub fn cursor_args(folder: &Path) -> Result<Vec<OsString>, LaunchFailure> {
    check_terminal_path(folder)?;
    let args = vec![folder.as_os_str().to_owned()];
    check_no_permission_skipping(&args)?;
    Ok(args)
}

fn check_terminal_path(path: &Path) -> Result<(), LaunchFailure> {
    let text = path.to_str().ok_or(LaunchFailure::UnsafePath)?;
    check_path_text(text).map_err(|_| LaunchFailure::UnsafePath)?;
    if !path.is_absolute() {
        return Err(LaunchFailure::UnsafePath);
    }
    Ok(())
}

/// Refuse any argument list that could skip permission checks: the skip flags in any form,
/// a permission mode other than the normal, plan or don't-ask modes, or the names of the
/// permissive modes anywhere at all (inside a `--settings` JSON, for example).
pub fn check_no_permission_skipping(args: &[OsString]) -> Result<(), LaunchFailure> {
    let lowered: Vec<String> = args
        .iter()
        .map(|arg| arg.to_string_lossy().to_ascii_lowercase())
        .collect();
    for (index, arg) in lowered.iter().enumerate() {
        if FORBIDDEN_TEXT.iter().any(|text| arg.contains(text)) {
            return Err(LaunchFailure::Refused);
        }
        let mode = if arg == "--permission-mode" {
            Some(lowered.get(index + 1).map(String::as_str).unwrap_or(""))
        } else {
            arg.strip_prefix("--permission-mode=")
        };
        if let Some(mode) = mode {
            if !ALLOWED_PERMISSION_MODES.contains(&mode) {
                return Err(LaunchFailure::Refused);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const CLAUDE: &str =
        r"C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe";

    fn strings(args: &[OsString]) -> Vec<String> {
        args.iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    /// The assertion every built list must pass, spelled out rather than through the guard.
    fn assert_never_skips_permissions(args: &[OsString]) {
        let args = strings(args);
        for arg in &args {
            for forbidden in [
                "--dangerously-skip-permissions",
                "--allow-dangerously-skip-permissions",
                "bypassPermissions",
                "acceptEdits",
            ] {
                assert!(!arg.contains(forbidden), "{forbidden} in {args:?}");
            }
        }
        for pair in args.windows(2) {
            if pair[0] == "--permission-mode" {
                assert!(
                    !["auto", "bypassPermissions", "acceptEdits"].contains(&pair[1].as_str()),
                    "{args:?}"
                );
            }
        }
    }

    #[test]
    fn the_terminal_tab_runs_claude_in_its_normal_mode_with_nothing_else() {
        let args = terminal_args(
            Path::new(r"C:\Users\me\Documents\My Projects\localflow"),
            Path::new(CLAUDE),
            "default",
        )
        .unwrap();
        assert_eq!(
            strings(&args),
            [
                "-w",
                "0",
                "new-tab",
                "-d",
                r"C:\Users\me\Documents\My Projects\localflow",
                CLAUDE,
                "--permission-mode",
                "default",
            ]
        );
        assert_never_skips_permissions(&args);
        let manual = terminal_args(Path::new(r"C:\work\app"), Path::new(CLAUDE), "manual").unwrap();
        assert_eq!(strings(&manual).last().unwrap(), "manual");
    }

    #[test]
    fn only_the_normal_mode_can_be_used_for_the_terminal() {
        for mode in [
            "auto",
            "bypassPermissions",
            "acceptEdits",
            "plan",
            "dontAsk",
            "",
        ] {
            assert_eq!(
                terminal_args(Path::new(r"C:\work\app"), Path::new(CLAUDE), mode),
                Err(LaunchFailure::Refused),
                "{mode}"
            );
        }
    }

    #[test]
    fn terminal_separator_characters_in_paths_are_refused() {
        for folder in [
            r"C:\work\a;b",
            r"C:\work\%USERPROFILE%",
            r#"C:\work\a"b"#,
            "C:\\work\\a\nb",
            r"work\relative",
        ] {
            assert_eq!(
                terminal_args(Path::new(folder), Path::new(CLAUDE), "default"),
                Err(LaunchFailure::UnsafePath),
                "{folder}"
            );
        }
        assert_eq!(
            terminal_args(
                Path::new(r"C:\work\app"),
                Path::new(r"C:\tools;calc\claude.exe"),
                "default"
            ),
            Err(LaunchFailure::UnsafePath)
        );
    }

    #[test]
    fn the_question_run_is_read_only_and_has_one_settings_argument() {
        for include_max_turns in [true, false] {
            let args = question_args(include_max_turns);
            assert_never_skips_permissions(&args);
            assert!(check_no_permission_skipping(&args).is_ok());
            let args = strings(&args);
            let after = |flag: &str| {
                let at = args.iter().position(|arg| arg == flag).unwrap();
                args[at + 1].clone()
            };
            assert_eq!(args[0], "-p");
            assert_eq!(after("--output-format"), "json");
            assert_eq!(after("--permission-mode"), "dontAsk");
            assert_eq!(after("--tools"), "Read,Grep,Glob");
            assert_eq!(after("--setting-sources"), "user");
            assert_eq!(after("--settings"), r#"{"disableAllHooks":true}"#);
            assert_eq!(after("--append-system-prompt"), ANSWER_STYLE);
            assert!(args.contains(&"--strict-mcp-config".to_string()));
            assert!(args.contains(&"--no-session-persistence".to_string()));
            assert_eq!(args.contains(&"--max-turns".to_string()), include_max_turns);
            if include_max_turns {
                assert_eq!(after("--max-turns"), "20");
            }
            // A variadic `--tools` must be followed by another option, not by a value it
            // could swallow.
            assert!(
                after("--tools").starts_with("Read") && after(READ_ONLY_TOOLS).starts_with("--")
            );
        }
    }

    #[test]
    fn cursor_gets_only_the_folder() {
        let args = cursor_args(Path::new(r"C:\work\localflow")).unwrap();
        assert_eq!(strings(&args), [r"C:\work\localflow"]);
        assert_never_skips_permissions(&args);
        assert_eq!(
            cursor_args(Path::new(r"C:\work\a;b")),
            Err(LaunchFailure::UnsafePath)
        );
    }

    #[test]
    fn the_guard_refuses_every_way_of_skipping_permissions() {
        let refused = |args: &[&str]| {
            let args: Vec<OsString> = args.iter().map(OsString::from).collect();
            check_no_permission_skipping(&args) == Err(LaunchFailure::Refused)
        };
        assert!(refused(&["--dangerously-skip-permissions"]));
        assert!(refused(&["-p", "--DANGEROUSLY-SKIP-PERMISSIONS"]));
        assert!(refused(&["--allow-dangerously-skip-permissions"]));
        assert!(refused(&["--permission-mode", "bypassPermissions"]));
        assert!(refused(&["--permission-mode", "auto"]));
        assert!(refused(&["--permission-mode", "acceptEdits"]));
        assert!(refused(&["--permission-mode=auto"]));
        assert!(refused(&["--permission-mode"]));
        assert!(refused(&[
            "--settings",
            r#"{"permissions":{"defaultMode":"bypassPermissions"}}"#
        ]));
        assert!(refused(&["--settings", r#"{"defaultMode":"acceptEdits"}"#]));
        assert!(!refused(&["--permission-mode", "default"]));
        assert!(!refused(&["--permission-mode", "dontAsk"]));
        assert!(!refused(&[r"C:\work\automation"]));
    }

    #[test]
    fn hostile_paths_never_reach_an_argument_list() {
        let hostile = PathBuf::from("C:\\work\\x\" ; calc.exe ; \"");
        assert!(terminal_args(&hostile, Path::new(CLAUDE), "default").is_err());
        assert!(cursor_args(&hostile).is_err());
    }
}
