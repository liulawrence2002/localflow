//! A cheap, clonable cancellation signal shared between a coordinator and its workers.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

/// Set once by whoever wants work to stop; polled by the work. Clones share the same signal,
/// so one flag can be handed to the language model, the voice and playback of a single turn
/// and cancel all three together.
#[derive(Debug, Clone, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clones_share_one_signal() {
        let flag = CancelFlag::default();
        let worker_copy = flag.clone();
        assert!(!worker_copy.is_cancelled());

        flag.cancel();

        assert!(worker_copy.is_cancelled());
    }

    #[test]
    fn separate_flags_are_independent() {
        let first = CancelFlag::default();
        let second = CancelFlag::default();

        first.cancel();

        assert!(!second.is_cancelled());
    }
}
