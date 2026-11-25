//! Keeps the Windows user name out of what LocalFlow writes down or shows.
//!
//! File paths under `C:\Users\<name>` carry the account name. Anything that goes to the log
//! file, the desktop-health file, diagnostics or a message box passes through
//! [`redact_user_paths`], which replaces the profile folder with `<user folder>` and keeps the
//! rest of the path, so a message still says which file was meant.

use std::{borrow::Cow, sync::OnceLock};

use regex::Regex;

/// What a user's profile folder is replaced with.
pub const USER_FOLDER: &str = "<user folder>";

fn users_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        // A drive, any run of slashes or backslashes (JSON doubles them), `Users`, and the
        // account folder, which may contain spaces. Covers `\\?\C:\Users\name` too, since it
        // contains `C:\Users\name`. A bare profile folder followed by more words takes those
        // words with it: too much is removed rather than too little.
        Regex::new(r#"(?i)\b[a-z]:[\\/]+users[\\/]+[^\\/"'<>|:*?\r\n,;]+"#)
            .expect("the user-folder pattern is valid")
    })
}

/// `text` with every user-profile folder replaced by `<user folder>`.
pub fn redact_user_paths(text: &str) -> Cow<'_, str> {
    redact_with_profile(text, profile_dir().as_deref())
}

/// The current user's profile folder, if it is not under `X:\Users` (a roaming or renamed
/// profile); that one is replaced literally as well.
fn profile_dir() -> Option<String> {
    static PROFILE: OnceLock<Option<String>> = OnceLock::new();
    PROFILE
        .get_or_init(|| {
            std::env::var("USERPROFILE")
                .ok()
                .map(|value| value.trim_end_matches(['\\', '/']).to_string())
                .filter(|value| value.len() > 3)
        })
        .clone()
}

fn redact_with_profile<'a>(text: &'a str, profile: Option<&str>) -> Cow<'a, str> {
    let mut result = users_pattern().replace_all(text, USER_FOLDER);
    if let Some(profile) = profile {
        if !users_pattern().is_match(profile) {
            let lower = result.to_lowercase();
            let needle = profile.to_lowercase();
            // Lower-casing keeps byte offsets only for ASCII; skip the literal pass otherwise.
            if lower.len() == result.len() && lower.contains(&needle) {
                let mut out = String::with_capacity(result.len());
                let mut rest = 0;
                for (start, _) in lower.match_indices(&needle) {
                    out.push_str(&result[rest..start]);
                    out.push_str(USER_FOLDER);
                    rest = start + needle.len();
                }
                out.push_str(&result[rest..]);
                result = Cow::Owned(out);
            }
        }
    }
    result
}

/// The last component of `path`, for messages that must name a file without its folder.
pub fn file_name_only(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| redact_user_paths(&path.to_string_lossy()).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_folders_are_replaced_and_the_rest_of_the_path_is_kept() {
        assert_eq!(
            redact_with_profile(
                r"Missing model at C:\Users\Alice Smith\models\parakeet.gguf",
                None
            ),
            r"Missing model at <user folder>\models\parakeet.gguf"
        );
        assert_eq!(
            redact_with_profile("d:/users/bob/AppData/Roaming/x.json", None),
            "<user folder>/AppData/Roaming/x.json"
        );
        // JSON-escaped and extended-length forms.
        assert_eq!(
            redact_with_profile(r#"{"path":"C:\\Users\\carol\\a.txt"}"#, None),
            r#"{"path":"<user folder>\\a.txt"}"#
        );
        assert_eq!(
            redact_with_profile(r"\\?\C:\Users\dan\x", None),
            r"\\?\<user folder>\x"
        );
        assert!(!redact_with_profile(r"C:\Users\eve", None).contains("eve"));
    }

    #[test]
    fn text_without_a_profile_folder_is_unchanged_and_not_copied() {
        let text = r"C:\Program Files\LocalFlow\localflow.exe failed with code 5";
        assert!(matches!(redact_with_profile(text, None), Cow::Borrowed(_)));
        assert_eq!(
            redact_with_profile("Usersguide.txt", None),
            "Usersguide.txt"
        );
    }

    #[test]
    fn a_profile_outside_users_is_replaced_literally() {
        assert_eq!(
            redact_with_profile(
                r"log at E:\Profiles\frank\x.log",
                Some(r"E:\Profiles\frank")
            ),
            r"log at <user folder>\x.log"
        );
        assert_eq!(
            redact_with_profile(r"E:\PROFILES\FRANK\y", Some(r"E:\Profiles\frank")),
            r"<user folder>\y"
        );
    }

    #[test]
    fn only_the_file_name_is_kept() {
        assert_eq!(
            file_name_only(std::path::Path::new(r"C:\Users\gina\models\voice.gguf")),
            "voice.gguf"
        );
    }
}
