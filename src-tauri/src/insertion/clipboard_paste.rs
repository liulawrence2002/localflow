//! Clipboard-based text insertion: put the text on the clipboard, send `Ctrl+V`, and put the
//! earlier clipboard back once the target has read the text.
//!
//! The text is published with delayed rendering ([`DelayedText`]), so the clipboard tells us
//! when a program fetches it. The earlier text is restored shortly after the target fetched
//! it, instead of after a fixed delay that a slow target (Remote Desktop, a busy Electron
//! app) could outlast and then paste the *earlier* clipboard. When delayed rendering cannot
//! be set up, the text is placed directly and restored after a delay that is longer in a
//! remote session. Content another program marked as private (a password manager's secret)
//! is never put back: the clipboard is cleared instead. Restored text always carries the
//! history and cloud exclusions, so a restore never adds a clipboard-history entry.

use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use arboard::Clipboard;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};

use crate::platform::clipboard::{
    clipboard_sequence, privacy_markers, DelayedText, FinishOutcome, FinishRequest, Render,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct PasteTimings {
    pub(crate) set_clipboard_ms: u128,
    pub(crate) send_paste_ms: u128,
    pub(crate) visible_at: Instant,
}

/// A paste that was sent. The clipboard still has to be tidied with
/// [`PendingRestore::complete`].
pub(crate) struct PasteStart {
    pub(crate) restore: PendingRestore,
    pub(crate) timings: PasteTimings,
}

/// What was on the clipboard before the paste.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviousClipboard {
    /// Plain text that may be put back.
    Text(String),
    /// Content marked private by the program that copied it. Never read, never put back.
    Private,
    /// Nothing, or something that is not text.
    Other,
}

/// Paste `text` into the focused window of process `target_pid`.
///
/// On error nothing was pasted and the clipboard has already been put back.
pub(crate) fn paste_text(text: &str, target_pid: Option<u32>) -> Result<PasteStart, String> {
    // A restore still waiting from an earlier paste is finished first; otherwise this paste
    // would take that dictated text for "the earlier clipboard".
    hurry_pending_restores(Duration::from_millis(1_500));

    let previous = read_previous_clipboard();
    let set_clipboard_started_at = Instant::now();
    let publication = match DelayedText::publish(text) {
        Ok(delayed) => Publication::Delayed(delayed),
        Err(reason) => {
            tracing::warn!(reason = %reason, "delayed clipboard rendering unavailable; setting the text directly");
            let mut clipboard =
                Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
            if let Err(error) = set_transient_clipboard_text(&mut clipboard, text) {
                // The clipboard may have been emptied before the failure. Only plain text is
                // put back; anything else is left as the failure left it.
                if matches!(previous, PreviousClipboard::Text(_)) {
                    restore_directly(&previous);
                }
                return Err(format!("Could not set clipboard text: {error}"));
            }
            Publication::Direct {
                sequence_after_set: clipboard_sequence(),
            }
        }
    };
    let set_clipboard_ms = set_clipboard_started_at.elapsed().as_millis();

    let send_paste_started_at = Instant::now();
    let sent = send_ctrl_v();
    let send_paste_ms = send_paste_started_at.elapsed().as_millis();
    let restore = PendingRestore::new(previous, publication, target_pid, Instant::now());
    if let Err(error) = sent {
        // Nothing reached the target, so the earlier clipboard goes straight back.
        let _ = restore.complete_now();
        return Err(error);
    }

    Ok(PasteStart {
        restore,
        timings: PasteTimings {
            set_clipboard_ms,
            send_paste_ms,
            visible_at: Instant::now(),
        },
    })
}

/// Read what is on the clipboard now, without reading content marked private.
pub(crate) fn read_previous_clipboard() -> PreviousClipboard {
    let sequence = clipboard_sequence();
    // Text LocalFlow itself put back carries the exclusion markers but is not a secret.
    let ours = sequence.is_some() && sequence == own_text_sequence();
    if !ours && privacy_markers().is_private() {
        return PreviousClipboard::Private;
    }
    match Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
        Ok(text) => PreviousClipboard::Text(text),
        Err(_) => PreviousClipboard::Other,
    }
}

/// Put dictated text on the clipboard for the paste, asking Windows to keep it out of
/// clipboard history (Win+V) and out of cloud clipboard sync.
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

/// Remember that the clipboard now holds text LocalFlow put there that is not private (a
/// restore, or "Copy last transcript"), so the next paste may put it back even though it
/// carries the exclusion markers.
pub(crate) fn remember_own_clipboard_text() {
    if let Some(sequence) = clipboard_sequence() {
        OWN_TEXT_SEQUENCE.store(sequence, Ordering::SeqCst);
    }
}

