//! Reads one window's accessibility tree through Windows UI Automation and hands it back as
//! plain data ([`RawTree`]), so the code that decides what to read aloud never touches COM.
//!
//! - [`window_info`] says which program a window belongs to using plain Win32 only. The
//!   capture policy runs on that before anything inside the window is looked at.
//! - [`UiaReader::snapshot`] reads the window's content: the web page of a browser window,
//!   the window itself for an ordinary application.
//!
//! All UI Automation work happens on one background thread owned by the reader. A call into
//! an application that has stopped answering cannot be cancelled, so the caller waits with a
//! deadline; when it passes, that thread is left to finish on its own and the next read
//! starts a new one.
//!
//! Privacy: the reader records names (visible text and labels) only. It never reads the
//! value of a control, never enters an editable field, a combo box or a password field, and
//! takes no text at all from a password field. Nothing read from a window is logged or put
//! in an error. Nothing is written to disk or sent anywhere.

use std::time::{Duration, Instant};

use crate::context::page::{
    raw::{RawTree, WindowInfo},
    types::PageError,
};

mod automation;
mod map;
mod tree;
mod window;
mod worker;

use automation::Session;
use worker::Worker;

/// Limits on one read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotOptions {
    /// Most nodes to keep. A larger page is cut after this many, in reading order.
    pub max_nodes: u32,
    /// Most levels below the content root to keep.
    pub max_depth: u32,
    /// Longest [`UiaReader::snapshot`] may block.
    pub deadline: Duration,
    /// How long to wait for a browser to put its page into the accessibility tree. Browsers
    /// build that tree only once something asks for it, so the first read of a page finds it
    /// missing or empty for a moment. Never more than half of `deadline`.
    pub document_wait: Duration,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self {
            max_nodes: 20_000,
            max_depth: 80,
            deadline: Duration::from_secs(4),
            document_wait: Duration::from_millis(1_500),
        }
    }
}

/// Which program the window `hwnd` (a raw `HWND` value) belongs to, its title and its window
/// class. `url` is left empty.
///
/// Uses Win32 window and process queries only: nothing inside the window is read, no message
/// is sent to it, and the call does not wait on the application, so it is safe on any thread.
/// `None` when the window does not exist or its program cannot be identified.
pub fn window_info(hwnd: isize) -> Option<WindowInfo> {
    window::describe(window::to_hwnd(hwnd)).filter(|info| !info.process_name.is_empty())
}

pub struct UiaReader {
    worker: Worker<Session>,
}

impl UiaReader {
    /// Cheap: the worker thread is started by the first read.
    pub fn new() -> Self {
        Self {
            worker: Worker::new("uia-reader", Session::open),
        }
    }

    /// Read the window `hwnd` (a raw `HWND` value). Blocks at most `options.deadline`.
    ///
    /// A page over the node or depth limit comes back cut short with `truncated` set.
    ///
    /// Errors: [`PageError::TargetChanged`] when the window does not exist or went away;
    /// [`PageError::NothingReadable`] when it exposes nothing under its root;
    /// [`PageError::Timeout`] when the application did not answer in time, which includes a
    /// browser page not yet in the accessibility tree (a second try usually finds it) and a
    /// page too large to fetch within the deadline.
    ///
    /// A window of this process is not read: the result has its `window` facts and no root.
    pub fn snapshot(&self, hwnd: isize, options: &SnapshotOptions) -> Result<RawTree, PageError> {
        let started = Instant::now();
        let job_options = options.clone();
        self.worker.run(options.deadline, move |session, cancel| {
            session.read_window(hwnd, &job_options, started, cancel)
        })
    }
}

impl Default for UiaReader {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::WindowsAndMessaging::GetShellWindow;

    use super::*;

    #[test]
    fn the_reader_can_be_shared_between_threads() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<UiaReader>();
    }

    #[test]
    fn the_default_limits_are_the_documented_ones() {
        let options = SnapshotOptions::default();

        assert_eq!(options.max_nodes, 20_000);
        assert_eq!(options.max_depth, 80);
        assert_eq!(options.deadline, Duration::from_secs(4));
        assert_eq!(options.document_wait, Duration::from_millis(1_500));
    }

    #[test]
    fn a_window_that_does_not_exist_has_no_info() {
        assert_eq!(window_info(0), None);
        assert_eq!(window_info(0x7FFF_FFF1), None);
    }

    #[test]
    fn reading_a_window_that_does_not_exist_reports_that_it_is_gone() {
        let reader = UiaReader::new();

        assert_eq!(
            reader.snapshot(0, &SnapshotOptions::default()),
            Err(PageError::TargetChanged)
        );
        assert_eq!(
            reader.snapshot(0x7FFF_FFF1, &SnapshotOptions::default()),
            Err(PageError::TargetChanged)
        );
    }

    /// The desktop's own window always exists in an interactive session and belongs to the
    /// shell, so it is a real window whose facts are known in advance.
    #[test]
    fn the_shell_window_is_identified_without_reading_it() {
        let shell = unsafe { GetShellWindow() };
        if shell.0.is_null() {
            // No shell (a service session or a bare CI runner): nothing to check against.
            return;
        }

        let started = Instant::now();
        let info = window_info(shell.0 as isize).expect("the shell window exists");
        let took = started.elapsed();

        assert_eq!(info.process_name, "explorer.exe");
        assert_eq!(info.class_name, "Progman");
        assert!(info.url.is_empty());
        assert!(took < Duration::from_millis(250), "took {took:?}");
    }
}
