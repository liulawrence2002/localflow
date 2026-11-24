//! Finding Claude Code, Windows Terminal and Cursor on this computer.
//!
//! Only real executables are used, never the `.cmd` or `.ps1` shims npm creates: a shim
//! would put `cmd.exe` or PowerShell between LocalFlow and the agent, and with them their
//! quoting rules. The search order for Claude Code is a real `claude.exe` on `PATH`, then the
//! executable inside the npm package next to an npm shim, then `~\.local\bin\claude.exe`.
//! Copies under `~\.local\share\claude\versions` are never used: they are the updater's
//! working files.

use std::{
    ffi::OsString,
    fmt,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Serialize;

use super::run::{run_captured, RunEnd, RunLimits};
use crate::cancel::CancelFlag;

/// Where to look.
#[derive(Debug, Clone, Default)]
pub struct SearchPaths {
    pub path: Vec<PathBuf>,
    /// `%APPDATA%`, where npm puts global shims (`npm\claude.cmd`).
    pub app_data: Option<PathBuf>,
    /// `%LOCALAPPDATA%`, for Windows Terminal's alias and Cursor's install.
    pub local_app_data: Option<PathBuf>,
    /// `%USERPROFILE%`, for `.local\bin\claude.exe`.
    pub user_profile: Option<PathBuf>,
}

impl SearchPaths {
    pub fn from_environment() -> Self {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        Self {
            path: std::env::var_os("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
            app_data: var("APPDATA"),
            local_app_data: var("LOCALAPPDATA"),
            user_profile: var("USERPROFILE"),
        }
    }
}

/// Whether a file (or an app execution alias such as `wt.exe`, which is a reparse point that
/// ordinary metadata calls cannot open) exists at `path`.
pub fn file_exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| !metadata.is_dir())
        .unwrap_or(false)
}

/// How Claude Code was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClaudeSource {
    /// `claude.exe` in a `PATH` folder.
    Path,
    /// Inside the npm package, next to an npm shim.
    NpmPackage,
    /// `%USERPROFILE%\.local\bin\claude.exe`.
    LocalBin,
}

fn is_updater_copy(path: &Path) -> bool {
    let text = path
        .to_string_lossy()
        .to_ascii_lowercase()
        .replace('/', "\\");
    text.contains(r"\.local\share\claude\versions")
}

pub fn find_claude(
    paths: &SearchPaths,
    exists: impl Fn(&Path) -> bool,
) -> Option<(PathBuf, ClaudeSource)> {
    for dir in &paths.path {
        let candidate = dir.join("claude.exe");
        if !is_updater_copy(&candidate) && exists(&candidate) {
            return Some((candidate, ClaudeSource::Path));
        }
    }

    let npm_dirs = paths
        .path
        .iter()
        .cloned()
        .chain(paths.app_data.iter().map(|app_data| app_data.join("npm")));
    for dir in npm_dirs {
        let has_shim = ["claude.cmd", "claude.ps1", "claude"]
            .iter()
            .any(|shim| exists(&dir.join(shim)));
        if !has_shim {
            continue;
        }
        let candidate = dir
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin")
            .join("claude.exe");
        if exists(&candidate) {
            return Some((candidate, ClaudeSource::NpmPackage));
        }
    }

    let local_bin = paths
        .user_profile
        .as_ref()?
        .join(".local")
        .join("bin")
        .join("claude.exe");
    exists(&local_bin).then_some((local_bin, ClaudeSource::LocalBin))
}

pub fn find_windows_terminal(
    paths: &SearchPaths,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    paths
        .path
        .iter()
        .map(|dir| dir.join("wt.exe"))
        .chain(
            paths
                .local_app_data
                .iter()
                .map(|local| local.join("Microsoft").join("WindowsApps").join("wt.exe")),
        )
        .find(|candidate| exists(candidate))
}

