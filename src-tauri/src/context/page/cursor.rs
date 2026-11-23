//! Where the listener is on each page.
//!
//! A position is the id of the last block the listener actually heard, kept per page and only
//! in memory. Planning what to read next never moves it: the position moves when the voice
//! reports that a block was spoken, so stopping halfway through a page resumes at the right
//! block instead of where the plan would have ended.

use super::types::{Block, BlockId, BlockKind, PageSnapshot};

/// Pages whose position is remembered. The least recently used page is forgotten first.
const MAX_REMEMBERED_PAGES: usize = 16;

/// What the listener asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadRequest {
    /// Carry on after the last block heard, to the end of the page.
    FromHere,
    /// The whole page from its first block.
    FromStart,
    /// Only the block after the last one heard.
    Next,
    /// Only the block before the last one heard.
    Previous,
    /// The last block heard, once more.
    Again,
}

/// The blocks to speak for one request, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadPlan {
    /// Index in the snapshot of the first block returned. The snapshot's block count when
    /// `blocks` is empty.
    pub start_index: usize,
    pub blocks: Vec<Block>,
    /// Nothing follows the returned blocks on this page.
    pub reached_end: bool,
}

/// The last block heard on each recently read page. Holds hashes and ids only, never text.
#[derive(Debug, Clone, Default)]
pub struct ReadingPositions {
    /// Least recently used first.
    pages: Vec<(u64, BlockId)>,
}

impl ReadingPositions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Choose the blocks of `snapshot` that answer `request`. The stored position is not
    /// advanced; call [`ReadingPositions::mark_heard`] as each block is spoken.
    ///
    /// A stored position whose block is no longer on the page (the page changed) counts as
    /// no position, and reading starts at the first visible block.
    pub fn plan(&mut self, snapshot: &PageSnapshot, request: ReadRequest) -> ReadPlan {
        let blocks = &snapshot.blocks;
        let count = blocks.len();
        let heard = self
            .position(snapshot.page_key)
            .and_then(|id| blocks.iter().position(|block| block.id == id));
        let first_visible = snapshot.first_visible().min(count);

        let (start, end) = match request {
            ReadRequest::FromStart => (0, count),
            ReadRequest::FromHere => (heard.map_or(first_visible, |index| index + 1), count),
            ReadRequest::Next => {
                let start = heard.map_or(first_visible, |index| index + 1);
                (start, end_after_headings(blocks, start))
            }
            ReadRequest::Previous => {
                let start = heard.map_or(first_visible, |index| index.saturating_sub(1));
                (start, start + 1)
            }
            ReadRequest::Again => {
                let start = heard.unwrap_or(first_visible);
                (start, start + 1)
            }
        };
        let start = start.min(count);
        let end = end.clamp(start, count);

        ReadPlan {
            start_index: start,
            blocks: blocks[start..end].to_vec(),
            reached_end: end >= count,
        }
    }

    /// Record that the listener heard `block` on the page `page_key`.
    pub fn mark_heard(&mut self, page_key: u64, block: BlockId) {
        self.pages.retain(|(key, _)| *key != page_key);
        self.pages.push((page_key, block));
        if self.pages.len() > MAX_REMEMBERED_PAGES {
            let excess = self.pages.len() - MAX_REMEMBERED_PAGES;
            self.pages.drain(..excess);
        }
    }

    /// Forget every position.
    pub fn clear(&mut self) {
        self.pages.clear();
    }

    /// The stored position for a page. Looking a page up counts as using it, so a page the
    /// listener keeps returning to is not the one forgotten.
    fn position(&mut self, page_key: u64) -> Option<BlockId> {
        let index = self.pages.iter().position(|(key, _)| *key == page_key)?;
        let entry = self.pages.remove(index);
        self.pages.push(entry);
        Some(entry.1)
    }
}

