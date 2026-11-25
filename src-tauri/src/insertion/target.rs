//! The window focused when dictation started, and the check made before pasting.

/// The focused window captured when dictation started, used to verify the insertion
/// target has not changed before pasting. Handles are stored as `isize` (the raw handle
/// value) so the token is `Send` and can cross to the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetWindow {
    /// The foreground (top-level) window.
    pub(crate) hwnd: isize,
    pub(crate) pid: u32,
    /// The window with the keyboard focus inside it (`GUITHREADINFO::hwndFocus`, 0 when the
    /// thread has none), or `None` when it could not be read.
    pub(crate) focus: Option<isize>,
}

/// Read the current foreground window as an insertion target. Returns `None` if no window
/// is focused or its owning process cannot be determined.
#[cfg(windows)]
pub(crate) fn foreground_target() -> Option<TargetWindow> {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        let thread_id = GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let focus = (thread_id != 0 && GetGUIThreadInfo(thread_id, &mut info).is_ok())
            .then_some(info.hwndFocus.0 as isize);
        Some(TargetWindow {
            hwnd: hwnd.0 as isize,
            pid,
            focus,
        })
    }
}

#[cfg(not(windows))]
pub(crate) fn foreground_target() -> Option<TargetWindow> {
    None
}

/// Whether it is safe to insert into the current foreground window. Fails closed: unless
/// the captured target and the current foreground window are both known and identical,
/// down to the control holding the keyboard focus, the caller must not paste (spec: never
/// insert into a target that cannot be revalidated). A focus that could be read at one end
/// and not the other counts as changed.
pub(crate) fn target_matches(
    captured: Option<TargetWindow>,
    current: Option<TargetWindow>,
) -> bool {
    match (captured, current) {
        (Some(captured), Some(current)) => {
            captured.hwnd == current.hwnd
                && captured.pid == current.pid
                && captured.focus == current.focus
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(hwnd: isize, pid: u32, focus: Option<isize>) -> TargetWindow {
        TargetWindow { hwnd, pid, focus }
    }

    #[test]
    fn target_matches_only_identical_foreground_window() {
        let a = window(0x1234, 42, Some(0x5678));
        let same = window(0x1234, 42, Some(0x5678));
        let other_window = window(0x9999, 42, Some(0x5678));
        let other_process = window(0x1234, 7, Some(0x5678));

        assert!(target_matches(Some(a), Some(same)));
        assert!(!target_matches(Some(a), Some(other_window)));
        assert!(!target_matches(Some(a), Some(other_process)));
    }

    #[test]
    fn a_focus_change_inside_the_same_window_blocks_the_paste() {
        // Same top-level window and process, but the keyboard focus moved to another
        // control (for example from a chat box to a password field in the same app).
        let at_start = window(0x1234, 42, Some(0x5678));
        let moved = window(0x1234, 42, Some(0x9abc));
        let lost = window(0x1234, 42, Some(0));

        assert!(!target_matches(Some(at_start), Some(moved)));
        assert!(!target_matches(Some(at_start), Some(lost)));
    }

    #[test]
    fn an_unreadable_focus_at_one_end_fails_closed() {
        let known = window(0x1234, 42, Some(0x5678));
        let unknown = window(0x1234, 42, None);

        assert!(!target_matches(Some(known), Some(unknown)));
        assert!(!target_matches(Some(unknown), Some(known)));
        // Unreadable at both ends: the window and process alone decide, as before.
        assert!(target_matches(Some(unknown), Some(unknown)));
    }

    #[test]
    fn target_matches_fails_closed_when_target_unknown() {
        let a = window(0x1234, 42, Some(0x5678));

        // Cannot revalidate -> must not paste.
        assert!(!target_matches(None, Some(a)));
        assert!(!target_matches(Some(a), None));
        assert!(!target_matches(None, None));
    }
}
