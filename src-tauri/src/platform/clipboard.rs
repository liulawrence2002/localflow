//! The clipboard as the paste uses it.
//!
//! - [`DelayedText`] puts the dictated text on the clipboard with *delayed rendering*: the
//!   clipboard announces text, but the text itself is handed over only when a program asks
//!   for it (`WM_RENDERFORMAT`). That request is how the paste learns that the target has
//!   read the text, so the earlier clipboard is put back after the target has its copy rather
//!   than after a guessed delay. Clipboard-history and cloud-sync exclusions are set with it.
//! - [`privacy_markers`] reads the formats password managers add to keep a secret out of
//!   clipboard history, cloud sync and clipboard managers, so such a secret is never put back
//!   on the clipboard by LocalFlow.
//!
//! Nothing here logs clipboard contents.

/// The standard Unicode text format.
pub const CF_UNICODETEXT: u32 = 13;

/// What other programs said about the content on the clipboard before the paste.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PrivacyMarkers {
    /// `ExcludeClipboardContentFromMonitorProcessing` is present.
    pub exclude_from_monitoring: bool,
    /// `Clipboard Viewer Ignore` is present (the older convention clipboard managers honour).
    pub viewer_ignore: bool,
    /// `CanIncludeInClipboardHistory`: `None` when absent, otherwise its DWORD value, or
    /// `Some(u32::MAX)` when present but unreadable.
    pub history: Option<u32>,
    /// `CanUploadToCloudClipboard`, read the same way.
    pub cloud: Option<u32>,
}

impl PrivacyMarkers {
    /// Whether the content was marked as something that must not be kept or shared. A marker
    /// that is present but cannot be read counts as private.
    pub fn is_private(&self) -> bool {
        self.exclude_from_monitoring
            || self.viewer_ignore
            || matches!(self.history, Some(value) if value != 1)
            || matches!(self.cloud, Some(value) if value != 1)
    }
}

/// When a program asked for the delayed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Render {
    pub at: std::time::Instant,
    /// The process that had the clipboard open when it asked, if it opened it with a window.
    pub requester_pid: Option<u32>,
}

/// What finishing a delayed publication did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishOutcome {
    /// The earlier text was put back. `sequence` is the clipboard sequence number after it.
    Restored { sequence: Option<u32> },
    /// The dictated text was removed and nothing was put back.
    Cleared,
    /// Another program had put something on the clipboard since; it was left alone.
    LeftAlone,
    /// The clipboard could not be opened in time; the dictated text may still be there.
    Failed,
}

/// What to put on the clipboard when the paste is finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishRequest {
    Restore(String),
    Clear,
}

#[cfg(windows)]
pub use windows_impl::{clipboard_sequence, privacy_markers, DelayedText};

#[cfg(not(windows))]
pub use fallback::{clipboard_sequence, privacy_markers, DelayedText};

#[cfg(windows)]
mod windows_impl {
    use std::{
        sync::{mpsc, Arc, Condvar, Mutex, OnceLock},
        thread,
        time::{Duration, Instant},
    };