/// End of a single-block read starting at `start`, extended past headings so that "next"
/// never speaks a heading alone and then stops.
fn end_after_headings(blocks: &[Block], start: usize) -> usize {
    let mut end = start;
    while end < blocks.len() {
        end += 1;
        if !matches!(blocks[end - 1].kind, BlockKind::Heading(_)) {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::super::types::{ExtractQuality, VerbatimText};
    use super::*;

    const PAGE: u64 = 7;

    /// A page of blocks with ids 0, 1, 2, ... Kinds: `h` heading, anything else paragraph.
    /// Blocks from `first_visible` on are in the viewport.
    fn page(kinds: &str, first_visible: usize) -> PageSnapshot {
        let blocks = kinds
            .chars()
            .enumerate()
            .map(|(index, kind)| Block {
                id: BlockId(index as u64),
                kind: if kind == 'h' {
                    BlockKind::Heading(2)
                } else {
                    BlockKind::Paragraph
                },
                text: VerbatimText::from_page("words"),
                in_viewport: index >= first_visible,
            })
            .collect();
        PageSnapshot {
            page_key: PAGE,
            title: VerbatimText::from_page("title"),
            blocks,
            quality: ExtractQuality::MainContent,
            truncated: false,
        }
    }

    fn ids(plan: &ReadPlan) -> Vec<u64> {
        plan.blocks.iter().map(|block| block.id.0).collect()
    }

    #[test]
    fn reading_from_here_on_a_new_page_starts_at_the_first_visible_block() {
        let mut positions = ReadingPositions::new();

        let plan = positions.plan(&page("ppppp", 2), ReadRequest::FromHere);

        assert_eq!(plan.start_index, 2);
        assert_eq!(ids(&plan), vec![2, 3, 4]);
        assert!(plan.reached_end);
    }

    #[test]
    fn reading_from_here_continues_after_the_last_block_heard() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(1));

        let plan = positions.plan(&page("ppppp", 0), ReadRequest::FromHere);

        assert_eq!(plan.start_index, 2);
        assert_eq!(ids(&plan), vec![2, 3, 4]);
        assert!(plan.reached_end);
    }

    #[test]
    fn planning_never_moves_the_position() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(1));
        let snapshot = page("ppppp", 0);

        let first = positions.plan(&snapshot, ReadRequest::Next);
        let second = positions.plan(&snapshot, ReadRequest::Next);
        positions.plan(&snapshot, ReadRequest::FromStart);
        let third = positions.plan(&snapshot, ReadRequest::Next);

        assert_eq!(ids(&first), vec![2]);
        assert_eq!(first, second);
        assert_eq!(first, third);
    }

    #[test]
    fn reading_from_the_start_ignores_the_position_and_the_viewport() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(3));

        let plan = positions.plan(&page("ppppp", 2), ReadRequest::FromStart);

        assert_eq!(plan.start_index, 0);
        assert_eq!(ids(&plan), vec![0, 1, 2, 3, 4]);
        assert!(plan.reached_end);
    }

    #[test]
    fn next_reads_one_block_and_says_whether_more_follows() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("pppp", 0);

        positions.mark_heard(PAGE, BlockId(0));
        let middle = positions.plan(&snapshot, ReadRequest::Next);
        positions.mark_heard(PAGE, BlockId(2));
        let last = positions.plan(&snapshot, ReadRequest::Next);

        assert_eq!(
            (middle.start_index, ids(&middle), middle.reached_end),
            (1, vec![1], false)
        );
        assert_eq!(
            (last.start_index, ids(&last), last.reached_end),
            (3, vec![3], true)
        );
    }

    #[test]
    fn next_never_reads_a_heading_alone() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("phhpp", 0);
        positions.mark_heard(PAGE, BlockId(0));

        let plan = positions.plan(&snapshot, ReadRequest::Next);

        assert_eq!(
            ids(&plan),
            vec![1, 2, 3],
            "both headings and the paragraph under them"
        );
        assert!(!plan.reached_end);

        // A heading that ends the page has nothing to bring along.
        let trailing = page("ph", 0);
        let plan = positions.plan(&trailing, ReadRequest::Next);
        assert_eq!(ids(&plan), vec![1]);
        assert!(plan.reached_end);
    }

    #[test]
    fn next_on_a_new_page_reads_the_first_visible_block() {
        let mut positions = ReadingPositions::new();

        let plan = positions.plan(&page("pphpp", 2), ReadRequest::Next);

        assert_eq!(plan.start_index, 2);
        assert_eq!(ids(&plan), vec![2, 3]);
    }

    #[test]
    fn next_and_from_here_at_the_end_of_the_page_return_nothing() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("ppp", 0);
        positions.mark_heard(PAGE, BlockId(2));

        for request in [ReadRequest::Next, ReadRequest::FromHere] {
            let plan = positions.plan(&snapshot, request);

            assert!(plan.blocks.is_empty(), "{request:?}");
            assert!(plan.reached_end, "{request:?}");
            assert_eq!(plan.start_index, 3, "{request:?}");
        }
    }

    #[test]
    fn previous_reads_the_block_before_the_last_one_heard() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("pppp", 0);
        positions.mark_heard(PAGE, BlockId(2));

        let plan = positions.plan(&snapshot, ReadRequest::Previous);

        assert_eq!(
            (plan.start_index, ids(&plan), plan.reached_end),
            (1, vec![1], false)
        );
    }

    #[test]
    fn previous_at_the_start_reads_the_first_block() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("ppp", 0);
        positions.mark_heard(PAGE, BlockId(0));

        let plan = positions.plan(&snapshot, ReadRequest::Previous);

        assert_eq!((plan.start_index, ids(&plan)), (0, vec![0]));
    }

    #[test]
    fn again_repeats_the_last_block_heard() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("ppp", 0);
        positions.mark_heard(PAGE, BlockId(2));

        let plan = positions.plan(&snapshot, ReadRequest::Again);

        assert_eq!(
            (plan.start_index, ids(&plan), plan.reached_end),
            (2, vec![2], true)
        );
    }

    #[test]
    fn again_and_previous_on_a_new_page_read_the_first_visible_block() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("pppp", 1);

        assert_eq!(ids(&positions.plan(&snapshot, ReadRequest::Again)), vec![1]);
        assert_eq!(
            ids(&positions.plan(&snapshot, ReadRequest::Previous)),
            vec![1]
        );
    }

    #[test]
    fn a_position_whose_block_left_the_page_counts_as_no_position() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(99));
        let snapshot = page("pppp", 1);

        assert_eq!(
            ids(&positions.plan(&snapshot, ReadRequest::FromHere)),
            vec![1, 2, 3]
        );
        assert_eq!(ids(&positions.plan(&snapshot, ReadRequest::Next)), vec![1]);
        assert_eq!(
            ids(&positions.plan(&snapshot, ReadRequest::Previous)),
            vec![1]
        );
        assert_eq!(ids(&positions.plan(&snapshot, ReadRequest::Again)), vec![1]);
    }

    #[test]
    fn positions_are_kept_per_page() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(2));
        let mut other = page("pppp", 0);
        other.page_key = PAGE + 1;

        assert_eq!(
            ids(&positions.plan(&other, ReadRequest::FromHere)),
            vec![0, 1, 2, 3]
        );
        assert_eq!(
            ids(&positions.plan(&page("pppp", 0), ReadRequest::FromHere)),
            vec![3]
        );
    }

    #[test]
    fn an_empty_page_gives_an_empty_plan_for_every_request() {
        let mut positions = ReadingPositions::new();
        let snapshot = page("", 0);

        for request in [
            ReadRequest::FromHere,
            ReadRequest::FromStart,
            ReadRequest::Next,
            ReadRequest::Previous,
            ReadRequest::Again,
        ] {
            let plan = positions.plan(&snapshot, request);

            assert!(plan.blocks.is_empty());
            assert!(plan.reached_end);
            assert_eq!(plan.start_index, 0);
        }
    }

    #[test]
    fn only_the_sixteen_most_recently_used_pages_are_remembered() {
        let mut positions = ReadingPositions::new();
        let on_page = |key: u64| {
            let mut snapshot = page("ppp", 0);
            snapshot.page_key = key;
            snapshot
        };
        for key in 0..16 {
            positions.mark_heard(key, BlockId(1));
        }
        // Using page 0 makes page 1 the least recently used.
        assert_eq!(
            ids(&positions.plan(&on_page(0), ReadRequest::FromHere)),
            vec![2]
        );

        positions.mark_heard(100, BlockId(1));

        assert_eq!(positions.pages.len(), 16);
        assert_eq!(
            ids(&positions.plan(&on_page(1), ReadRequest::FromHere)),
            vec![0, 1, 2],
            "the least recently used page was forgotten"
        );
        assert_eq!(
            ids(&positions.plan(&on_page(0), ReadRequest::FromHere)),
            vec![2]
        );
        assert_eq!(
            ids(&positions.plan(&on_page(100), ReadRequest::FromHere)),
            vec![2]
        );
    }

    #[test]
    fn hearing_another_block_on_the_same_page_replaces_the_position() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(0));
        positions.mark_heard(PAGE, BlockId(2));

        assert_eq!(positions.pages.len(), 1);
        assert_eq!(
            ids(&positions.plan(&page("pppp", 0), ReadRequest::FromHere)),
            vec![3]
        );
    }

    #[test]
    fn clearing_forgets_every_position() {
        let mut positions = ReadingPositions::new();
        positions.mark_heard(PAGE, BlockId(2));

        positions.clear();

        assert_eq!(
            ids(&positions.plan(&page("ppp", 0), ReadRequest::FromHere)),
            vec![0, 1, 2]
        );
    }
}
