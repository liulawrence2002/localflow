//! Decides whether a window may be read at all, before anything is taken from it.
//!
//! Every rule fails closed: when the program cannot be identified, or a window looks like it
//! might be private, the answer is no. A refusal carries a fixed reason and nothing from the
//! window.

use super::raw::WindowInfo;
use super::types::DenyReason;

/// What the user has chosen on top of the built-in rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CapturePolicy {
    /// Programs the user never wants read, by file name (`notes.exe` or `notes`, any case).
    pub extra_denied_programs: Vec<String>,
    /// Read private browsing windows too. Off unless the user turns it on.
    pub allow_private_windows: bool,
}

/// LocalFlow's own program name, without the extension.
const OWN_PROGRAM: &str = "localflow";

/// Programs that hold secrets, matched when the program name starts with the entry. Prefix
/// rather than exact, because these ship helpers and variants (`keepassxc`, `1password-cli`,
/// `bitwarden-desktop`, `keeperpasswordmanager`).
const SENSITIVE_PREFIXES: &[&str] = &[
    "1password",
    "bitwarden",
    "keepass",
    "keeweb",
    "lastpass",
    "dashlane",
    "enpass",
    "nordpass",
    "protonpass",
    "proton pass",
    "authy",
    "keeper",
    "roboform",
    "robotaskbaricon",
    "passwordsafe",
    "pwsafe",
    "winauth",
    "stickypassword",
    // Windows' own credential and elevation prompts.
    "credentialuibroker",
    "consent",
];

/// Browsers and the words their private windows put in the title bar, in lower case.
const PRIVATE_TITLE_MARKS: &[(&[&str], &[&str])] = &[
    (&["msedge"], &["inprivate"]),
    (&["chrome", "chromium"], &["incognito"]),
    (
        &["firefox", "librewolf", "waterfox", "zen"],
        &["private browsing", "private window"],
    ),
    (&["brave", "opera", "vivaldi"], &["private", "incognito"]),
];

/// Whether `window` may be read.
///
/// The reason reported is the one that would still hold after the user fixed the others:
/// first whether the program can be identified at all, then facts about the program (our own
/// window, a password manager), then about the window (private browsing), and last the
/// passing state of the keyboard focus. Saying "a password field is focused" about a password
/// manager would suggest that moving the focus helps, and it would not.
///
/// Private browsing is detected from the window title, which is a best-effort heuristic and
/// not a guarantee. It knows the English title text of Edge, Chrome, Firefox, Brave, Opera
/// and a few of their relatives. A browser in another display language, a browser not in
/// the list, or a browser that changes its title wording will not be recognised, and Tor
/// Browser cannot be told apart from Firefox at all. In the other direction, an ordinary
/// page whose own title contains the marker word (a page titled "Private equity" in Brave)
/// is refused, because the title does not say which part came from the browser.
pub fn evaluate_capture(
    window: &WindowInfo,
    focused_is_password: bool,
    policy: &CapturePolicy,
) -> Result<(), DenyReason> {
    let program = program_stem(&window.process_name);
    if program.is_empty() {
        return Err(DenyReason::UnknownProgram);
    }
    if program == OWN_PROGRAM {
        return Err(DenyReason::OwnWindow);
    }
    if is_sensitive_program(&program, policy) {
        return Err(DenyReason::SensitiveProgram);
    }
    if !policy.allow_private_windows && is_private_window(&program, &window.title) {
        return Err(DenyReason::PrivateWindow);
    }
    if focused_is_password {
        return Err(DenyReason::PasswordFocused);
    }
    Ok(())
}

/// The program's file name in lower case, without directories or the `.exe` ending, so that
/// `C:\Apps\KeePass.EXE` and `keepass` compare equal.
fn program_stem(process_name: &str) -> String {
    let lower = process_name.trim().to_lowercase();
    let file = lower.rsplit(['\\', '/']).next().unwrap_or("").trim();
    file.strip_suffix(".exe").unwrap_or(file).trim().to_string()
}

fn is_sensitive_program(program: &str, policy: &CapturePolicy) -> bool {
    SENSITIVE_PREFIXES
        .iter()
        .any(|prefix| program.starts_with(prefix))
        || policy
            .extra_denied_programs
            .iter()
            .map(|extra| program_stem(extra))
            .any(|extra| !extra.is_empty() && extra == program)
}

