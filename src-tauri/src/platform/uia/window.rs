//! Facts about a window that plain Win32 can supply without looking inside it.
//!
//! Nothing here uses UI Automation or COM, sends the window a message, or waits on the
//! owning application, so it is safe from any thread and returns at once even when the
//! application is hung. The capture policy is applied to these facts before any content is
//! fetched.

use std::ffi::c_void;

use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HWND},
        System::Threading::{
            GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{
            GetClassNameW, GetWindowThreadProcessId, InternalGetWindowText, IsWindow,
        },
    },
};

use crate::context::page::raw::WindowInfo;

/// Long enough for any title a window manager shows; longer titles are cut.
const TITLE_CAPACITY: usize = 1024;
/// Window class names are limited to 256 characters by `RegisterClass`.
const CLASS_NAME_CAPACITY: usize = 260;
/// Extended-length paths can reach 32 767 characters.
const IMAGE_PATH_CAPACITY: usize = 32_768;

pub(crate) fn to_hwnd(hwnd: isize) -> HWND {
    HWND(hwnd as *mut c_void)
}

pub(crate) fn exists(hwnd: HWND) -> bool {
    !hwnd.0.is_null() && unsafe { IsWindow(Some(hwnd)) }.as_bool()
}

/// The window's process id, or `None` when the window does not exist.
pub(crate) fn process_id(hwnd: HWND) -> Option<u32> {
    let mut process_id = 0u32;
    let thread_id = unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
    (thread_id != 0 && process_id != 0).then_some(process_id)
}

pub(crate) fn is_own_process(process_id: u32) -> bool {
    process_id == unsafe { GetCurrentProcessId() }
}

/// Describe the window. `None` when it does not exist; `process_name` is empty when the
/// owning process could not be identified. `url` is always empty here.
pub(crate) fn describe(hwnd: HWND) -> Option<WindowInfo> {
    if !exists(hwnd) {
        return None;
    }
    let process_id = process_id(hwnd)?;

    let mut class_name = [0u16; CLASS_NAME_CAPACITY];
    let class_length = unsafe { GetClassNameW(hwnd, &mut class_name) }.max(0) as usize;

    // `GetWindowText` sends WM_GETTEXT to a window of the calling process, which would wait
    // on our own UI thread; `InternalGetWindowText` reads the stored caption and never
    // sends anything.
    let mut title = vec![0u16; TITLE_CAPACITY];
    let title_length = unsafe { InternalGetWindowText(hwnd, &mut title) }.max(0) as usize;

    Some(WindowInfo {
        process_name: process_image_path(process_id)
            .map(|path| exe_file_name(&path))
            .unwrap_or_default(),
        title: String::from_utf16_lossy(&title[..title_length.min(TITLE_CAPACITY)]),
        class_name: String::from_utf16_lossy(&class_name[..class_length.min(CLASS_NAME_CAPACITY)]),
        url: String::new(),
    })
}

fn process_image_path(process_id: u32) -> Option<String> {
    let process =
        unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
    let mut path = vec![0u16; IMAGE_PATH_CAPACITY];
    let mut length = path.len() as u32;
    let queried = unsafe {
        QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(path.as_mut_ptr()),
            &mut length,
        )
    };
    let _ = unsafe { CloseHandle(process) };
    queried.ok()?;
    Some(String::from_utf16_lossy(
        &path[..(length as usize).min(path.len())],
    ))
}

/// `C:\Program Files\App\App.EXE` becomes `app.exe`.
pub(crate) fn exe_file_name(image_path: &str) -> String {
    image_path
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

/// Chromium (Edge, Chrome, Electron applications) and Firefox build a page's accessibility
/// tree only after something asks for it, so the first look at such a window can find no
/// document, or an empty one, where a moment later there is a full page. Only these windows
/// are worth waiting on; an ordinary application without a document will never grow one.
pub(crate) fn builds_content_lazily(class_name: &str) -> bool {
    class_name.starts_with("Chrome_WidgetWin_") || class_name == "MozillaWindowClass"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_program_name_is_the_lower_case_file_name() {
        assert_eq!(
            exe_file_name(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"),
            "msedge.exe"
        );
        assert_eq!(exe_file_name(r"C:\Tools\NotePad.EXE"), "notepad.exe");
        assert_eq!(
            exe_file_name(r"\\?\D:\apps/mixed\Some App.exe"),
            "some app.exe"
        );
        assert_eq!(exe_file_name("plain.exe"), "plain.exe");
        assert_eq!(exe_file_name(""), "");
    }

    #[test]
    fn only_browser_engine_windows_are_waited_on() {
        assert!(builds_content_lazily("Chrome_WidgetWin_1"));
        assert!(builds_content_lazily("Chrome_WidgetWin_0"));
        assert!(builds_content_lazily("MozillaWindowClass"));
        assert!(!builds_content_lazily("Notepad"));
        assert!(!builds_content_lazily("CabinetWClass"));
        assert!(!builds_content_lazily(""));
    }

    #[test]
    fn a_window_that_does_not_exist_has_no_description() {
        assert!(describe(to_hwnd(0)).is_none());
        // Window handles are small even numbers in practice; this one is not a window.
        assert!(describe(to_hwnd(0x7FFF_FFF1)).is_none());
        assert!(!exists(to_hwnd(0)));
    }
}
