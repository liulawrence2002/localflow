pub mod clipboard;
pub mod keyboard;
pub mod process;
pub mod uia;
pub mod window;

pub trait StartupRegistration: Send + Sync {
    fn set_enabled(&self, enabled: bool) -> Result<(), String>;
    fn is_enabled(&self) -> Result<bool, String>;
}

pub struct NoOpStartupRegistration;

impl StartupRegistration for NoOpStartupRegistration {
    fn set_enabled(&self, _enabled: bool) -> Result<(), String> {
        Ok(())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        Ok(false)
    }
}

/// Show `message` in a native message box with an OK button and wait until it is dismissed.
/// For problems LocalFlow cannot show in its own window: a start-up failure, or settings that
/// had to be reset. `error` picks the error icon over the warning one. Elsewhere than Windows
/// the message goes to standard error.
pub fn show_message_box(title: &str, message: &str, error: bool) {
    #[cfg(windows)]
    {
        use windows::{
            core::PCWSTR,
            Win32::UI::WindowsAndMessaging::{
                MessageBoxW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST,
            },
        };
        let wide = |text: &str| -> Vec<u16> { text.encode_utf16().chain(Some(0)).collect() };
        let (title, message) = (wide(title), wide(message));
        let icon = if error { MB_ICONERROR } else { MB_ICONWARNING };
        // SAFETY: both buffers are NUL-terminated UTF-16 and outlive the call.
        unsafe {
            MessageBoxW(
                None,
                PCWSTR(message.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_OK | icon | MB_SETFOREGROUND | MB_TOPMOST,
            );
        }
    }
    #[cfg(not(windows))]
    eprintln!("{title}: {message}");
}