fn is_private_window(program: &str, title: &str) -> bool {
    let title = title.to_lowercase();
    PRIVATE_TITLE_MARKS
        .iter()
        .filter(|(browsers, _)| browsers.contains(&program))
        .any(|(_, marks)| marks.iter().any(|mark| title.contains(mark)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(process_name: &str, title: &str) -> WindowInfo {
        WindowInfo {
            process_name: process_name.to_string(),
            title: title.to_string(),
            ..WindowInfo::default()
        }
    }

    fn check(process_name: &str, title: &str) -> Result<(), DenyReason> {
        evaluate_capture(
            &window(process_name, title),
            false,
            &CapturePolicy::default(),
        )
    }

    #[test]
    fn an_ordinary_window_may_be_read() {
        assert_eq!(
            check("msedge.exe", "Harbour bridge approved - Microsoft Edge"),
            Ok(())
        );
        assert_eq!(check("notepad.exe", "notes.txt - Notepad"), Ok(()));
    }

    #[test]
    fn the_default_policy_denies_nothing_extra_and_keeps_private_windows_closed() {
        let policy = CapturePolicy::default();

        assert!(policy.extra_denied_programs.is_empty());
        assert!(!policy.allow_private_windows);
    }

    #[test]
    fn a_window_without_a_known_program_is_refused() {
        assert_eq!(check("", "Anything"), Err(DenyReason::UnknownProgram));
        assert_eq!(check("   ", "Anything"), Err(DenyReason::UnknownProgram));
        assert_eq!(check(".exe", "Anything"), Err(DenyReason::UnknownProgram));
    }

    #[test]
    fn our_own_window_is_refused_in_any_spelling() {
        assert_eq!(
            check("localflow.exe", "LocalFlow"),
            Err(DenyReason::OwnWindow)
        );
        assert_eq!(
            check("LocalFlow.EXE", "LocalFlow"),
            Err(DenyReason::OwnWindow)
        );
        assert_eq!(
            check("C:\\Program Files\\LocalFlow\\localflow.exe", "Settings"),
            Err(DenyReason::OwnWindow)
        );
    }

    #[test]
    fn a_focused_password_field_is_refused() {
        let result = evaluate_capture(
            &window("msedge.exe", "Sign in - Microsoft Edge"),
            true,
            &CapturePolicy::default(),
        );

        assert_eq!(result, Err(DenyReason::PasswordFocused));
    }

    #[test]
    fn password_managers_and_authenticators_are_refused() {
        for program in [
            "1password.exe",
            "Bitwarden.exe",
            "KeePass.exe",
            "keepassxc.exe",
            "KeePassXC-proxy.exe",
            "lastpass.exe",
            "Dashlane.exe",
            "Enpass.exe",
            "NordPass.exe",
            "ProtonPass.exe",
            "Authy Desktop.exe",
            "keeperpasswordmanager.exe",
            "RoboForm.exe",
        ] {
            assert_eq!(
                check(program, "Vault"),
                Err(DenyReason::SensitiveProgram),
                "{program}"
            );
        }
    }

    #[test]
    fn a_program_that_only_resembles_a_sensitive_name_is_not_refused() {
        assert_eq!(check("housekeeper.exe", "Chores"), Ok(()));
        assert_eq!(check("compass.exe", "Map"), Ok(()));
    }

    #[test]
    fn programs_the_user_added_are_refused_with_or_without_the_extension() {
        let policy = CapturePolicy {
            extra_denied_programs: vec![
                "Signal".to_string(),
                "MyBank.EXE".to_string(),
                " ".to_string(),
            ],
            allow_private_windows: false,
        };
        let check = |program: &str| evaluate_capture(&window(program, "x"), false, &policy);

        assert_eq!(check("signal.exe"), Err(DenyReason::SensitiveProgram));
        assert_eq!(check("mybank.exe"), Err(DenyReason::SensitiveProgram));
        // An added name is matched whole, so a short entry cannot swallow other programs.
        assert_eq!(check("signalstudio.exe"), Ok(()));
        assert_eq!(check("notepad.exe"), Ok(()));
    }

    #[test]
    fn private_browsing_windows_are_refused_for_each_known_browser() {
        for (program, title) in [
            ("msedge.exe", "News - [InPrivate] - Microsoft Edge"),
            ("chrome.exe", "News - Google Chrome (Incognito)"),
            ("firefox.exe", "News — Mozilla Firefox Private Browsing"),
            ("firefox.exe", "News - Mozilla Firefox (Private Window)"),
            ("brave.exe", "News - Brave (Private)"),
            ("opera.exe", "News - Opera (Private)"),
            ("MSEDGE.EXE", "news - [inprivate] - microsoft edge"),
        ] {
            assert_eq!(
                check(program, title),
                Err(DenyReason::PrivateWindow),
                "{program}"
            );
        }
    }

    #[test]
    fn the_private_marker_of_one_browser_means_nothing_in_another_program() {
        assert_eq!(
            check("chrome.exe", "What is InPrivate mode? - Google Chrome"),
            Ok(())
        );
        assert_eq!(check("winword.exe", "Private notes - Word"), Ok(()));
        assert_eq!(
            check("msedge.exe", "Incognito cinema listings - Microsoft Edge"),
            Ok(())
        );
    }

    #[test]
    fn private_windows_may_be_read_when_the_user_allows_them() {
        let policy = CapturePolicy {
            extra_denied_programs: Vec::new(),
            allow_private_windows: true,
        };
        let private = window("chrome.exe", "News - Google Chrome (Incognito)");

        assert_eq!(evaluate_capture(&private, false, &policy), Ok(()));
        // Allowing private windows does not lift the password rule.
        assert_eq!(
            evaluate_capture(&private, true, &policy),
            Err(DenyReason::PasswordFocused)
        );
    }

    #[test]
    fn the_most_lasting_reason_is_the_one_reported() {
        let policy = CapturePolicy::default();
        let refuse = |program: &str, title: &str| {
            evaluate_capture(&window(program, title), true, &policy).unwrap_err()
        };

        assert_eq!(refuse("", "Vault (Incognito)"), DenyReason::UnknownProgram);
        assert_eq!(refuse("localflow.exe", "LocalFlow"), DenyReason::OwnWindow);
        assert_eq!(
            refuse("keepassxc.exe", "Unlock database"),
            DenyReason::SensitiveProgram
        );
        assert_eq!(
            refuse("chrome.exe", "Sign in - Google Chrome (Incognito)"),
            DenyReason::PrivateWindow
        );
        assert_eq!(
            refuse("chrome.exe", "Sign in - Google Chrome"),
            DenyReason::PasswordFocused
        );
    }
}
