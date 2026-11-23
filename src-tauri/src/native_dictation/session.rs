/// Tracks which dictation session is currently authorized to insert text.
///
/// Every recording is assigned a monotonically increasing `id`. `active` holds the
/// id that is allowed to reach insertion. Starting a new recording or cancelling
/// bumps the sequence so any in-flight worker for an older id becomes non-current
/// and aborts before pasting. This is the native-path equivalent of the spec's
/// session-identity + stale-event rejection requirement (§4.4): a cancelled or
/// superseded session must never insert text.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct SessionRegistry {
    pub(crate) seq: u64,
    pub(crate) active: u64,
}

impl SessionRegistry {
    /// Assign and activate a fresh session id.
    pub(crate) fn begin(&mut self) -> u64 {
        self.seq += 1;
        self.active = self.seq;
        self.seq
    }

    /// Invalidate the current session without starting a new recording (cancel).
    /// The active id becomes one that no worker holds, so any in-flight worker aborts.
    pub(crate) fn invalidate(&mut self) {
        self.seq += 1;
        self.active = self.seq;
    }

    /// Whether `id` is still the session authorized to insert.
    pub(crate) fn is_current(&self, id: u64) -> bool {
        id != 0 && self.active == id
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
    }

    #[test]
    fn cancel_invalidates_the_current_session() {
        let mut registry = SessionRegistry::default();
        let session = registry.begin();
        assert!(registry.is_current(session));

        registry.invalidate();

        // The cancelled session can no longer insert, and no worker holds the new id.
        assert!(!registry.is_current(session));
    }
}
