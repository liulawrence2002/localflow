//! The last transcript kept for "Copy last transcript", and the notice that a superseded
//! dictation was not pasted.
//!
//! Transcripts are ordered by session id, so a worker that finishes late never replaces the
//! transcript of a newer dictation. A superseded dictation never pastes; its transcript is
//! kept and the user is told once nothing else is using the overlay, and only while that
//! transcript is still the one "Copy last transcript" would return. Volatile memory only.

#[derive(Debug, Default)]
pub(crate) struct Recovery {
    last: Option<(u64, String)>,
    /// A stored transcript that was not pasted and not yet announced.
    unannounced: Option<u64>,
}

impl Recovery {
    /// Keep `text` as the last transcript unless a newer session already stored one.
    /// Returns whether it was kept.
    pub(crate) fn store(&mut self, session_id: u64, text: &str) -> bool {
        if self
            .last
            .as_ref()
            .is_some_and(|(stored, _)| *stored > session_id)
        {
            return false;
        }
        self.last = Some((session_id, text.to_string()));
        if self.unannounced.is_some_and(|pending| pending < session_id) {
            // A newer transcript replaced the one the notice was about.
            self.unannounced = None;
        }
        true
    }

    /// Replace the transcript of `session_id` with its cleaned-up version, if it is still
    /// the last one.
    pub(crate) fn refine(&mut self, session_id: u64, text: &str) -> bool {
        match self.last.as_mut() {
            Some((stored, current)) if *stored == session_id => {
                *current = text.to_string();
                true
            }
            _ => false,
        }
    }

    pub(crate) fn text(&self) -> Option<String> {
        self.last.as_ref().map(|(_, text)| text.clone())
    }

    /// Note that the stored transcript of `session_id` was not pasted, so the user should be
    /// told.
    pub(crate) fn mark_unpasted(&mut self, session_id: u64) {
        if self
            .last
            .as_ref()
            .is_some_and(|(stored, _)| *stored == session_id)
        {
            self.unannounced = Some(session_id);
        }
    }

    /// Whether to show the "not pasted" notice now. `busy` is true while a recording or a
    /// transcription is running, which owns the overlay; the notice then waits. Answers yes
    /// at most once per transcript.
    pub(crate) fn take_notice(&mut self, busy: bool) -> bool {
        if busy {
            return false;
        }
        match self.unannounced.take() {
            Some(pending) => self
                .last
                .as_ref()
                .is_some_and(|(stored, _)| *stored == pending),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_late_worker_never_replaces_a_newer_transcript() {
        let mut recovery = Recovery::default();

        assert!(recovery.store(2, "newer"));
        assert!(!recovery.store(1, "older, finished late"));

        assert_eq!(recovery.text().as_deref(), Some("newer"));
    }

    #[test]
    fn a_superseded_transcript_is_recoverable_and_announced_once_idle() {
        let mut recovery = Recovery::default();
        recovery.store(1, "first dictation");
        recovery.mark_unpasted(1);

        // A newer recording is running: the overlay belongs to it.
        assert!(!recovery.take_notice(true));
        // It ended without a transcript of its own (nothing said): now tell the user.
        assert!(recovery.take_notice(false));
        assert!(!recovery.take_notice(false), "announced only once");
        assert_eq!(recovery.text().as_deref(), Some("first dictation"));
    }

    #[test]
    fn the_notice_is_dropped_once_a_newer_transcript_replaces_it() {
        let mut recovery = Recovery::default();
        recovery.store(1, "first dictation");
        recovery.mark_unpasted(1);

        recovery.store(2, "second dictation");

        // "Copy last transcript" now returns the second one, so the notice would mislead.
        assert!(!recovery.take_notice(false));
    }

    #[test]
    fn only_the_stored_transcript_can_be_marked_unpasted() {
        let mut recovery = Recovery::default();
        recovery.store(3, "third");

        recovery.mark_unpasted(2);

        assert!(!recovery.take_notice(false));
    }

    #[test]
    fn cleanup_replaces_only_its_own_transcript() {
        let mut recovery = Recovery::default();
        recovery.store(1, "first");
        assert!(recovery.refine(1, "First."));
        assert_eq!(recovery.text().as_deref(), Some("First."));

        recovery.store(2, "second");
        assert!(!recovery.refine(1, "First, again."));
        assert_eq!(recovery.text().as_deref(), Some("second"));
    }
}