static OWN_TEXT_SEQUENCE: AtomicU32 = AtomicU32::new(0);

fn own_text_sequence() -> Option<u32> {
    match OWN_TEXT_SEQUENCE.load(Ordering::SeqCst) {
        0 => None,
        sequence => Some(sequence),
    }
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
    /// Nothing may be put back (the clipboard was empty, held something that is not text,
    /// or held content marked private), so remove the dictated text rather than leave it.
    Clear,
    /// Something else was copied after the paste. Leave it alone.
    Leave,
}

/// Decide how to tidy the clipboard after a paste. Pure, so every case is unit-tested.
pub(crate) fn plan_clipboard_restore(
    previous: &PreviousClipboard,
    changed_since_paste: bool,
) -> ClipboardRestore {
    if changed_since_paste {
        return ClipboardRestore::Leave;
    }
    match previous {
        PreviousClipboard::Text(_) => ClipboardRestore::RestorePrevious,
        PreviousClipboard::Private | PreviousClipboard::Other => ClipboardRestore::Clear,
    }
}

/// Whether the clipboard changed between two sequence numbers. Unknown numbers count as
/// unchanged, which is the behaviour before sequence numbers were checked.
pub(crate) fn sequence_changed(after_set: Option<u32>, now: Option<u32>) -> bool {
    matches!((after_set, now), (Some(after_set), Some(now)) if after_set != now)
}

/// How long the restore waits, in a local session or a remote one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RestoreTiming {
    /// After the target fetched the text: time for it to finish reading.
    pub(crate) after_target_read: Duration,
    /// After the paste when some other program fetched the text first, so the target's
    /// own fetch cannot be seen.
    pub(crate) after_other_read: Duration,
    /// After the paste when nothing fetched the text: the target ignored the paste.
    pub(crate) unread_limit: Duration,
    /// After the paste when the text had to be placed directly (no delayed rendering).
    pub(crate) direct_delay: Duration,
}

pub(crate) fn restore_timing(remote_session: bool) -> RestoreTiming {
    if remote_session {
        RestoreTiming {
            after_target_read: Duration::from_millis(600),
            after_other_read: Duration::from_millis(3_000),
            unread_limit: Duration::from_millis(8_000),
            direct_delay: Duration::from_millis(2_000),
        }
    } else {
        RestoreTiming {
            after_target_read: Duration::from_millis(300),
            after_other_read: Duration::from_millis(1_000),
            unread_limit: Duration::from_millis(3_000),
            direct_delay: Duration::from_millis(400),
        }
    }
}

/// When the earlier clipboard may go back for a delayed publication. Never earlier than the
/// fixed delay used before delayed rendering (`direct_delay`), so the change can only make
/// the restore later, never sooner. A fetch by a process that opened the clipboard without a
/// window cannot be attributed and is taken to be the target, which is by far the likeliest
/// reader right after the paste.
pub(crate) fn restore_due(
    sent_at: Instant,
    render: Option<Render>,
    target_pid: Option<u32>,
    timing: &RestoreTiming,
) -> Instant {
    let floor = sent_at + timing.direct_delay;
    let due = match render {
        Some(render) if render.requester_pid.is_none() || render.requester_pid == target_pid => {
            render.at + timing.after_target_read
        }
        Some(render) => {
            (sent_at + timing.after_other_read).max(render.at + timing.after_target_read)
        }
        None => sent_at + timing.unread_limit,
    };
    due.max(floor)
}

/// What tidying the clipboard did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RestoreReport {
    pub(crate) outcome: FinishOutcome,
    /// Time from the paste to the restore.
    pub(crate) waited: Duration,
    /// Whether the target was seen fetching the text (delayed rendering only).
    pub(crate) target_read: bool,
}

enum Publication {
    Delayed(DelayedText),
    Direct { sequence_after_set: Option<u32> },
}

/// The clipboard tidy-up owed by a paste. Dropping it without calling
/// [`PendingRestore::complete`] restores at once.
pub(crate) struct PendingRestore {
    job: Option<RestoreJob>,
    _pending: PendingGuard,
}

struct RestoreJob {
    previous: PreviousClipboard,
    publication: Publication,
    target_pid: Option<u32>,
    sent_at: Instant,
    hurry_generation: u64,
}

impl PendingRestore {
    fn new(
        previous: PreviousClipboard,
        publication: Publication,
        target_pid: Option<u32>,
        sent_at: Instant,
    ) -> Self {
        Self {
            job: Some(RestoreJob {
                previous,
                publication,
                target_pid,
                sent_at,
                hurry_generation: HURRY.load(Ordering::SeqCst),
            }),
            _pending: PendingGuard::new(),
        }
    }

