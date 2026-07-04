//! Streaming transcript stabilizer.
//!
//! When ASR is run on rolling, overlapping audio windows, each decode returns a fresh
//! hypothesis for the whole utterance so far. To emit a stable stream we commit the longest
//! token prefix that agreed across the previous and current hypotheses, and only ever grow
//! the committed text — so committed words are never duplicated or rewritten (spec §3.2).
//!
//! Authoritative Rust port of the tested `src/domain/transcriptStabilizer.ts`.
//!
//! Streaming ASR foundation (spec Phase 3): exercised by the unit tests below and wired into
//! `StreamingSession`; not yet on the default one-shot path, so `dead_code` is allowed.
#![allow(dead_code)]

/// The result of feeding one hypothesis to the stabilizer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StabilizerUpdate {
    /// All committed (stable) text so far.
    pub committed_text: String,
    /// Just the text newly committed by this update (empty if nothing new committed).
    pub new_commit: String,
    /// The tentative tail that is not yet stable.
    pub provisional_text: String,
}

#[derive(Debug, Default)]
pub struct TranscriptStabilizer {
    committed: Vec<String>,
    previous: Vec<String>,
}

impl TranscriptStabilizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed the latest full-utterance hypothesis. Commits the newly stable prefix and
    /// reports the tentative tail.
    pub fn update(&mut self, hypothesis: &str) -> StabilizerUpdate {
        let current = tokenize(hypothesis);
        let stable = longest_common_prefix(&self.previous, &current);

        let new_tokens: Vec<String> = if stable.len() > self.committed.len() {
            stable[self.committed.len()..].to_vec()
        } else {
            Vec::new()
        };
        if !new_tokens.is_empty() {
            self.committed = stable;
        }

        let provisional: Vec<String> = if current.len() > self.committed.len() {
            current[self.committed.len()..].to_vec()
        } else {
            Vec::new()
        };
        self.previous = current;

        StabilizerUpdate {
            committed_text: detokenize(&self.committed),
            new_commit: detokenize(&new_tokens),
            provisional_text: detokenize(&provisional),
        }
    }

    /// Final decode when recording stops: everything becomes committed.
    pub fn finalize(&mut self, hypothesis: &str) -> StabilizerUpdate {
        let current = tokenize(hypothesis);
        let new_tokens: Vec<String> = if current.len() > self.committed.len() {
            current[self.committed.len()..].to_vec()
        } else {
            Vec::new()
        };
        self.committed = current.clone();
        self.previous = current;

        StabilizerUpdate {
            committed_text: detokenize(&self.committed),
            new_commit: detokenize(&new_tokens),
            provisional_text: String::new(),
        }
    }

    pub fn reset(&mut self) {
        self.committed.clear();
        self.previous.clear();
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

fn detokenize(tokens: &[String]) -> String {
    tokens.join(" ")
}

fn longest_common_prefix(left: &[String], right: &[String]) -> Vec<String> {
    let mut index = 0;
    let limit = left.len().min(right.len());
    while index < limit && left[index].eq_ignore_ascii_case(&right[index]) {
        index += 1;
    }
    right[..index].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commits_growing_prefix_without_duplicating_words() {
        let mut stabilizer = TranscriptStabilizer::new();

        // First hypothesis has no prior to agree with, so nothing commits yet.
        let first = stabilizer.update("the");
        assert_eq!(first.committed_text, "");
        assert_eq!(first.provisional_text, "the");

        let second = stabilizer.update("the quick");
        assert_eq!(second.new_commit, "the");
        assert_eq!(second.committed_text, "the");
        assert_eq!(second.provisional_text, "quick");

        let third = stabilizer.update("the quick brown");
        assert_eq!(third.new_commit, "quick");
        assert_eq!(third.committed_text, "the quick");
        assert_eq!(third.provisional_text, "brown");
    }

    #[test]
    fn overlapping_windows_never_double_committed_words() {
        let mut stabilizer = TranscriptStabilizer::new();
        stabilizer.update("hello world");
        stabilizer.update("hello world this");
        let update = stabilizer.update("hello world this is");
        // "hello world" committed once; no repeats even as the window re-decodes them.
        assert_eq!(update.committed_text, "hello world this");
        assert!(!update.committed_text.contains("hello world hello"));
    }

    #[test]
    fn diverging_tail_is_not_committed() {
        let mut stabilizer = TranscriptStabilizer::new();
        stabilizer.update("meet me on");
        stabilizer.update("meet me on tuesday");
        // The model revises the tail; only the agreed prefix stays committed.
        let update = stabilizer.update("meet me on wednesday");
        assert_eq!(update.committed_text, "meet me on");
        assert_eq!(update.provisional_text, "wednesday");
    }

    #[test]
    fn finalize_commits_everything() {
        let mut stabilizer = TranscriptStabilizer::new();
        stabilizer.update("the quick");
        let update = stabilizer.finalize("the quick brown fox");
        assert_eq!(update.committed_text, "the quick brown fox");
        assert_eq!(update.provisional_text, "");
    }
}
