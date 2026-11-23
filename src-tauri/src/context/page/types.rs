//! What a window's content looks like once it is ready to be read aloud.
//!
//! These types are the contract between extraction (`extract.rs`), the capture policy
//! (`policy.rs`), the reading position (`cursor.rs`), speech preparation (`speech.rs`) and
//! the code that speaks.

use std::fmt;

/// Text taken from the page exactly as the page presents it.
///
/// The only way to make one is [`VerbatimText::from_page`], which extraction calls on text it
/// read from the accessibility tree. Nothing produced by a language model can become a
/// `VerbatimText`, so code that accepts only this type cannot paraphrase by accident.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct VerbatimText(String);

impl VerbatimText {
    /// Wrap text read from the page. Whitespace is collapsed to single spaces and trimmed;
    /// nothing else is changed.
    pub fn from_page(text: &str) -> Self {
        Self(text.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn char_count(&self) -> usize {
        self.0.chars().count()
    }
}

/// Page text must not reach a log through a stray `{:?}`.
impl fmt::Debug for VerbatimText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "VerbatimText({} chars)", self.char_count())
    }
}

/// Identifies a block across snapshots of the same page: derived from the block's kind, its
/// place under the headings above it, and its text, so re-reading the page gives the same
/// ids for content that did not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockKind {
    /// A heading of level 1 to 6.
    Heading(u8),
    Paragraph,
    ListItem,
    /// One table row, cells joined in reading order.
    TableRow,
    /// An image's description.
    Image,
    Quote,
}

/// One readable unit of a page: a heading, a paragraph, a list item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: BlockId,
    pub kind: BlockKind,
    pub text: VerbatimText,
    /// The block is at least partly visible in the window.
    pub in_viewport: bool,
}

/// How much confidence extraction has in what it found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractQuality {
    /// The page marks its main content (a `main` landmark or a single article).
    MainContent,
    /// No main content was marked; the densest text region was chosen by scoring.
    Guessed,
    /// No document structure at all: every piece of readable text in the window, in order.
    PlainText,
}

/// The readable content of one window at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageSnapshot {
    /// Identifies the page for keeping a reading position: a hash of the program and the
    /// address (or the title when there is no address). Never the address itself.
    pub page_key: u64,
    /// The window title, for announcing what is being read. Not logged.
    pub title: VerbatimText,
    pub blocks: Vec<Block>,
    pub quality: ExtractQuality,
    /// The page was larger than the limits and was cut.
    pub truncated: bool,
}

impl PageSnapshot {
    /// Index of the first block that is visible, or 0 when none is marked visible.
    pub fn first_visible(&self) -> usize {
        self.blocks
            .iter()
            .position(|block| block.in_viewport)
            .unwrap_or(0)
    }
}

/// Limits applied while extracting, so an enormous page cannot exhaust memory or time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractLimits {
    pub max_blocks: usize,
    pub max_chars: usize,
}

impl Default for ExtractLimits {
    fn default() -> Self {
        Self {
            max_blocks: 2_000,
            max_chars: 200_000,
        }
    }
}

/// Why a window may not be read. Each has a fixed sentence for the user; none contains
/// anything from the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyReason {
    /// The keyboard focus is in a password field.
    PasswordFocused,
    /// A private browsing window (InPrivate, Incognito, Private Browsing).
    PrivateWindow,
    /// A program that holds secrets, such as a password manager.
    SensitiveProgram,
    /// LocalFlow's own windows.
    OwnWindow,
    /// The owning program could not be identified, so the rules could not be applied.
    UnknownProgram,
}

impl DenyReason {
    pub fn message(self) -> &'static str {
        match self {
            Self::PasswordFocused => "I won't read a window while a password field is focused.",
            Self::PrivateWindow => "I don't read private browsing windows.",
            Self::SensitiveProgram => {
                "I don't read windows of password managers and similar programs."
            }
            Self::OwnWindow => "That is LocalFlow's own window.",
            Self::UnknownProgram => {
                "I can't tell which program that window belongs to, so I won't read it."
            }
        }
    }
}

/// Why a page could not be read. Messages never contain page content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// Reading this window is not allowed.
    Denied(DenyReason),
    /// The window exposes nothing readable (a canvas application, an elevated window, an
    /// application without accessibility support).
    NothingReadable,
    /// The window changed or closed between the request and the read.
    TargetChanged,
    /// The application did not answer in time.
    Timeout,
    /// The accessibility interface failed. Holds a short, content-free description.
    Unavailable(String),
}

impl fmt::Display for PageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied(reason) => formatter.write_str(reason.message()),
            Self::NothingReadable => {
                formatter.write_str("I can't find anything readable in that window.")
            }
            Self::TargetChanged => {
                formatter.write_str("The window changed before I could read it.")
            }
            Self::Timeout => formatter.write_str("That window took too long to answer."),
            Self::Unavailable(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_text_keeps_its_words_and_only_loses_extra_whitespace() {
        let text = VerbatimText::from_page("  The  quick\tbrown\n fox, 3.14 & co.  ");

        assert_eq!(text.as_str(), "The quick brown fox, 3.14 & co.");
        assert_eq!(text.char_count(), 31);
        assert!(VerbatimText::from_page(" \n ").is_empty());
    }

    #[test]
    fn debug_output_never_contains_the_text() {
        let text = VerbatimText::from_page("my secret account number");

        let shown = format!("{text:?}");

        assert!(!shown.contains("secret"));
        assert!(shown.contains("24 chars"));
    }

    #[test]
    fn reading_starts_at_the_first_visible_block() {
        let block = |id: u64, visible: bool| Block {
            id: BlockId(id),
            kind: BlockKind::Paragraph,
            text: VerbatimText::from_page("x"),
            in_viewport: visible,
        };
        let mut snapshot = PageSnapshot {
            page_key: 1,
            title: VerbatimText::from_page("t"),
            blocks: vec![
                block(1, false),
                block(2, false),
                block(3, true),
                block(4, true),
            ],
            quality: ExtractQuality::MainContent,
            truncated: false,
        };
        assert_eq!(snapshot.first_visible(), 2);

        snapshot
            .blocks
            .iter_mut()
            .for_each(|block| block.in_viewport = false);
        assert_eq!(snapshot.first_visible(), 0);
    }

    #[test]
    fn no_refusal_message_can_leak_window_content() {
        // Fixed sentences only: every message is a static string.
        for reason in [
            DenyReason::PasswordFocused,
            DenyReason::PrivateWindow,
            DenyReason::SensitiveProgram,
            DenyReason::OwnWindow,
            DenyReason::UnknownProgram,
        ] {
            assert!(!reason.message().is_empty());
            assert_eq!(PageError::Denied(reason).to_string(), reason.message());
        }
    }
}