    use windows::{
        core::{w, PCWSTR},
        Win32::{
            Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM},
            System::DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardOwner,
                GetClipboardSequenceNumber, GetOpenClipboardWindow, IsClipboardFormatAvailable,
                OpenClipboard, RegisterClipboardFormatW,
            },
            UI::WindowsAndMessaging::{
                CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
                GetWindowLongPtrW, GetWindowThreadProcessId, PeekMessageW, PostMessageW,
                PostQuitMessage, RegisterClassW, SetWindowLongPtrW, TranslateMessage,
                GWLP_USERDATA, HWND_MESSAGE, MSG, PM_NOREMOVE, WINDOW_EX_STYLE, WINDOW_STYLE,
                WM_APP, WM_DESTROY, WM_NCDESTROY, WM_RENDERALLFORMATS, WM_RENDERFORMAT, WNDCLASSW,
            },
        },
    };

    use super::{FinishOutcome, FinishRequest, PrivacyMarkers, Render, CF_UNICODETEXT};

    // `SetClipboardData` and the global-memory functions are declared here rather than taken
    // from the `windows` crate: its wrapper reports the NULL that a delayed-rendering call
    // returns on success as an error, and the memory functions need a crate feature the app
    // does not otherwise use.
    #[link(name = "user32")]
    extern "system" {
        fn SetClipboardData(format: u32, memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalAlloc(flags: u32, bytes: usize) -> *mut core::ffi::c_void;
        fn GlobalLock(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GlobalUnlock(memory: *mut core::ffi::c_void) -> i32;
        fn GlobalFree(memory: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn GlobalSize(memory: *mut core::ffi::c_void) -> usize;
    }

    extern "C" {
        /// The linker's symbol for the start of this image, used as its instance handle.
        static __ImageBase: u8;
    }

    const GMEM_MOVEABLE: u32 = 0x0002;
    /// Posted to the owner window to put the earlier clipboard back and close.
    const WM_FINISH: u32 = WM_APP + 0x4C46;
    /// How long publishing may take before the paste gives up on delayed rendering.
    const PUBLISH_TIMEOUT: Duration = Duration::from_secs(2);
    /// How long finishing waits for a program that has the clipboard open to close it.
    const OPEN_RETRY_FOR: Duration = Duration::from_secs(1);
    const OPEN_RETRY_STEP: Duration = Duration::from_millis(10);

    /// The clipboard sequence number, which changes every time the contents do. `None` when
    /// this process may not read the window station's clipboard.
    pub fn clipboard_sequence() -> Option<u32> {
        let sequence = unsafe { GetClipboardSequenceNumber() };
        (sequence != 0).then_some(sequence)
    }

    fn registered_format(cell: &OnceLock<u32>, name: PCWSTR) -> u32 {
        *cell.get_or_init(|| unsafe { RegisterClipboardFormatW(name) })
    }

    fn exclude_from_monitoring_format() -> u32 {
        static CELL: OnceLock<u32> = OnceLock::new();
        registered_format(&CELL, w!("ExcludeClipboardContentFromMonitorProcessing"))
    }

    fn viewer_ignore_format() -> u32 {
        static CELL: OnceLock<u32> = OnceLock::new();
        registered_format(&CELL, w!("Clipboard Viewer Ignore"))
    }

    fn history_format() -> u32 {
        static CELL: OnceLock<u32> = OnceLock::new();
        registered_format(&CELL, w!("CanIncludeInClipboardHistory"))
    }

    fn cloud_format() -> u32 {
        static CELL: OnceLock<u32> = OnceLock::new();
        registered_format(&CELL, w!("CanUploadToCloudClipboard"))
    }

    /// Open the clipboard, retrying briefly while another program has it open. While
    /// retrying, messages sent to this thread are handled, so a program waiting for this
    /// thread to render data is never deadlocked against it.
    fn open_clipboard(owner: Option<HWND>, retry_for: Duration) -> bool {
        let deadline = Instant::now() + retry_for;
        loop {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            if owner.is_some() {
                let mut message = MSG::default();
                unsafe {
                    let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                }
            }
            thread::sleep(OPEN_RETRY_STEP);
        }
    }

    /// Copy `bytes` into a new movable global memory block, as the clipboard requires.
    unsafe fn global_copy(bytes: &[u8]) -> Option<*mut core::ffi::c_void> {
        let memory = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1));
        if memory.is_null() {
            return None;
        }
        let target = GlobalLock(memory) as *mut u8;
        if target.is_null() {
            GlobalFree(memory);
            return None;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len());
        GlobalUnlock(memory);
        Some(memory)
    }

    /// Put `bytes` on the open clipboard in `format`. The clipboard owns the memory after a
    /// successful call; on failure it is freed here.
    unsafe fn set_bytes(format: u32, bytes: &[u8]) -> bool {
        let Some(memory) = global_copy(bytes) else {
            return false;
        };
        if SetClipboardData(format, memory).is_null() {
            GlobalFree(memory);
            return false;
        }
        true
    }

    fn utf16_with_nul(text: &str) -> Vec<u8> {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .flat_map(u16::to_ne_bytes)
            .collect()
    }

    /// Mark the content on the open clipboard as excluded from clipboard history and cloud
    /// clipboard sync (a DWORD of zero in each registered format).
    unsafe fn set_exclusions() {
        let zero = 0u32.to_ne_bytes();
        let _ = set_bytes(history_format(), &zero);
        let _ = set_bytes(cloud_format(), &zero);
    }

    /// Read a DWORD marker from the open clipboard: `None` when absent, `Some(u32::MAX)` when
    /// present but unreadable.
    unsafe fn read_dword_marker(format: u32) -> Option<u32> {
        if format == 0 || IsClipboardFormatAvailable(format).is_err() {
            return None;
        }
        let Ok(handle) = GetClipboardData(format) else {
            return Some(u32::MAX);
        };
        let memory = handle.0;
        if GlobalSize(memory) < 4 {
            return Some(u32::MAX);
        }
        let data = GlobalLock(memory) as *const u8;
        if data.is_null() {
            return Some(u32::MAX);
        }
        let mut value = [0u8; 4];
        std::ptr::copy_nonoverlapping(data, value.as_mut_ptr(), 4);
        GlobalUnlock(memory);
        Some(u32::from_ne_bytes(value))
    }

    fn format_present(format: u32) -> bool {
        format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    }

    /// The privacy markers on the current clipboard content. When the clipboard cannot be
    /// opened the content is reported as private, so it is never restored blindly.
    pub fn privacy_markers() -> PrivacyMarkers {
        let exclude_from_monitoring = format_present(exclude_from_monitoring_format());
        let viewer_ignore = format_present(viewer_ignore_format());
        if !open_clipboard(None, Duration::from_millis(50)) {
            return PrivacyMarkers {
                exclude_from_monitoring: true,
                ..PrivacyMarkers::default()
            };
        }
        let markers = unsafe {
            PrivacyMarkers {
                exclude_from_monitoring,
                viewer_ignore,
                history: read_dword_marker(history_format()),
                cloud: read_dword_marker(cloud_format()),
            }
        };
        unsafe {
            let _ = CloseClipboard();
        }
        markers
    }

    #[derive(Default)]
    struct State {
        /// UTF-16 text with its terminator, as bytes; emptied once rendered.
        text: Vec<u8>,
        render: Option<Render>,
        request: Option<FinishRequest>,
        outcome: Option<FinishOutcome>,
    }

    struct Shared {
        state: Mutex<State>,
        changed: Condvar,
    }

    impl Shared {
        fn lock(&self) -> std::sync::MutexGuard<'_, State> {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
        }
    }

    /// Dictated text on the clipboard with delayed rendering, owned by a hidden message-only
    /// window on its own thread. Finish it with [`DelayedText::finish`]; dropping it without
    /// finishing clears the dictated text.
    pub struct DelayedText {
        shared: Arc<Shared>,
        hwnd: isize,
        finished: bool,
    }

    impl DelayedText {
        /// Announce `text` on the clipboard, excluded from history and cloud sync. Returns
        /// once the clipboard has been changed, or an error when it could not be (the caller
        /// then falls back to putting the text there directly).
        pub fn publish(text: &str) -> Result<Self, String> {
            let shared = Arc::new(Shared {
                state: Mutex::new(State {
                    text: utf16_with_nul(text),
                    ..State::default()
                }),
                changed: Condvar::new(),
            });
            let (ready_tx, ready_rx) = mpsc::channel();
            let thread_shared = shared.clone();
            thread::Builder::new()
                .name("localflow-clipboard-owner".to_string())
                .spawn(move || owner_thread(thread_shared, ready_tx))
                .map_err(|error| format!("could not start the clipboard thread: {error}"))?;

            match ready_rx.recv_timeout(PUBLISH_TIMEOUT) {
                Ok(Ok(hwnd)) => Ok(Self {
                    shared,
                    hwnd,
                    finished: false,
                }),
                Ok(Err(error)) => Err(error),
                Err(_) => Err("the clipboard thread did not answer".to_string()),
            }
        }

        /// The first request for the text, if there has been one.
        pub fn render(&self) -> Option<Render> {
            self.shared.lock().render
        }

        /// Wait up to `timeout` for the first request for the text.
        pub fn wait_for_render(&self, timeout: Duration) -> Option<Render> {
            let state = self.shared.lock();
            if state.render.is_some() {
                return state.render;
            }
            let (state, _) = self
                .shared
                .changed
                .wait_timeout_while(state, timeout, |state| state.render.is_none())
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.render
        }

        /// Put the earlier text back (or clear the dictated text) unless another program has
        /// taken the clipboard since, then close the owner window.
        pub fn finish(mut self, request: FinishRequest) -> FinishOutcome {
            self.finished = true;
            finish_owner(&self.shared, self.hwnd, request)
        }
    }

    impl Drop for DelayedText {
        fn drop(&mut self) {
            if !self.finished {
                let _ = finish_owner(&self.shared, self.hwnd, FinishRequest::Clear);
            }
        }
    }

    fn finish_owner(shared: &Shared, hwnd: isize, request: FinishRequest) -> FinishOutcome {
        shared.lock().request = Some(request);
        let posted = unsafe {
            PostMessageW(
                Some(HWND(hwnd as *mut core::ffi::c_void)),
                WM_FINISH,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if posted.is_err() {
            return FinishOutcome::Failed;
        }
        let state = shared.lock();
        let (state, _) = shared
            .changed
            .wait_timeout_while(state, Duration::from_secs(5), |state| {
                state.outcome.is_none()
            })
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.outcome.unwrap_or(FinishOutcome::Failed)
    }

    fn owner_class() -> Option<PCWSTR> {
        static REGISTERED: OnceLock<bool> = OnceLock::new();
        let class = w!("LocalFlowClipboardOwner");
        let registered = *REGISTERED.get_or_init(|| unsafe {
            let window_class = WNDCLASSW {
                lpfnWndProc: Some(owner_proc),
                hInstance: instance(),
                lpszClassName: class,
                ..Default::default()
            };
            RegisterClassW(&window_class) != 0
        });
        registered.then_some(class)
    }

    fn instance() -> HINSTANCE {
        HINSTANCE(unsafe { &__ImageBase as *const u8 as *mut core::ffi::c_void })
    }

    fn owner_thread(shared: Arc<Shared>, ready: mpsc::Sender<Result<isize, String>>) {
        let Some(class) = owner_class() else {
            let _ = ready.send(Err(
                "the clipboard window class could not be registered".into()
            ));
            return;
        };
        let hwnd = match unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR::null(),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance()),
                None,
            )
        } {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = ready.send(Err(format!(
                    "the clipboard window could not be created: {}",
                    error.code()
                )));
                return;
            }
        };
        // The window holds one reference to the shared state until WM_NCDESTROY.
        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Arc::into_raw(shared.clone()) as isize);
        }

        if let Err(error) = publish_delayed(hwnd) {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            let _ = ready.send(Err(error));
            return;
        }
        let _ = ready.send(Ok(hwnd.0 as isize));

        let mut message = MSG::default();
        // GetMessageW returns -1 on error, 0 on WM_QUIT.
        while unsafe { GetMessageW(&mut message, None, 0, 0) }.0 > 0 {
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
    }

    fn publish_delayed(hwnd: HWND) -> Result<(), String> {
        if !open_clipboard(Some(hwnd), Duration::from_millis(250)) {
            return Err("the clipboard is in use by another program".to_string());
        }
        let result = unsafe {
            if EmptyClipboard().is_err() {
                Err("the clipboard could not be emptied".to_string())
            } else {
                // A NULL handle announces the format; the text is supplied on request.
                let _ = SetClipboardData(CF_UNICODETEXT, std::ptr::null_mut());
                set_exclusions();
                Ok(())
            }
        };
        unsafe {
            let _ = CloseClipboard();
        }
        result?;
        // Publishing makes this window the clipboard owner; anything else means it failed.
        if unsafe { GetClipboardOwner() }.ok() == Some(hwnd) {
            Ok(())
        } else {
            Err("another program took the clipboard straight away".to_string())
        }
    }

    unsafe fn shared_of(hwnd: HWND) -> Option<&'static Shared> {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared;
        // The reference lives until WM_NCDESTROY releases it, after every other message.
        pointer.as_ref()
    }

    unsafe extern "system" fn owner_proc(
        hwnd: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match message {
            WM_RENDERFORMAT => {
                if wparam.0 as u32 == CF_UNICODETEXT {
                    if let Some(shared) = shared_of(hwnd) {
                        render_text(shared);
                    }
                }
                LRESULT(0)
            }
            WM_RENDERALLFORMATS => {
                // The window is closing while the text was never asked for. Remove it rather
                // than leave dictated text behind.
                if OpenClipboard(Some(hwnd)).is_ok() {
                    if GetClipboardOwner().ok() == Some(hwnd) {
                        let _ = EmptyClipboard();
                    }
                    let _ = CloseClipboard();
                }
                LRESULT(0)
            }
            WM_FINISH => {
                if let Some(shared) = shared_of(hwnd) {
                    let request = shared.lock().request.take();
                    let outcome = finish_on_owner_thread(hwnd, request);
                    shared.lock().outcome = Some(outcome);
                    shared.changed.notify_all();
                }
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const Shared;
                if !pointer.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let shared = Arc::from_raw(pointer);
                    // Anyone still waiting for an outcome learns that there will be none.
                    {
                        let mut state = shared.lock();
                        if state.outcome.is_none() && state.request.is_some() {
                            state.outcome = Some(FinishOutcome::Failed);
                        }
                    }
                    shared.changed.notify_all();
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }

    /// Hand the text to the program that asked for it. Runs inside `WM_RENDERFORMAT`, where
    /// the clipboard is already open by the requester and must not be opened again.
    unsafe fn render_text(shared: &Shared) {
        let text = std::mem::take(&mut shared.lock().text);
        let requester_pid = GetOpenClipboardWindow().ok().and_then(|window| {
            let mut pid = 0u32;
            GetWindowThreadProcessId(window, Some(&mut pid));
            (pid != 0).then_some(pid)
        });
        if !text.is_empty() {
            let _ = set_bytes(CF_UNICODETEXT, &text);
        }
        let mut state = shared.lock();
        if state.render.is_none() {
            state.render = Some(Render {
                at: Instant::now(),
                requester_pid,
            });
        }
        drop(state);
        shared.changed.notify_all();
    }

    unsafe fn finish_on_owner_thread(hwnd: HWND, request: Option<FinishRequest>) -> FinishOutcome {
        if GetClipboardOwner().ok() != Some(hwnd) {
            return FinishOutcome::LeftAlone;
        }
        if !open_clipboard(Some(hwnd), OPEN_RETRY_FOR) {
            return FinishOutcome::Failed;
        }
        // Checked again with the clipboard open: nothing can change it now.
        let outcome = if GetClipboardOwner().ok() != Some(hwnd) {
            FinishOutcome::LeftAlone
        } else if EmptyClipboard().is_err() {
            FinishOutcome::Failed
        } else {
            match request {
                Some(FinishRequest::Restore(text)) => {
                    if set_bytes(CF_UNICODETEXT, &utf16_with_nul(&text)) {
                        set_exclusions();
                        FinishOutcome::Restored { sequence: None }
                    } else {
                        FinishOutcome::Cleared
                    }
                }
                Some(FinishRequest::Clear) | None => FinishOutcome::Cleared,
            }
        };
        let _ = CloseClipboard();
        match outcome {
            FinishOutcome::Restored { .. } => FinishOutcome::Restored {
                sequence: clipboard_sequence(),
            },
            other => other,
        }
    }
}

#[cfg(not(windows))]
mod fallback {
    use std::time::Duration;

    use super::{FinishOutcome, FinishRequest, PrivacyMarkers, Render};

    pub fn clipboard_sequence() -> Option<u32> {
        None
    }

    pub fn privacy_markers() -> PrivacyMarkers {
        PrivacyMarkers::default()
    }

    pub struct DelayedText;

    impl DelayedText {
        pub fn publish(_text: &str) -> Result<Self, String> {
            Err("not supported on this platform".to_string())
        }

        pub fn render(&self) -> Option<Render> {
            None
        }

        pub fn wait_for_render(&self, _timeout: Duration) -> Option<Render> {
            None
        }

        pub fn finish(self, _request: FinishRequest) -> FinishOutcome {
            FinishOutcome::Failed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_without_markers_is_not_private() {
        assert!(!PrivacyMarkers::default().is_private());
    }

    #[test]
    fn password_manager_markers_make_content_private() {
        let monitoring = PrivacyMarkers {
            exclude_from_monitoring: true,
            ..PrivacyMarkers::default()
        };
        let viewer = PrivacyMarkers {
            viewer_ignore: true,
            ..PrivacyMarkers::default()
        };
        let history = PrivacyMarkers {
            history: Some(0),
            ..PrivacyMarkers::default()
        };
        let cloud = PrivacyMarkers {
            cloud: Some(0),
            ..PrivacyMarkers::default()
        };

        for markers in [monitoring, viewer, history, cloud] {
            assert!(markers.is_private(), "{markers:?}");
        }
    }

    /// Delayed rendering on the real clipboard: publish, read the text from another thread
    /// (which is what makes Windows ask for it), then put the earlier text back; and leave
    /// the clipboard alone once something else has been copied. It replaces the clipboard for
    /// a few milliseconds, so it is opt-in:
    /// `LOCALFLOW_LIVE_CLIPBOARD=1 cargo test --release --lib live_delayed_rendering -- --ignored`.
    /// It skips itself unless the clipboard holds plain, unmarked text, which it restores.
    #[cfg(windows)]
    #[test]
    #[ignore = "replaces the system clipboard briefly; set LOCALFLOW_LIVE_CLIPBOARD=1"]
    fn live_delayed_rendering_round_trip() {
        use std::time::Duration;

        if std::env::var("LOCALFLOW_LIVE_CLIPBOARD").as_deref() != Ok("1") {
            return;
        }
        let read_text = || arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text());
        if privacy_markers().is_private() {
            eprintln!("skipped: the clipboard holds content marked private");
            return;
        }
        let Ok(before) = read_text() else {
            eprintln!("skipped: the clipboard does not hold plain text");
            return;
        };
        let probe = "LocalFlow delayed rendering probe";

        let delayed = DelayedText::publish(probe).expect("publish with delayed rendering");
        let asked_before_read = delayed.render().is_some();
        let read = std::thread::spawn(read_text).join().unwrap();
        let render = delayed.wait_for_render(Duration::from_secs(2));
        let outcome = delayed.finish(FinishRequest::Restore(before.clone()));
        let after = read_text();

        // Something else copied in between: finishing must leave it alone.
        let second = DelayedText::publish(probe).expect("publish again");
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text("copied by someone else"))
            .unwrap();
        let left = second.finish(FinishRequest::Restore(before.clone()));
        let someone_else = read_text();
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.set_text(before.clone()))
            .unwrap();

        assert!(
            !asked_before_read,
            "nothing asks for the text until it is read"
        );
        assert_eq!(read.ok().as_deref(), Some(probe));
        assert!(render.is_some(), "the read is seen");
        assert!(
            matches!(outcome, FinishOutcome::Restored { .. }),
            "{outcome:?}"
        );
        assert_eq!(after.ok().as_deref(), Some(before.as_str()));
        assert_eq!(left, FinishOutcome::LeftAlone);
        assert_eq!(someone_else.ok().as_deref(), Some("copied by someone else"));
    }

    #[test]
    fn an_explicit_permission_is_not_private_but_an_unreadable_marker_is() {
        let allowed = PrivacyMarkers {
            history: Some(1),
            cloud: Some(1),
            ..PrivacyMarkers::default()
        };
        assert!(!allowed.is_private());

        let unreadable = PrivacyMarkers {
            history: Some(u32::MAX),
            ..PrivacyMarkers::default()
        };
        assert!(unreadable.is_private());
    }
}
