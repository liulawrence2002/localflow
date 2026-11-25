use std::collections::VecDeque;

/// How many cancelled ids are remembered. Only a worker started shortly before a cancel ever
/// asks, so a handful is plenty.
const REMEMBERED_CANCELS: usize = 16;

/// Where a dictation session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStatus {
    /// The newest session, still allowed to paste.
    Current,
    /// A newer recording started. It must not paste, but its transcript is kept so it can be
    /// recovered with "Copy last transcript".
    Superseded,
    /// The user cancelled it (Escape) or it was abandoned without speech. Discarded.
    Cancelled,
}

/// Tracks which dictation session is currently authorized to insert text.
///
/// Every recording is assigned a monotonically increasing `id`. `active` holds the id that is
/// allowed to reach insertion. Starting a new recording supersedes the previous one;
/// cancelling discards the active one. Either way any in-flight worker for an older id
/// becomes non-current and never pastes. This is the native-path equivalent of the spec's
/// session-identity + stale-event rejection requirement (§4.4).
#[derive(Debug, Default, Clone)]
pub(crate) struct SessionRegistry {
    seq: u64,
    active: Option<u64>,
    cancelled: VecDeque<u64>,
}

impl SessionRegistry {
    /// Assign and activate a fresh session id.
    pub(crate) fn begin(&mut self) -> u64 {
        self.seq += 1;
        self.active = Some(self.seq);
        self.seq
    }

    /// Cancel the active session without starting a new recording.
    pub(crate) fn invalidate(&mut self) {
        if let Some(id) = self.active.take() {
            if self.cancelled.len() == REMEMBERED_CANCELS {
                self.cancelled.pop_front();
            }
            self.cancelled.push_back(id);
        }
    }

    /// Whether `id` is still the session authorized to insert.
    pub(crate) fn is_current(&self, id: u64) -> bool {
        id != 0 && self.active == Some(id)
    }

    pub(crate) fn status(&self, id: u64) -> SessionStatus {
        if self.is_current(id) {
            SessionStatus::Current
        } else if id == 0 || self.cancelled.contains(&id) {
            SessionStatus::Cancelled
        } else {
            SessionStatus::Superseded
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_registry_begin_activates_new_id() {
        let mut registry = SessionRegistry::default();

        // id 0 (never started) is never authorized to insert.
        assert!(!registry.is_current(0));

        let first = registry.begin();
        assert_eq!(first, 1);
        assert!(registry.is_current(first));
        assert_eq!(registry.status(first), SessionStatus::Current);
    }

    #[test]
    fn new_session_supersedes_previous_in_flight_worker() {
        let mut registry = SessionRegistry::default();
        let first = registry.begin();

        // A second recording starts while the first is still being processed.
        let second = registry.begin();

        assert_ne!(first, second);
        assert!(!registry.is_current(first)); // stale worker must abort before pasting
        assert!(registry.is_current(second));
        // ...but its transcript is still worth keeping for recovery.
        assert_eq!(registry.status(first), SessionStatus::Superseded);
    }

    #[test]
    fn cancel_invalidates_the_current_session() {
        let mut registry = SessionRegistry::default();
        let session = registry.begin();
        assert!(registry.is_current(session));

        registry.invalidate();

        // The cancelled session can no longer insert and is discarded.
        assert!(!registry.is_current(session));
        assert_eq!(registry.status(session), SessionStatus::Cancelled);
    }

    #[test]
    fn cancelling_a_newer_recording_does_not_cancel_a_superseded_one() {
        let mut registry = SessionRegistry::default();
        let first = registry.begin();
        let second = registry.begin();

        registry.invalidate();

        assert_eq!(registry.status(first), SessionStatus::Superseded);
        assert_eq!(registry.status(second), SessionStatus::Cancelled);
        // A second cancel with nothing active changes nothing.
        registry.invalidate();
        assert_eq!(registry.status(first), SessionStatus::Superseded);
    }

    #[test]
    fn only_recent_cancels_are_remembered() {
        let mut registry = SessionRegistry::default();
        let first = registry.begin();
        registry.invalidate();
        for _ in 0..REMEMBERED_CANCELS {
            registry.begin();
            registry.invalidate();
        }

        // Forgotten, so it reads as superseded; never as current.
        assert_eq!(registry.status(first), SessionStatus::Superseded);
        assert!(!registry.is_current(first));
    }
}
