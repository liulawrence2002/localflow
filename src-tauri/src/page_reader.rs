//! Reads the focused window aloud, word for word.
//!
//! Two ways in, one path through:
//!
//! - the reading hotkey, pressed over any window, reads it with the conversation voice and
//!   stops when pressed again;
//! - inside a conversation, saying "read this page", "next paragraph" or "go back" does the
//!   same through the conversation's own voice ([`PageSource`]).
//!
//! Either way the window is read only because the user just asked, the capture policy is
//! applied before and after its content is fetched, and the text goes straight to the voice.
//! No language model sees it, and it is never logged, stored, or sent in an event: logs carry
//! counts and timings only. The reading position is kept in memory and advances only by what
//! was actually heard.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    audio::devices::SpeakerStream,
    cancel::CancelFlag,
    context::page::{
        cursor::{ReadRequest, ReadingPositions},
        extract::extract,
        policy::{evaluate_capture, CapturePolicy},
        speech::speech_chunks,
        types::{BlockId, ExtractLimits, PageError},
    },
    conversation::{engine::PageSource, router::ReadWhat, speaking::speak_pieces},
    insertion::target::foreground_target,
    overlay::{emit_status, emit_status_if_current, emit_status_tracked},
    platform::uia::{window_info, SnapshotOptions, UiaReader},
    tts::{
        phonon::{PhononConfig, PhononProvider},
        stream::StreamingTextToSpeech,
    },
};

const END_OF_PAGE_LINE: &str = "That's the end of the page.";
const NO_WINDOW_LINE: &str = "I can't tell which window to read.";
/// Pause between spoken pieces.
const SENTENCE_GAP: Duration = Duration::from_millis(140);
/// The reading voice is kept loaded between readings, and freed after this long without one.
const VOICE_IDLE_UNLOAD: Duration = Duration::from_secs(300);

/// Which blocks the pieces of the last read came from, so "heard this many pieces" can be
/// turned into "heard these blocks".
#[derive(Default)]
struct LastRead {
    page_key: u64,
    /// For each block read, its id and the number of pieces up to and including its last.
    block_ends: Vec<(BlockId, usize)>,
}

/// What the settings window shows about reading windows aloud.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenReadingStatus {
    pub available: bool,
    pub detail: String,
    pub hotkey: Option<String>,
    pub reading: bool,
}

/// Shared state for reading windows. Cheap to clone: everything is behind `Arc`.
#[derive(Clone)]
pub struct ScreenReader {
    inner: Arc<Inner>,
}

struct Inner {
    uia: UiaReader,
    policy: CapturePolicy,
    positions: Mutex<ReadingPositions>,
    last_read: Mutex<LastRead>,
    /// The hotkey-started reading in progress, if any: its number and the flag that stops it.
    active: Mutex<Option<(u64, CancelFlag)>>,
    /// Numbers readings so a finished one only clears its own entry.
    next_reading: AtomicU64,
    hotkey: Mutex<Option<String>>,
    /// The reading voice, loaded on the first reading and reused, with the folder it is
    /// from. Loading it again for every reading cost the model load each time.
    voice: Mutex<Option<(PathBuf, Arc<PhononProvider>)>>,
}

impl Default for ScreenReader {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                uia: UiaReader::new(),
                policy: CapturePolicy::default(),
                positions: Mutex::new(ReadingPositions::new()),
                last_read: Mutex::new(LastRead::default()),
                active: Mutex::new(None),
                next_reading: AtomicU64::new(1),
                hotkey: Mutex::new(None),
                voice: Mutex::new(None),
            }),
        }
    }
}

/// The blocks heard to their end, given that the first `pieces` pieces were heard. A block
/// cut off part-way is not included, so it is read again from its start next time.
fn blocks_heard(block_ends: &[(BlockId, usize)], pieces: usize) -> Vec<BlockId> {
    block_ends
        .iter()
        .filter(|(_, end)| *end <= pieces)
        .map(|(block, _)| *block)
        .collect()
}

/// Counts over an accessibility tree. Numbers only.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct TreeStats {
    offscreen: u32,
    without_rect: u32,
    empty_rect: u32,
    named: u32,
}

impl TreeStats {
    fn of(node: &crate::context::page::raw::RawNode) -> Self {
        let mut stats = Self::default();
        stats.add(node);
        stats
    }

