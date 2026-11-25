//! Facts about the session and about the process behind a window, for the paste.

/// Mandatory integrity levels (the last sub-authority of the integrity SID).
pub const MEDIUM_INTEGRITY: u32 = 0x2000;
pub const HIGH_INTEGRITY: u32 = 0x3000;

/// Whether Windows will drop synthetic input sent from a process at `own` integrity to a
/// window owned by a process at `target` integrity (User Interface Privilege Isolation:
/// input never flows up to a higher level). Unknown levels are not treated as blocked, so a
/// level that cannot be read leaves the paste as it was before this check existed.
pub fn input_blocked(own: Option<u32>, target: Option<u32>) -> bool {
    matches!((own, target), (Some(own), Some(target)) if target > own)
}

/// Whether synthetic keystrokes from LocalFlow would be dropped by the window of `pid`,
/// typically because that program runs as administrator and LocalFlow does not.
pub fn input_blocked_for_process(pid: u32) -> bool {
    input_blocked(current_integrity_level(), process_integrity_level(pid))
}

/// Whether LocalFlow runs inside a Remote Desktop session, where everything a paste involves
/// is slower.
#[cfg(windows)]
pub fn is_remote_session() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};

    unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 }
}

#[cfg(not(windows))]
pub fn is_remote_session() -> bool {
    false
}

/// The integrity level of this process.
#[cfg(windows)]
pub fn current_integrity_level() -> Option<u32> {
    use windows::Win32::System::Threading::GetCurrentProcess;

    // The pseudo-handle needs no closing.
    unsafe { token_integrity_level(GetCurrentProcess()) }
}

/// The integrity level of the process `pid`, if it can be read.
#[cfg(windows)]
pub fn process_integrity_level(pid: u32) -> Option<u32> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };

    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let level = token_integrity_level(process);
        let _ = CloseHandle(process);
        level
    }
}

#[cfg(windows)]
unsafe fn token_integrity_level(process: windows::Win32::Foundation::HANDLE) -> Option<u32> {
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        Security::{
            GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
            TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
        },
        System::Threading::OpenProcessToken,
    };

    let mut token = HANDLE::default();
    OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;

    // TOKEN_MANDATORY_LABEL is followed by the SID it points to; 64 bytes is plenty for it.
    let mut buffer = [0usize; 16];
    let mut written = 0u32;
    let read = GetTokenInformation(
        token,
        TokenIntegrityLevel,
        Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
        std::mem::size_of_val(&buffer) as u32,
        &mut written,
    );
    let _ = CloseHandle(token);
    read.ok()?;

    let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
    let sid = label.Label.Sid;
    if sid.0.is_null() {
        return None;
    }
    let count = *GetSidSubAuthorityCount(sid);
    if count == 0 {
        return None;
    }
    Some(*GetSidSubAuthority(sid, u32::from(count) - 1))
}

#[cfg(not(windows))]
pub fn current_integrity_level() -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub fn process_integrity_level(_pid: u32) -> Option<u32> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_to_a_higher_integrity_window_is_blocked() {
        assert!(input_blocked(Some(MEDIUM_INTEGRITY), Some(HIGH_INTEGRITY)));
    }

    #[test]
    fn input_to_the_same_or_a_lower_level_is_allowed() {
        assert!(!input_blocked(
            Some(MEDIUM_INTEGRITY),
            Some(MEDIUM_INTEGRITY)
        ));
        assert!(!input_blocked(Some(HIGH_INTEGRITY), Some(MEDIUM_INTEGRITY)));
    }

    #[test]
    fn unknown_levels_are_not_treated_as_blocked() {
        assert!(!input_blocked(None, Some(HIGH_INTEGRITY)));
        assert!(!input_blocked(Some(MEDIUM_INTEGRITY), None));
        assert!(!input_blocked(None, None));
    }

    #[cfg(windows)]
    #[test]
    fn this_process_has_a_readable_integrity_level() {
        let own = current_integrity_level().expect("own integrity level");
        assert!(own >= MEDIUM_INTEGRITY, "{own:#x}");
        assert_eq!(process_integrity_level(std::process::id()), Some(own));
        assert!(!input_blocked_for_process(std::process::id()));
    }

    #[test]
    fn reading_the_session_type_does_not_fail() {
        let _ = is_remote_session();
    }
}