/// `Cursor.exe` itself, not the `cursor.cmd` shim.
pub fn find_cursor(paths: &SearchPaths, exists: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if let Some(local) = &paths.local_app_data {
        let installed = local.join("Programs").join("cursor").join("Cursor.exe");
        if exists(&installed) {
            return Some(installed);
        }
    }
    // The shim lives in `<install>\resources\app\bin`; the editor is three folders up.
    for dir in &paths.path {
        if !exists(&dir.join("cursor.cmd")) {
            continue;
        }
        if let Some(install) = dir.parent().and_then(Path::parent).and_then(Path::parent) {
            let candidate = install.join("Cursor.exe");
            if exists(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// `claude --version` prints "2.1.170 (Claude Code)".
pub fn parse_version(output: &str) -> Option<Version> {
    output.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches('v');
        let mut parts = token.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch: String = parts
            .next()?
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        Some(Version::new(major, minor, patch.parse().ok()?))
    })
}

/// `--max-turns` is hidden from `--help` but has long existed; it is passed from this
/// version on, and dropped again if the program reports it as unknown.
pub const MAX_TURNS_SINCE: Version = Version::new(1, 0, 0);

/// What `claude --help` says is available.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HelpProbe {
    pub options: Vec<String>,
    /// The choices listed for `--permission-mode`, if any are.
    pub permission_modes: Vec<String>,
}

/// Options a background question relies on.
pub const QUESTION_OPTIONS: &[&str] = &[
    "--print",
    "--output-format",
    "--permission-mode",
    "--tools",
    "--strict-mcp-config",
    "--setting-sources",
    "--settings",
    "--no-session-persistence",
    "--append-system-prompt",
];

impl HelpProbe {
    pub fn has(&self, option: &str) -> bool {
        self.options.iter().any(|known| known == option)
    }

    /// The name of the normal, ask-before-acting permission mode in this version: `default`,
    /// or `manual` in versions that renamed it. When no choices are listed, `default`.
    /// `None` when the option itself is missing: then a visible launch is refused, because
    /// without the option the user's default mode (which may skip prompts) would apply.
    pub fn normal_permission_mode(&self) -> Option<&'static str> {
        if !self.has("--permission-mode") {
            return None;
        }
        let listed = |mode: &str| self.permission_modes.iter().any(|m| m == mode);
        if self.permission_modes.is_empty() || listed("default") {
            Some("default")
        } else if listed("manual") {
            Some("manual")
        } else {
            None
        }
    }

    /// Options a background question needs that this version lacks.
    pub fn missing_for_questions(&self) -> Vec<&'static str> {
        let mut missing: Vec<&'static str> = QUESTION_OPTIONS
            .iter()
            .copied()
            .filter(|option| !self.has(option))
            .collect();
        if !self.permission_modes.is_empty()
            && !self.permission_modes.iter().any(|m| m == "dontAsk")
        {
            missing.push("--permission-mode dontAsk");
        }
        missing
    }
}

/// Read the option names and the permission-mode choices from `--help` output.
pub fn parse_help(output: &str) -> HelpProbe {
    let mut probe = HelpProbe::default();
    let lines: Vec<&str> = output.lines().collect();
    let mut permission_block = String::new();
    let mut in_permission_block = false;
    for line in &lines {
        let trimmed = line.trim_start();
        let starts_option = trimmed.starts_with('-');
        if starts_option {
            in_permission_block = false;
            // "-p, --print", "--allowedTools, --allowed-tools <tools...>"
            let names = trimmed.split("  ").next().unwrap_or(trimmed);
            for token in names.split([',', ' ']) {
                if token.starts_with("--") && token.len() > 2 {
                    let name = token.split(['<', '[', '=']).next().unwrap_or(token);
                    if !probe.options.iter().any(|known| known == name) {
                        probe.options.push(name.to_string());
                    }
                }
            }
            if names
                .split([',', ' '])
                .any(|token| token == "--permission-mode")
            {
                in_permission_block = true;
            }
        } else if trimmed.is_empty() || !line.starts_with(' ') {
            in_permission_block = false;
        }
        if in_permission_block {
            permission_block.push(' ');
            permission_block.push_str(trimmed);
        }
    }
    if let Some(choices) = permission_block.split("choices:").nth(1) {
        let choices = choices.split(')').next().unwrap_or("");
        probe.permission_modes = choices
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect();
    }
    probe
}