    fn add(&mut self, node: &crate::context::page::raw::RawNode) {
        self.offscreen += u32::from(node.is_offscreen);
        match node.rect {
            None => self.without_rect += 1,
            Some(rect) if rect.is_empty() => self.empty_rect += 1,
            Some(_) => {}
        }
        self.named += u32::from(!node.name.is_empty());
        for child in &node.children {
            self.add(child);
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl ScreenReader {
    /// Read window `hwnd` and return the pieces to speak for `what`. An empty list means the
    /// end of the page. The error is a fixed sentence to say instead.
    fn read_window(&self, hwnd: isize, what: ReadWhat) -> Result<Vec<String>, String> {
        let inner = &self.inner;

        // Decide from the window's program and title before any of its content is fetched.
        let info = window_info(hwnd).ok_or_else(|| PageError::TargetChanged.to_string())?;
        evaluate_capture(&info, false, &inner.policy)
            .map_err(|reason| reason.message().to_string())?;

        let tree = inner
            .uia
            .snapshot(hwnd, &SnapshotOptions::default())
            .map_err(|error| error.to_string())?;
        // Shape of what came back, as counts: enough to diagnose "nothing readable" without
        // recording anything from the window.
        let stats = tree.root.as_ref().map(TreeStats::of).unwrap_or_default();
        tracing::info!(
            nodes = tree.node_count,
            read_ms = tree.elapsed_ms,
            has_root = tree.root.is_some(),
            root_kind = ?tree.root.as_ref().map(|root| root.control),
            offscreen = stats.offscreen,
            without_rect = stats.without_rect,
            empty_rect = stats.empty_rect,
            named = stats.named,
            "window tree fetched"
        );

        // Again with what only the tree knows: whether a password field has the focus.
        evaluate_capture(&tree.window, tree.focused_is_password, &inner.policy)
            .map_err(|reason| reason.message().to_string())?;

        let snapshot =
            extract(&tree, ExtractLimits::default()).map_err(|error| error.to_string())?;
        let request = match what {
            ReadWhat::FromHere => ReadRequest::FromHere,
            ReadWhat::FromStart => ReadRequest::FromStart,
            ReadWhat::Next => ReadRequest::Next,
            ReadWhat::Previous => ReadRequest::Previous,
            ReadWhat::Again => ReadRequest::Again,
        };
        let plan = lock(&inner.positions).plan(&snapshot, request);

        let mut pieces = Vec::new();
        let mut block_ends = Vec::with_capacity(plan.blocks.len());
        for block in &plan.blocks {
            pieces.extend(speech_chunks(block));
            block_ends.push((block.id, pieces.len()));
        }
        // Counts and timings only: nothing from the window itself.
        tracing::info!(
            nodes = tree.node_count,
            read_ms = tree.elapsed_ms,
            tree_truncated = tree.truncated,
            blocks_on_page = snapshot.blocks.len(),
            blocks_to_read = plan.blocks.len(),
            pieces = pieces.len(),
            quality = ?snapshot.quality,
            page_truncated = snapshot.truncated,
            "window read for speaking"
        );
        *lock(&inner.last_read) = LastRead {
            page_key: snapshot.page_key,
            block_ends,
        };
        Ok(pieces)
    }

    /// The first `pieces` pieces of the last read were heard: move the reading position past
    /// every block that was heard to its end.
    fn mark_heard(&self, pieces: usize) {
        let last = lock(&self.inner.last_read);
        let mut positions = lock(&self.inner.positions);
        for block in blocks_heard(&last.block_ends, pieces) {
            positions.mark_heard(last.page_key, block);
        }
    }

    fn is_reading(&self) -> bool {
        lock(&self.inner.active).is_some()
    }

    /// The reading voice for `model_dir`, loaded once and kept.
    fn voice(&self, model_dir: PathBuf) -> Arc<PhononProvider> {
        let mut cached = lock(&self.inner.voice);
        if let Some((dir, voice)) = cached.as_ref() {
            if *dir == model_dir {
                return voice.clone();
            }
        }
        let voice = Arc::new(PhononProvider::new(PhononConfig {
            model_dir: model_dir.clone(),
        }));
        *cached = Some((model_dir, voice.clone()));
        voice
    }

    /// The number the next reading will get, to tell whether one started since.
    fn readings_started(&self) -> u64 {
        self.inner.next_reading.load(Ordering::SeqCst)
    }

    /// Free the reading voice if no reading started since `readings` and none is running.
    fn unload_voice_if_idle(&self, readings: u64) -> bool {
        if self.readings_started() != readings || self.is_reading() {
            return false;
        }
        lock(&self.inner.voice).take().is_some()
    }

    /// Stop a hotkey-started reading, if one is in progress. Returns whether there was one.
    fn stop_reading(&self) -> bool {
        match lock(&self.inner.active).take() {
            Some((_, cancel)) => {
                cancel.cancel();
                true
            }
            None => false,
        }
    }
}

/// Inside a conversation: read whatever window has the focus when the user asks.
impl PageSource for ScreenReader {
    fn read(&self, what: ReadWhat) -> Result<Vec<String>, String> {
        let target = foreground_target().ok_or_else(|| NO_WINDOW_LINE.to_string())?;
        self.read_window(target.hwnd, what)
    }

    fn heard(&self, pieces: usize) {
        self.mark_heard(pieces);
    }
}

/// The reading hotkey: stop if reading, otherwise read the window that has the focus now.
pub fn toggle(app: &AppHandle) {
    let Some(reader) = app.try_state::<ScreenReader>() else {
        return;
    };
    if reader.stop_reading() {
        return;
    }
    if crate::conversation::is_active(app) {
        // The conversation owns the speaker and the overlay; its own "read this page" does the
        // same job. An overlay message here would replace the conversation's phase and hide the
        // overlay a moment later.
        tracing::info!("reading hotkey ignored during a conversation");
        return;
    }
    if crate::native_dictation::is_recording(app) {
        return;
    }
    // Taken now, before anything else can take the focus.
    let Some(target) = foreground_target() else {
        emit_status(app, "error", NO_WINDOW_LINE);
        return;
    };

    let cancel = CancelFlag::default();
    let reading = reader.inner.next_reading.fetch_add(1, Ordering::SeqCst);
    *lock(&reader.inner.active) = Some((reading, cancel.clone()));
    // Read-aloud of a transcript and reading a window would talk over each other.
    crate::tts::stop(app);

    let reader = reader.inner().clone();
    let app = app.clone();
    thread::spawn(move || {
        // Each status this reading shows is numbered; the next is shown only while the
        // overlay still shows the last. A dictation, a conversation or a newer reading may
        // have taken the overlay since, and this reading's end must not hide theirs.
        let mut shown = emit_status_tracked(&app, "processing", "Reading the window");
        let outcome = read_aloud(&app, &reader, target.hwnd, &cancel, &mut shown);
        // A newer reading may have started since; only clear this one's entry.
        {
            let mut active = lock(&reader.inner.active);
            if active
                .as_ref()
                .is_some_and(|(current, _)| *current == reading)
            {
                *active = None;
            }
        }
        let (phase, message) = if cancel.is_cancelled() {
            ("cancelled", "Stopped reading".to_string())
        } else {
            match outcome {
                Ok(()) => ("cancelled", "Finished reading".to_string()),
                Err(problem) => {
                    tracing::warn!(problem = %problem, "window could not be read aloud");
                    ("error", problem)
                }
            }
        };
        emit_status_if_current(&app, shown, phase, &message);

        let readings = reader.readings_started();
        thread::sleep(VOICE_IDLE_UNLOAD);
        if reader.unload_voice_if_idle(readings) {
            tracing::info!("unloaded the idle reading voice");
        }
    });
}

/// Fetch, speak, and record what was heard. The error is a fixed sentence for the overlay.
/// `shown` is the overlay status this reading last put up.
fn read_aloud(
    app: &AppHandle,
    reader: &ScreenReader,
    hwnd: isize,
    cancel: &CancelFlag,
    shown: &mut u64,
) -> Result<(), String> {
    let model_dir = crate::conversation::voice_model_dir(app).ok_or_else(|| {
        "The reading voice is not installed. Run scripts\\Get-PhononModel.ps1.".to_string()
    })?;
    let voice = reader.voice(model_dir);
    voice.prepare().map_err(|error| error.to_string())?;

    // Why a window cannot be read is said aloud as well as shown.
    let (pieces, refusal) = match reader.read_window(hwnd, ReadWhat::FromHere) {
        Ok(pieces) if pieces.is_empty() => (vec![END_OF_PAGE_LINE.to_string()], None),
        Ok(pieces) => (pieces, None),
        Err(sentence) => (vec![sentence.clone()], Some(sentence)),
    };
    let from_page =
        refusal.is_none() && pieces.first().map(String::as_str) != Some(END_OF_PAGE_LINE);
    if cancel.is_cancelled() {
        return Ok(());
    }

    let speaker = SpeakerStream::open()?;
    if let Some(epoch) = emit_status_if_current(app, *shown, "refining", "Reading aloud") {
        *shown = epoch;
    }
    let (tx, rx) = mpsc::channel();
    for piece in pieces {
        let _ = tx.send(piece);
    }
    drop(tx);
    let spoken = speak_pieces(
        voice.as_ref(),
        &speaker,
        rx,
        cancel,
        SENTENCE_GAP,
        &mut || {},
    );
    if from_page {
        reader.mark_heard(spoken.finished_pieces);
    }

    match (spoken.error, refusal) {
        (Some(error), _) => Err(error.to_string()),
        (None, Some(sentence)) => Err(sentence),
        (None, None) => Ok(()),
    }
}

/// Stop a hotkey-started reading. Called from the tray's "Stop reading" and on exit.
pub fn stop(app: &AppHandle) {
    if let Some(reader) = app.try_state::<ScreenReader>() {
        reader.stop_reading();
    }
}

/// Record which hotkey was registered, for the status shown to the user.
pub fn set_hotkey_label(app: &AppHandle, label: Option<String>) {
    if let Some(reader) = app.try_state::<ScreenReader>() {
        *lock(&reader.inner.hotkey) = label;
    }
}

#[tauri::command]
pub fn screen_reading_status(app: AppHandle) -> ScreenReadingStatus {
    let reader = app.state::<ScreenReader>();
    let hotkey = lock(&reader.inner.hotkey).clone();
    let voice_installed = crate::conversation::voice_model_dir(&app).is_some();
    ScreenReadingStatus {
        available: voice_installed && hotkey.is_some(),
        detail: if !voice_installed {
            "The reading voice is not installed. Run scripts\\Get-PhononModel.ps1.".to_string()
        } else if hotkey.is_none() {
            "No reading hotkey could be registered. In a conversation, say \"read this page\"."
                .to_string()
        } else {
            "Reads the focused window word for word. Nothing is stored and no language model sees it.".to_string()
        },
        hotkey,
        reading: reader.is_reading(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::page::raw::{ControlKind, RawNode, Rect};

    #[test]
    fn the_reading_voice_is_loaded_once_and_freed_only_when_idle() {
        let reader = ScreenReader::default();
        let dir = PathBuf::from("Z:/someone/models/phonon");

        let first = reader.voice(dir.clone());
        let second = reader.voice(dir.clone());
        assert!(Arc::ptr_eq(&first, &second), "the voice must be reused");
        // Another install folder gets its own voice.
        assert!(!Arc::ptr_eq(
            &first,
            &reader.voice(PathBuf::from("Z:/other"))
        ));
        let first = reader.voice(dir);

        // A reading started after the idle timer was armed keeps the voice.
        let armed = reader.readings_started();
        reader.inner.next_reading.fetch_add(1, Ordering::SeqCst);
        assert!(!reader.unload_voice_if_idle(armed));
        // A reading in progress keeps it too.
        let armed = reader.readings_started();
        *lock(&reader.inner.active) = Some((armed, CancelFlag::default()));
        assert!(!reader.unload_voice_if_idle(armed));
        *lock(&reader.inner.active) = None;

        assert!(reader.unload_voice_if_idle(armed));
        assert!(lock(&reader.inner.voice).is_none());
        drop(first);
    }

    #[test]
    fn only_blocks_heard_to_the_end_move_the_reading_position() {
        // Three blocks of 2, 1 and 3 pieces.
        let ends = [(BlockId(10), 2), (BlockId(20), 3), (BlockId(30), 6)];

        assert_eq!(blocks_heard(&ends, 0), Vec::<BlockId>::new());
        // One piece into a two-piece block: not heard to the end.
        assert_eq!(blocks_heard(&ends, 1), Vec::<BlockId>::new());
        assert_eq!(blocks_heard(&ends, 2), vec![BlockId(10)]);
        assert_eq!(blocks_heard(&ends, 5), vec![BlockId(10), BlockId(20)]);
        assert_eq!(
            blocks_heard(&ends, 6),
            vec![BlockId(10), BlockId(20), BlockId(30)]
        );
        // More pieces reported than exist changes nothing.
        assert_eq!(blocks_heard(&ends, 99).len(), 3);
    }

    #[test]
    fn tree_statistics_count_shapes_and_never_hold_text() {
        let mut hidden = RawNode::text("secret words");
        hidden.is_offscreen = true;
        hidden.rect = Some(Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 10,
        });
        let mut visible = RawNode::text("more words");
        visible.rect = Some(Rect {
            left: 0,
            top: 0,
            right: 50,
            bottom: 10,
        });
        let root = RawNode::container(ControlKind::Document, vec![hidden, visible]);

        let stats = TreeStats::of(&root);

        assert_eq!(
            stats,
            TreeStats {
                offscreen: 1,
                without_rect: 1,
                empty_rect: 1,
                named: 2,
            }
        );
        assert!(!format!("{stats:?}").contains("secret"));
    }
}
