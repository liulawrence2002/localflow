//! The window that had focus when dictation started, and the check made before pasting.

/// The focused window captured when dictation started, used to verify the insertion
/// target has not changed before pasting. `hwnd` is stored as an `isize` (the raw handle
/// value) so the token is `Send` and can cross to the worker thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TargetWindow {
    pub(crate) hwnd: isize,
    pub(crate) pid: u32,
}

/// Read the current foreground window as an insertion target. Returns `None` if no window
/// is focused or its owning process cannot be determined.
#[cfg(windows)]
pub(crate) fn foreground_target() -> Option<TargetWindow> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        Some(TargetWindow {
            hwnd: hwnd.0 as isize,
            pid,
        })
    }
}

#[cfg(not(windows))]
pub(crate) fn foreground_target() -> Option<TargetWindow> {
    None
}

/// Whether it is safe to insert into the current foreground window. Fails closed: unless
/// the captured target and the current foreground window are both known and identical, the
/// caller must not paste (spec: never insert into a target that cannot be revalidated).
pub(crate) fn target_matches(
    captured: Option<TargetWindow>,
    current: Option<TargetWindow>,
) -> bool {
    match (captured, current) {
        (Some(captured), Some(current)) => {
            captured.hwnd == current.hwnd && captured.pid == current.pid
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_matches_only_identical_foreground_window() {
        let a = TargetWindow {
            hwnd: 0x1234,
            pid: 42,
        };
        let same = TargetWindow {
            hwnd: 0x1234,
            pid: 42,
        };
        let other_window = TargetWindow {
            hwnd: 0x9999,
            pid: 42,
        };
        let other_process = TargetWindow {
            hwnd: 0x1234,
            pid: 7,
        };

        assert!(target_matches(Some(a), Some(same)));
        assert!(!target_matches(Some(a), Some(other_window)));
        assert!(!target_matches(Some(a), Some(other_process)));
    }

    #[test]
    fn target_matches_fails_closed_when_target_unknown() {
        let a = TargetWindow {
            hwnd: 0x1234,
            pid: 42,
        };

        // Cannot revalidate -> must not paste.
        assert!(!target_matches(None, Some(a)));
        assert!(!target_matches(Some(a), None));
        assert!(!target_matches(None, None));
    }
}
