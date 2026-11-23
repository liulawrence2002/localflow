//! Clipboard-based text insertion: put text on the clipboard, send Ctrl+V, restore later.

use std::time::Instant;

use arboard::Clipboard;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PasteTimings {
    pub(crate) set_clipboard_ms: u128,
    pub(crate) send_paste_ms: u128,
    pub(crate) visible_at: Instant,
}

#[derive(Debug)]
pub(crate) struct PasteStart {
    pub(crate) previous_text: Option<String>,
    /// Clipboard sequence number right after the dictated text was placed, if known. A
    /// different number later means something else has been copied since.
    pub(crate) sequence_after_set: Option<u32>,
    pub(crate) timings: PasteTimings,
}

pub(crate) fn paste_text_for_visible_insert(text: &str) -> Result<PasteStart, String> {
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
    let previous_text = clipboard.get_text().ok();
    let set_clipboard_started_at = Instant::now();
    set_transient_clipboard_text(&mut clipboard, text)
        .map_err(|error| format!("Could not set clipboard text: {error}"))?;
    let set_clipboard_ms = set_clipboard_started_at.elapsed().as_millis();
    let sequence_after_set = clipboard_sequence();

    let send_paste_started_at = Instant::now();
    send_ctrl_v()?;
    let send_paste_ms = send_paste_started_at.elapsed().as_millis();
    let visible_at = Instant::now();

    Ok(PasteStart {
        previous_text,
        sequence_after_set,
        timings: PasteTimings {
            set_clipboard_ms,
            send_paste_ms,
            visible_at,
        },
    })
}

/// Put dictated text on the clipboard for the paste, asking Windows to keep it out of
/// clipboard history (Win+V) and out of cloud clipboard sync. The text is only there for the
/// fraction of a second the paste needs.
#[cfg(windows)]
pub(crate) fn set_transient_clipboard_text(
    clipboard: &mut Clipboard,
    text: &str,
) -> Result<(), arboard::Error> {
    use arboard::SetExtWindows;

    clipboard
        .set()
        .exclude_from_history()
        .exclude_from_cloud()
        .text(text.to_string())
}

#[cfg(not(windows))]
pub(crate) fn set_transient_clipboard_text(
    clipboard: &mut Clipboard,
    text: &str,
) -> Result<(), arboard::Error> {
    clipboard.set_text(text.to_string())
}

pub(crate) fn send_ctrl_v() -> Result<(), String> {
    let inputs = [
        key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];

    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(format!(
            "Could only send {sent} of {} key events.",
            inputs.len()
        ))
    }
}

pub(crate) fn key_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// What to do with the clipboard once the target application has read the pasted text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClipboardRestore {
    /// Put the text that was there before the paste back.
    RestorePrevious,
    /// Nothing can be put back (the clipboard was empty or held something that is not text),
    /// so remove the dictated text rather than leave it behind.
    Clear,
    /// Something else was copied after the paste. Leave it alone.
    Leave,
}

/// Decide how to tidy the clipboard after a paste. Pure, so every case is unit-tested.
///
/// The sequence numbers are `None` when the platform cannot report them; the clipboard is
/// then assumed unchanged, which is the behaviour before sequence numbers were checked.
pub(crate) fn plan_clipboard_restore(
    had_previous_text: bool,
    sequence_after_set: Option<u32>,
    sequence_now: Option<u32>,
) -> ClipboardRestore {
    if let (Some(after_set), Some(now)) = (sequence_after_set, sequence_now) {
        if after_set != now {
            return ClipboardRestore::Leave;
        }
    }

    if had_previous_text {
        ClipboardRestore::RestorePrevious
    } else {
        ClipboardRestore::Clear
    }
}

/// Tidy the clipboard after a paste: restore the earlier text, or clear the dictated text
/// when there is nothing to restore, unless the user has copied something else meanwhile.
pub(crate) fn restore_clipboard_after_paste(
    previous_text: Option<String>,
    sequence_after_set: Option<u32>,
) {
    let plan = plan_clipboard_restore(
        previous_text.is_some(),
        sequence_after_set,
        clipboard_sequence(),
    );
    if plan == ClipboardRestore::Leave {
        return;
    }

    let Ok(mut clipboard) = Clipboard::new() else {
        return;
    };
    match (plan, previous_text) {
        (ClipboardRestore::RestorePrevious, Some(previous_text)) => {
            let _ = clipboard.set_text(previous_text);
        }
        _ => {
            let _ = clipboard.clear();
        }
    }
}

/// The system clipboard sequence number, which changes every time the clipboard contents do.
#[cfg(windows)]
fn clipboard_sequence() -> Option<u32> {
    let sequence = unsafe { windows::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    // Zero means the calling process may not read the window station's clipboard.
    (sequence != 0).then_some(sequence)
}

#[cfg(not(windows))]
fn clipboard_sequence() -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_text_is_restored_when_the_clipboard_is_untouched() {
        assert_eq!(
            plan_clipboard_restore(true, Some(7), Some(7)),
            ClipboardRestore::RestorePrevious
        );
    }

    #[test]
    fn dictated_text_is_cleared_when_there_is_nothing_to_restore() {
        // The clipboard was empty or held an image: the dictated text must not stay behind.
        assert_eq!(
            plan_clipboard_restore(false, Some(7), Some(7)),
            ClipboardRestore::Clear
        );
    }

    #[test]
    fn a_newer_copy_by_the_user_is_never_overwritten() {
        assert_eq!(
            plan_clipboard_restore(true, Some(7), Some(8)),
            ClipboardRestore::Leave
        );
        assert_eq!(
            plan_clipboard_restore(false, Some(7), Some(8)),
            ClipboardRestore::Leave
        );
    }

    #[test]
    fn unknown_sequence_numbers_fall_back_to_tidying_up() {
        assert_eq!(
            plan_clipboard_restore(true, None, Some(3)),
            ClipboardRestore::RestorePrevious
        );
        assert_eq!(
            plan_clipboard_restore(false, Some(3), None),
            ClipboardRestore::Clear
        );
    }
}
