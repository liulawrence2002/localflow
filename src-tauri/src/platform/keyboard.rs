//! The physical state of the modifier keys.
//!
//! The dictation hotkey is `Ctrl+Alt+Space`. The global-shortcut plugin reports it released
//! when Space comes up, so `Ctrl` and `Alt` can still be held when the transcript is ready.
//! A synthetic `Ctrl+V` sent then arrives as `Ctrl+Alt+V`, which pastes nothing or runs some
//! other command. The paste waits here until the modifiers are physically up.

use std::time::{Duration, Instant};

/// How often the key state is read while waiting.
const POLL: Duration = Duration::from_millis(10);

/// Whether `Ctrl`, `Alt`, `Shift` or either Windows key is physically down right now.
#[cfg(windows)]
pub fn modifiers_held() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        // The high bit of the result is set while the key is down.
        .any(|key| unsafe { GetAsyncKeyState(i32::from(key.0)) } as u16 & 0x8000 != 0)
}

#[cfg(not(windows))]
pub fn modifiers_held() -> bool {
    false
}

/// Wait until no modifier key is held, for at most `timeout`. Returns whether they are up.
pub fn wait_for_modifiers_released(timeout: Duration) -> bool {
    wait_until(|| !modifiers_held(), timeout, POLL)
}

/// Poll `ready` until it returns true or `timeout` has passed. Returns the last answer, so a
/// condition that becomes true on the final poll still counts.
pub(crate) fn wait_until(
    mut ready: impl FnMut() -> bool,
    timeout: Duration,
    poll: Duration,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if ready() {
            return true;
        }
        let now = Instant::now();
        if now >= deadline {
            return false;
        }
        std::thread::sleep(poll.min(deadline - now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_at_once_when_the_keys_are_already_up() {
        let mut polls = 0;
        let started = Instant::now();

        assert!(wait_until(
            || {
                polls += 1;
                true
            },
            Duration::from_secs(5),
            Duration::from_millis(10),
        ));

        assert_eq!(polls, 1);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn waits_for_the_keys_to_come_up() {
        let mut polls = 0;

        assert!(wait_until(
            || {
                polls += 1;
                polls >= 4
            },
            Duration::from_secs(5),
            Duration::from_millis(1),
        ));

        assert_eq!(polls, 4);
    }

    #[test]
    fn gives_up_after_the_timeout_when_a_key_stays_held() {
        let started = Instant::now();

        assert!(!wait_until(
            || false,
            Duration::from_millis(60),
            Duration::from_millis(5),
        ));

        let waited = started.elapsed();
        assert!(waited >= Duration::from_millis(60), "{waited:?}");
        assert!(waited < Duration::from_secs(2), "{waited:?}");
    }

    #[test]
    fn reading_the_key_state_does_not_fail() {
        // Only checks the call works; the answer depends on the keyboard.
        let _ = modifiers_held();
    }
}