    /// Wait until the target has read the text (or the limits in [`restore_timing`] pass),
    /// then put the earlier clipboard back. Blocks; run it off the paste path.
    pub(crate) fn complete(mut self, remote_session: bool) -> RestoreReport {
        let job = self
            .job
            .take()
            .expect("restore job is present until completed");
        job.run(&restore_timing(remote_session), false)
    }

    /// Put the earlier clipboard back now.
    pub(crate) fn complete_now(mut self) -> RestoreReport {
        let job = self
            .job
            .take()
            .expect("restore job is present until completed");
        job.run(&restore_timing(false), true)
    }
}

impl Drop for PendingRestore {
    fn drop(&mut self) {
        if let Some(job) = self.job.take() {
            let _ = job.run(&restore_timing(false), true);
        }
    }
}

impl RestoreJob {
    fn run(self, timing: &RestoreTiming, immediately: bool) -> RestoreReport {
        const SLICE: Duration = Duration::from_millis(25);
        let RestoreJob {
            previous,
            publication,
            target_pid,
            sent_at,
            hurry_generation,
        } = self;
        let hurried = || HURRY.load(Ordering::SeqCst) != hurry_generation;

        let (outcome, target_read) = match publication {
            Publication::Delayed(delayed) => {
                if !immediately {
                    loop {
                        let due = restore_due(sent_at, delayed.render(), target_pid, timing);
                        let now = Instant::now();
                        if now >= due || hurried() {
                            break;
                        }
                        let _ = delayed.wait_for_render((due - now).min(SLICE));
                    }
                }
                let target_read = delayed.render().is_some_and(|render| {
                    render.requester_pid.is_none() || render.requester_pid == target_pid
                });
                // The owner window checks itself whether another program has taken the
                // clipboard since, and leaves it alone if so.
                let request = match plan_clipboard_restore(&previous, false) {
                    ClipboardRestore::RestorePrevious => match previous {
                        PreviousClipboard::Text(text) => FinishRequest::Restore(text),
                        _ => FinishRequest::Clear,
                    },
                    ClipboardRestore::Clear | ClipboardRestore::Leave => FinishRequest::Clear,
                };
                (delayed.finish(request), target_read)
            }
            Publication::Direct { sequence_after_set } => {
                if !immediately {
                    let due = sent_at + timing.direct_delay;
                    while Instant::now() < due && !hurried() {
                        thread::sleep(SLICE.min(due.saturating_duration_since(Instant::now())));
                    }
                }
                let changed = sequence_changed(sequence_after_set, clipboard_sequence());
                let outcome = match plan_clipboard_restore(&previous, changed) {
                    ClipboardRestore::Leave => FinishOutcome::LeftAlone,
                    ClipboardRestore::RestorePrevious | ClipboardRestore::Clear => {
                        restore_directly(&previous)
                    }
                };
                (outcome, false)
            }
        };

        if matches!(outcome, FinishOutcome::Restored { .. }) {
            remember_own_clipboard_text();
        }
        RestoreReport {
            outcome,
            waited: sent_at.elapsed(),
            target_read,
        }
    }
}

/// Replace the clipboard with the earlier text (with the exclusions), or clear it.
fn restore_directly(previous: &PreviousClipboard) -> FinishOutcome {
    let Ok(mut clipboard) = Clipboard::new() else {
        return FinishOutcome::Failed;
    };
    match previous {
        PreviousClipboard::Text(text) => match set_transient_clipboard_text(&mut clipboard, text) {
            Ok(()) => {
                remember_own_clipboard_text();
                FinishOutcome::Restored {
                    sequence: clipboard_sequence(),
                }
            }
            Err(_) => FinishOutcome::Failed,
        },
        PreviousClipboard::Private | PreviousClipboard::Other => match clipboard.clear() {
            Ok(()) => FinishOutcome::Cleared,
            Err(_) => FinishOutcome::Failed,
        },
    }
}

/// Bumped to ask every waiting restore to finish now.
static HURRY: AtomicU64 = AtomicU64::new(0);
/// Restores not yet finished, and the signal that one has finished.
static PENDING: Mutex<usize> = Mutex::new(0);
static PENDING_DONE: Condvar = Condvar::new();

struct PendingGuard;

impl PendingGuard {
    fn new() -> Self {
        *PENDING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
        Self
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        let mut pending = PENDING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *pending = pending.saturating_sub(1);
        PENDING_DONE.notify_all();
    }
}