/// What was learned by running the installed Claude Code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeProbe {
    pub version: Option<Version>,
    pub help: HelpProbe,
}

const PROBE_LIMITS: RunLimits = RunLimits {
    time_limit: Duration::from_secs(20),
    max_stdout: 256 * 1024,
    max_stderr: 64 * 1024,
};

/// Run `claude.exe --version` and `--help`, and nothing else.
pub fn probe_claude(exe: &Path) -> ClaudeProbe {
    let run = |flag: &str| -> String {
        let args = [OsString::from(flag)];
        match run_captured(exe, &args, None, None, PROBE_LIMITS, &CancelFlag::default()) {
            Ok(result) if result.end == RunEnd::Exited(Some(0)) => result.stdout,
            _ => String::new(),
        }
    };
    ClaudeProbe {
        version: parse_version(&run("--version")),
        help: parse_help(&run("--help")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELP_2_1_170: &str = include_str!("testdata/claude_help_2_1_170.txt");

    fn exists_in(files: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |path: &Path| {
            let path = path.to_string_lossy().to_ascii_lowercase();
            files.iter().any(|file| file.to_ascii_lowercase() == path)
        }
    }

    fn paths(path: &[&str]) -> SearchPaths {
        SearchPaths {
            path: path.iter().map(PathBuf::from).collect(),
            app_data: Some(PathBuf::from(r"C:\Users\me\AppData\Roaming")),
            local_app_data: Some(PathBuf::from(r"C:\Users\me\AppData\Local")),
            user_profile: Some(PathBuf::from(r"C:\Users\me")),
        }
    }

    const NPM_EXE: &str =
        r"C:\Users\me\AppData\Roaming\npm\node_modules\@anthropic-ai\claude-code\bin\claude.exe";

    #[test]
    fn a_real_claude_exe_on_path_comes_first() {
        let found = find_claude(
            &paths(&[r"C:\tools", r"C:\Users\me\AppData\Roaming\npm"]),
            exists_in(&[
                r"C:\tools\claude.exe",
                r"C:\Users\me\AppData\Roaming\npm\claude.cmd",
                NPM_EXE,
            ]),
        );
        assert_eq!(
            found,
            Some((PathBuf::from(r"C:\tools\claude.exe"), ClaudeSource::Path))
        );
    }

    #[test]
    fn the_npm_shim_is_resolved_to_the_package_executable() {
        let found = find_claude(
            &paths(&[r"C:\Users\me\AppData\Roaming\npm"]),
            exists_in(&[
                r"C:\Users\me\AppData\Roaming\npm\claude.cmd",
                r"C:\Users\me\AppData\Roaming\npm\claude.ps1",
                NPM_EXE,
            ]),
        );
        assert_eq!(
            found,
            Some((PathBuf::from(NPM_EXE), ClaudeSource::NpmPackage))
        );
    }

    #[test]
    fn the_npm_folder_is_checked_even_when_it_is_not_on_path() {
        let found = find_claude(
            &paths(&[]),
            exists_in(&[r"C:\Users\me\AppData\Roaming\npm\claude", NPM_EXE]),
        );
        assert_eq!(
            found,
            Some((PathBuf::from(NPM_EXE), ClaudeSource::NpmPackage))
        );
    }

    #[test]
    fn a_shim_without_the_package_executable_is_not_used() {
        let found = find_claude(
            &paths(&[r"C:\Users\me\AppData\Roaming\npm"]),
            exists_in(&[r"C:\Users\me\AppData\Roaming\npm\claude.cmd"]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn the_local_bin_copy_is_the_last_resort() {
        let found = find_claude(
            &paths(&[]),
            exists_in(&[r"C:\Users\me\.local\bin\claude.exe"]),
        );
        assert_eq!(
            found,
            Some((
                PathBuf::from(r"C:\Users\me\.local\bin\claude.exe"),
                ClaudeSource::LocalBin
            ))
        );
    }

    #[test]
    fn updater_copies_are_never_used() {
        let versions = r"C:\Users\me\.local\share\claude\versions\2.1.170";
        let found = find_claude(
            &paths(&[versions]),
            exists_in(&[r"C:\Users\me\.local\share\claude\versions\2.1.170\claude.exe"]),
        );
        assert_eq!(found, None);
    }

    #[test]
    fn windows_terminal_and_cursor_are_found() {
        let p = paths(&[r"C:\Users\me\AppData\Local\Programs\cursor\resources\app\bin"]);
        assert_eq!(
            find_windows_terminal(
                &p,
                exists_in(&[r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\wt.exe"])
            ),
            Some(PathBuf::from(
                r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\wt.exe"
            ))
        );
        assert_eq!(find_windows_terminal(&p, exists_in(&[])), None);
        assert_eq!(
            find_cursor(
                &p,
                exists_in(&[r"C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe"])
            ),
            Some(PathBuf::from(
                r"C:\Users\me\AppData\Local\Programs\cursor\Cursor.exe"
            ))
        );
        // Installed elsewhere: found from the shim's folder, never the shim itself.
        let elsewhere = SearchPaths {
            local_app_data: None,
            ..paths(&[r"D:\Apps\Cursor\resources\app\bin"])
        };
        assert_eq!(
            find_cursor(
                &elsewhere,
                exists_in(&[
                    r"D:\Apps\Cursor\resources\app\bin\cursor.cmd",
                    r"D:\Apps\Cursor\Cursor.exe"
                ])
            ),
            Some(PathBuf::from(r"D:\Apps\Cursor\Cursor.exe"))
        );
    }

    #[test]
    fn versions_are_parsed() {
        assert_eq!(
            parse_version("2.1.170 (Claude Code)\n"),
            Some(Version::new(2, 1, 170))
        );
        assert_eq!(parse_version("v1.0.3"), Some(Version::new(1, 0, 3)));
        assert_eq!(
            parse_version("Claude Code 2.0.0-beta"),
            Some(Version::new(2, 0, 0))
        );
        assert_eq!(parse_version("garbage"), None);
        assert_eq!(parse_version(""), None);
        assert!(Version::new(2, 1, 170) >= MAX_TURNS_SINCE);
        assert!(Version::new(0, 2, 9) < MAX_TURNS_SINCE);
    }

    #[test]
    fn the_real_help_text_lists_what_is_needed() {
        let help = parse_help(HELP_2_1_170);
        assert!(help.has("--print"));
        assert!(help.has("--allowed-tools"));
        assert!(help.has("--allowedTools"));
        assert!(
            help.has("--dangerously-skip-permissions"),
            "parsed, never used"
        );
        assert!(!help.has("--max-turns"), "hidden in 2.1.170");
        assert_eq!(
            help.permission_modes,
            [
                "acceptEdits",
                "auto",
                "bypassPermissions",
                "default",
                "dontAsk",
                "plan"
            ]
        );
        assert_eq!(help.normal_permission_mode(), Some("default"));
        assert!(
            help.missing_for_questions().is_empty(),
            "{:?}",
            help.missing_for_questions()
        );
    }

    #[test]
    fn a_version_that_renamed_default_to_manual_is_accepted() {
        let help = parse_help(&HELP_2_1_170.replace("\"default\"", "\"manual\""));
        assert_eq!(help.normal_permission_mode(), Some("manual"));
    }

    #[test]
    fn without_the_permission_option_a_visible_launch_is_refused() {
        let help = parse_help("Options:\n  -p, --print   Print\n");
        assert_eq!(help.normal_permission_mode(), None);
        assert!(help.missing_for_questions().contains(&"--tools"));
        let unlisted = parse_help("Options:\n  --permission-mode <mode>   Mode\n");
        assert_eq!(unlisted.normal_permission_mode(), Some("default"));
        let neither = parse_help(&HELP_2_1_170.replace("\"default\"", "\"normal\""));
        assert_eq!(neither.normal_permission_mode(), None);
    }

    #[test]
    fn a_version_without_dont_ask_cannot_answer_questions() {
        let help = parse_help(&HELP_2_1_170.replace("\"dontAsk\", ", ""));
        assert!(help
            .missing_for_questions()
            .contains(&"--permission-mode dontAsk"));
    }
}
