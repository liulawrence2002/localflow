//! Text dictation shows the user (the overlay, the health file, diagnostics). None of it may
//! contain dictated text, clipboard contents, or a path that names the Windows user.

use std::sync::LazyLock;

use regex::Regex;

pub(crate) const LISTENING: &str = "Listening";
/// Neutral wording: the engine may be Parakeet, the Whisper server or the Whisper CLI.
pub(crate) const TRANSCRIBING: &str = "Transcribing";
pub(crate) const INSERTED: &str = "Inserted transcript";
pub(crate) const CANCELLED: &str = "Cancelled";
pub(crate) const NOTHING_RECOGNISED: &str = "No speech was recognised.";
pub(crate) const PREVIOUS_NOT_PASTED: &str =
    "Previous dictation was not pasted. Use Copy last transcript.";
pub(crate) const FOCUS_CHANGED: &str = "Focus changed before insertion. The transcript was not pasted; use \"Copy last transcript\" to recover it.";
pub(crate) const ELEVATED_TARGET: &str =
    "Can't paste into an administrator window — use Copy last transcript.";
pub(crate) const KEYS_STILL_HELD: &str =
    "Keys were still held, so the transcript was not pasted. Use Copy last transcript.";
pub(crate) const INTERNAL_ERROR: &str = "Dictation hit an internal error and was reset. Try again.";
pub(crate) const ENGINE_UNAVAILABLE: &str =
    "The speech engine could not be started. See Diagnostics for details.";

pub(crate) fn paste_failed(reason: &str) -> String {
    format!("Paste failed ({reason}). Use Copy last transcript.")
}

/// No speech model was found in any of the `locations` looked in.
pub(crate) fn missing_speech_model(locations: usize) -> String {
    format!(
        "No speech model is installed ({locations} locations checked). Run scripts\\Get-AsrModel.ps1, or reinstall LocalFlow."
    )
}

/// The configured model is a Whisper model but the whisper.cpp program is missing.
pub(crate) fn missing_whisper_cli(locations: usize) -> String {
    format!(
        "The whisper.cpp program (whisper-cli.exe) is missing ({locations} locations checked). Reinstall LocalFlow, or see Runtime assets in docs\\DEVELOPMENT.md."
    )
}

/// `Users\<name>` in any path, whatever the drive or slash.
static USER_SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)([\\/])Users([\\/])[^\\/\s]+").expect("valid pattern"));

/// Remove what would identify the Windows user from a message: the profile folder (given by
/// `profile`, normally `%USERPROFILE%`) and any `Users\<name>` segment.
pub(crate) fn scrub_user_paths(message: &str, profile: Option<&str>) -> String {
    let mut scrubbed = message.to_string();
    if let Some(profile) = profile.map(str::trim).filter(|profile| profile.len() > 3) {
        for variant in [profile.to_string(), profile.replace('\\', "/")] {
            let pattern = format!("(?i){}", regex::escape(&variant));
            if let Ok(regex) = Regex::new(&pattern) {
                scrubbed = regex
                    .replace_all(&scrubbed, regex::NoExpand("%USERPROFILE%"))
                    .into_owned();
            }
        }
    }
    USER_SEGMENT
        .replace_all(&scrubbed, "${1}Users${2}<user>")
        .into_owned()
}

/// [`scrub_user_paths`] with this user's profile folder.
pub(crate) fn scrub(message: &str) -> String {
    scrub_user_paths(message, std::env::var("USERPROFILE").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_profile_folder_is_replaced_in_either_slash_style() {
        let message = r"Missing speech model. Checked: C:\Users\Ada Lovelace\localflow\.localflow-runtime\models\x.gguf; C:/Users/Ada Lovelace/y.bin";

        let scrubbed = scrub_user_paths(message, Some(r"C:\Users\Ada Lovelace"));

        assert!(!scrubbed.contains("Ada"), "{scrubbed}");
        assert!(scrubbed.contains(r"%USERPROFILE%\localflow"), "{scrubbed}");
        assert!(scrubbed.contains("%USERPROFILE%/y.bin"), "{scrubbed}");
    }

    #[test]
    fn any_users_segment_is_hidden_even_without_the_profile() {
        let message = r"Missing: D:\Users\ada\models\a.bin and \\server\Users\bob\b.bin";

        let scrubbed = scrub_user_paths(message, None);

        assert!(
            !scrubbed.contains("ada") && !scrubbed.contains("bob"),
            "{scrubbed}"
        );
        assert!(
            scrubbed.contains(r"D:\Users\<user>\models\a.bin"),
            "{scrubbed}"
        );
    }

    #[test]
    fn messages_without_paths_are_unchanged() {
        for message in [
            PREVIOUS_NOT_PASTED,
            ELEVATED_TARGET,
            FOCUS_CHANGED,
            NOTHING_RECOGNISED,
            "Local transcription failed: Missing speech model (parakeet.gguf)",
        ] {
            assert_eq!(scrub_user_paths(message, Some(r"C:\Users\ada")), message);
        }
    }

    #[test]
    fn engine_messages_name_files_and_scripts_but_no_paths() {
        for message in [missing_speech_model(4), missing_whisper_cli(2)] {
            assert!(
                !message.contains(":\\") && !message.contains(":/"),
                "{message}"
            );
            assert!(message.contains("locations checked"), "{message}");
        }
        assert!(missing_speech_model(4).contains("Get-AsrModel.ps1"));
        assert!(missing_whisper_cli(2).contains("whisper-cli.exe"));
    }

    #[test]
    fn the_status_wording_does_not_name_a_specific_engine() {
        assert!(!TRANSCRIBING.contains("whisper"));
        assert!(!NOTHING_RECOGNISED.to_lowercase().contains("whisper"));
    }
}