/// Ask any waiting restore to finish now, and wait up to `timeout` for it.
fn hurry_pending_restores(timeout: Duration) {
    HURRY.fetch_add(1, Ordering::SeqCst);
    let pending = PENDING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let _ = PENDING_DONE
        .wait_timeout_while(pending, timeout, |pending| *pending > 0)
        .unwrap_or_else(|poisoned| poisoned.into_inner());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_text_is_restored_when_the_clipboard_is_untouched() {
        assert_eq!(
            plan_clipboard_restore(&PreviousClipboard::Text("before".into()), false),
            ClipboardRestore::RestorePrevious
        );
    }

    #[test]
    fn dictated_text_is_cleared_when_there_is_nothing_to_restore() {
        // The clipboard was empty or held an image: the dictated text must not stay behind.
        assert_eq!(
            plan_clipboard_restore(&PreviousClipboard::Other, false),
            ClipboardRestore::Clear
        );
    }

    #[test]
    fn content_marked_private_is_never_put_back() {
        // A password manager's secret: cleared rather than restored, so it is not
        // re-published into clipboard history or cloud sync, and does not outlive the
        // password manager's own clearing.
        assert_eq!(
            plan_clipboard_restore(&PreviousClipboard::Private, false),
            ClipboardRestore::Clear
        );
    }

    #[test]
    fn a_newer_copy_by_the_user_is_never_overwritten() {
        assert_eq!(
            plan_clipboard_restore(&PreviousClipboard::Text("before".into()), true),
            ClipboardRestore::Leave
        );
        assert_eq!(
            plan_clipboard_restore(&PreviousClipboard::Private, true),
            ClipboardRestore::Leave
        );
    }

    #[test]
    fn unknown_sequence_numbers_count_as_unchanged() {
        assert!(sequence_changed(Some(7), Some(8)));
        assert!(!sequence_changed(Some(7), Some(7)));
        assert!(!sequence_changed(None, Some(3)));
        assert!(!sequence_changed(Some(3), None));
    }

    fn render(after: Duration, requester_pid: Option<u32>, sent_at: Instant) -> Render {
        Render {
            at: sent_at + after,
            requester_pid,
        }
    }

    #[test]
    fn the_restore_follows_the_targets_read_not_a_fixed_delay() {
        let timing = restore_timing(false);
        let sent_at = Instant::now();

        // A slow target that reads after 2.5 s gets the dictated text: the restore waits.
        let slow = render(Duration::from_millis(2_500), Some(42), sent_at);
        assert_eq!(
            restore_due(sent_at, Some(slow), Some(42), &timing),
            slow.at + timing.after_target_read
        );

        // A fast target: never sooner than the fixed delay used before.
        let fast = render(Duration::from_millis(20), Some(42), sent_at);
        assert_eq!(
            restore_due(sent_at, Some(fast), Some(42), &timing),
            sent_at + timing.direct_delay
        );

        // A target that reads after 300 ms: restored a grace period after its read.
        let medium = render(Duration::from_millis(300), Some(42), sent_at);
        assert_eq!(
            restore_due(sent_at, Some(medium), Some(42), &timing),
            sent_at + Duration::from_millis(600)
        );
    }

    #[test]
    fn a_read_by_another_program_falls_back_to_a_longer_wait() {
        let timing = restore_timing(false);
        let sent_at = Instant::now();
        let monitor = render(Duration::from_millis(5), Some(7), sent_at);

        assert_eq!(
            restore_due(sent_at, Some(monitor), Some(42), &timing),
            sent_at + timing.after_other_read
        );
    }

    #[test]
    fn an_unattributed_read_counts_as_the_target() {
        let timing = restore_timing(false);
        let sent_at = Instant::now();
        let read = render(Duration::from_millis(500), None, sent_at);

        assert_eq!(
            restore_due(sent_at, Some(read), Some(42), &timing),
            read.at + timing.after_target_read
        );
    }

    #[test]
    fn an_unread_paste_is_restored_after_the_limit() {
        let sent_at = Instant::now();
        let local = restore_timing(false);
        let remote = restore_timing(true);

        assert_eq!(
            restore_due(sent_at, None, Some(42), &local),
            sent_at + local.unread_limit
        );
        assert!(remote.unread_limit > local.unread_limit);
    }

    #[test]
    fn remote_sessions_wait_longer_everywhere() {
        let local = restore_timing(false);
        let remote = restore_timing(true);

        assert!(remote.after_target_read >= local.after_target_read);
        assert!(remote.after_other_read > local.after_other_read);
        assert!(remote.direct_delay > local.direct_delay);
        // The direct path keeps the delay it had before delayed rendering.
        assert_eq!(local.direct_delay, Duration::from_millis(400));
    }

    #[test]
    fn hurrying_with_nothing_pending_returns_at_once() {
        let started = Instant::now();
        hurry_pending_restores(Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
