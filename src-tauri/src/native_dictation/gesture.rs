//! What the recorder does with each hotkey and level-meter event: the tap-or-hold hybrid.
//!
//! - Press starts a recording, with the key counted as held.
//! - A release within [`QUICK_TAP_MS`] makes it hands-free: it stops after the end-of-speech
//!   silence, or on the next press.
//! - A later release ends it (push-to-talk). While the key is held, silence never stops it;
//!   only the maximum length does.
//! - A press within [`AFTER_AUTO_STOP_IGNORE_MS`] of an automatic stop is ignored: it was
//!   almost certainly meant to stop the recording that had just stopped itself, and starting
//!   a new one would supersede the dictation being transcribed.
//! - A lost release never leaves a recording unstoppable: a fresh press while the key still
//!   counts as held finishes it, and [`ReleaseWatch`] notices a key that has physically been
//!   up for a while and reports the release itself.
//!
//! Pure, so every rule is unit-tested.

use std::time::{Duration, Instant};

use crate::audio::vad::StopReason;

/// A release this soon after the press is a tap (hands-free), not the end of a hold.
pub(crate) const QUICK_TAP_MS: u64 = 700;
/// A press this soon after an automatic stop is ignored.
pub(crate) const AFTER_AUTO_STOP_IGNORE_MS: u64 = 700;
/// How long the hotkey must read as physically up, while the recording still counts it as
/// held, before the release is taken as lost. The global-shortcut plugin reports a release
/// within about 50 ms, so a real one always arrives first.
pub(crate) const MISSED_RELEASE_AFTER_MS: u64 = 300;

/// Notices a hotkey release that was never reported. Feed it the key's physical state while
/// the recording counts the key as held; it answers true once per release.
#[derive(Debug, Default)]
pub(crate) struct ReleaseWatch {
    up_since: Option<Instant>,
    reported: bool,
}

impl ReleaseWatch {
    /// Whether the release should be reported now.
    pub(crate) fn observe(&mut self, now: Instant, key_down: bool) -> bool {
        if key_down {
            self.up_since = None;
            self.reported = false;
            return false;
        }
        let up_since = *self.up_since.get_or_insert(now);
        if !self.reported
            && now.duration_since(up_since) >= Duration::from_millis(MISSED_RELEASE_AFTER_MS)
        {
            self.reported = true;
            return true;
        }
        false
    }
}

/// An event for the recorder loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecorderInput {
    Pressed,
    Released,
    /// The level meter ended the recording.
    MeterStop {
        reason: StopReason,
        speech_seen: bool,
    },
    /// Escape.
    Cancel,
    /// A worker finished; a deferred "not pasted" notice may be shown now.
    WorkerDone,
}

/// The recording in progress, as the decision needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActiveRecording {
    /// Time since the recording started.
    pub(crate) elapsed: Duration,
    /// Whether the hotkey that started it is still held.
    pub(crate) key_held: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Start a recording.
    Start,
    /// Stop recording and transcribe.
    Finish,
    /// Stop recording silently and discard it (nothing was said).
    Discard,
    /// Stop recording, discard it and cancel any dictation still being processed.
    Cancel,
    /// The tap was released: carry on hands-free.
    HandsFree,
    /// Show a deferred notice if nothing else is going on.
    ShowNotice,
    /// Do nothing, for the reason given (logged, never shown).
    Ignore(&'static str),
}

/// Decide what to do with `input`. `since_auto_stop` is the time since the level meter last
/// stopped a recording, if it ever did.
pub(crate) fn decide(
    input: RecorderInput,
    active: Option<ActiveRecording>,
    since_auto_stop: Option<Duration>,
) -> Decision {
    let quick_tap = Duration::from_millis(QUICK_TAP_MS);
    match (input, active) {
        (RecorderInput::Cancel, _) => Decision::Cancel,
        (RecorderInput::WorkerDone, _) => Decision::ShowNotice,

        (RecorderInput::Pressed, None) => {
            if since_auto_stop
                .is_some_and(|since| since < Duration::from_millis(AFTER_AUTO_STOP_IGNORE_MS))
            {
                Decision::Ignore("press right after an automatic stop")
            } else {
                Decision::Start
            }
        }
        (RecorderInput::Pressed, Some(active)) => {
            if active.elapsed < quick_tap {
                Decision::Ignore("second press too soon after the first")
            } else {
                // Hands-free: the second tap of tap-to-start, tap-to-stop. Still counted as
                // held: a new press can only follow a release, so that release was lost and
                // this press ends the recording.
                Decision::Finish
            }
        }

        (RecorderInput::Released, None) => Decision::Ignore("release with nothing recording"),
        (RecorderInput::Released, Some(active)) => {
            if !active.key_held {
                // The release of a press that was itself ignored.
                Decision::Ignore("release of an ignored press")
            } else if active.elapsed < quick_tap {
                Decision::HandsFree
            } else {
                Decision::Finish
            }
        }

        (RecorderInput::MeterStop { .. }, None) => {
            Decision::Ignore("meter stop with nothing recording")
        }
        (
            RecorderInput::MeterStop {
                reason,
                speech_seen,
            },
            Some(active),
        ) => {
            // The meter does not send silence stops while the key is held; this guards the
            // rule anyway. The maximum length always applies.
            if active.key_held && reason != StopReason::MaxLength {
                Decision::Ignore("silence while the hotkey is held")
            } else if speech_seen {
                Decision::Finish
            } else {
                Decision::Discard
            }
        }
    }
}

/// Whether a meter stop counts as an automatic stop for [`AFTER_AUTO_STOP_IGNORE_MS`].
pub(crate) fn is_auto_stop(input: RecorderInput, decision: Decision) -> bool {
    matches!(input, RecorderInput::MeterStop { .. })
        && matches!(decision, Decision::Finish | Decision::Discard)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    fn held(elapsed_ms: u64) -> Option<ActiveRecording> {
        Some(ActiveRecording {
            elapsed: ms(elapsed_ms),
            key_held: true,
        })
    }

    fn hands_free(elapsed_ms: u64) -> Option<ActiveRecording> {
        Some(ActiveRecording {
            elapsed: ms(elapsed_ms),
            key_held: false,
        })
    }

    fn stop(reason: StopReason) -> RecorderInput {
        RecorderInput::MeterStop {
            reason,
            speech_seen: true,
        }
    }

    #[test]
    fn a_press_starts_a_recording() {
        assert_eq!(decide(RecorderInput::Pressed, None, None), Decision::Start);
        assert_eq!(
            decide(RecorderInput::Pressed, None, Some(ms(5_000))),
            Decision::Start
        );
    }

    #[test]
    fn a_quick_release_goes_hands_free_and_a_late_one_ends_push_to_talk() {
        assert_eq!(
            decide(RecorderInput::Released, held(QUICK_TAP_MS - 1), None),
            Decision::HandsFree
        );
        assert_eq!(
            decide(RecorderInput::Released, held(QUICK_TAP_MS), None),
            Decision::Finish
        );
        assert_eq!(
            decide(RecorderInput::Released, held(30_000), None),
            Decision::Finish
        );
    }

    #[test]
    fn a_held_hotkey_is_never_stopped_by_silence() {
        assert_eq!(
            decide(stop(StopReason::EndOfSpeech), held(5_000), None),
            Decision::Ignore("silence while the hotkey is held")
        );
        assert_eq!(
            decide(
                RecorderInput::MeterStop {
                    reason: StopReason::NoSpeech,
                    speech_seen: false,
                },
                held(5_000),
                None
            ),
            Decision::Ignore("silence while the hotkey is held")
        );
        // The maximum length still ends it.
        assert_eq!(
            decide(stop(StopReason::MaxLength), held(120_000), None),
            Decision::Finish
        );
    }

    #[test]
    fn a_press_after_a_lost_release_finishes_the_recording() {
        // The release event never arrived, so the key still counts as held and silence
        // cannot stop it. The user presses again to stop: that must work.
        assert_eq!(
            decide(RecorderInput::Pressed, held(QUICK_TAP_MS), None),
            Decision::Finish
        );
        assert_eq!(
            decide(RecorderInput::Pressed, held(45_000), None),
            Decision::Finish
        );
        // Within the tap window it is still a bounce, not a stop.
        assert_eq!(
            decide(RecorderInput::Pressed, held(QUICK_TAP_MS - 1), None),
            Decision::Ignore("second press too soon after the first")
        );
    }

    #[test]
    fn a_reported_release_after_a_lost_one_still_ends_push_to_talk() {
        // The watch reports the missed release as an ordinary release.
        assert_eq!(
            decide(RecorderInput::Released, held(10_000), None),
            Decision::Finish
        );
        // A tap whose release was lost becomes hands-free, as a reported one would.
        assert_eq!(
            decide(RecorderInput::Released, held(QUICK_TAP_MS - 100), None),
            Decision::HandsFree
        );
    }

    #[test]
    fn the_watch_reports_a_key_that_stays_up() {
        let start = Instant::now();
        let mut watch = ReleaseWatch::default();

        assert!(!watch.observe(start, true));
        assert!(!watch.observe(start + ms(10), false));
        assert!(!watch.observe(start + ms(10 + MISSED_RELEASE_AFTER_MS - 1), false));
        assert!(watch.observe(start + ms(10 + MISSED_RELEASE_AFTER_MS), false));
        // Once per release.
        assert!(!watch.observe(start + ms(2_000), false));
    }

    #[test]
    fn the_watch_ignores_a_key_that_is_held_or_only_briefly_up() {
        let start = Instant::now();
        let mut watch = ReleaseWatch::default();

        for step in 0..100 {
            assert!(!watch.observe(start + ms(step * 30), true));
        }
        // Up for less than the threshold, then down again: no report, and the clock resets.
        assert!(!watch.observe(start + ms(3_000), false));
        assert!(!watch.observe(start + ms(3_200), false));
        assert!(!watch.observe(start + ms(3_250), true));
        assert!(!watch.observe(start + ms(3_300), false));
        assert!(!watch.observe(start + ms(3_550), false));
        assert!(watch.observe(start + ms(3_300 + MISSED_RELEASE_AFTER_MS), false));
    }

    #[test]
    fn hands_free_stops_on_silence_or_discards_an_abandoned_tap() {
        assert_eq!(
            decide(stop(StopReason::EndOfSpeech), hands_free(2_000), None),
            Decision::Finish
        );
        assert_eq!(
            decide(
                RecorderInput::MeterStop {
                    reason: StopReason::NoSpeech,
                    speech_seen: false,
                },
                hands_free(2_500),
                None
            ),
            Decision::Discard
        );
    }

    #[test]
    fn a_second_tap_stops_hands_free_but_not_too_soon() {
        assert_eq!(
            decide(RecorderInput::Pressed, hands_free(QUICK_TAP_MS), None),
            Decision::Finish
        );
        assert_eq!(
            decide(RecorderInput::Pressed, hands_free(QUICK_TAP_MS - 1), None),
            Decision::Ignore("second press too soon after the first")
        );
        // Its release then finds nothing recording.
        assert_eq!(
            decide(RecorderInput::Released, None, None),
            Decision::Ignore("release with nothing recording")
        );
        // The release of an ignored press leaves hands-free running.
        assert_eq!(
            decide(RecorderInput::Released, hands_free(600), None),
            Decision::Ignore("release of an ignored press")
        );
    }

    #[test]
    fn a_press_right_after_an_auto_stop_is_ignored() {
        // The user tapped to stop just as the silence stopped it: no new recording, so the
        // dictation being transcribed is not superseded.
        assert_eq!(
            decide(
                RecorderInput::Pressed,
                None,
                Some(ms(AFTER_AUTO_STOP_IGNORE_MS - 1))
            ),
            Decision::Ignore("press right after an automatic stop")
        );
        assert_eq!(
            decide(
                RecorderInput::Pressed,
                None,
                Some(ms(AFTER_AUTO_STOP_IGNORE_MS))
            ),
            Decision::Start
        );
    }

    #[test]
    fn only_meter_stops_count_as_automatic() {
        let finish = decide(stop(StopReason::EndOfSpeech), hands_free(2_000), None);
        assert!(is_auto_stop(stop(StopReason::EndOfSpeech), finish));
        assert!(!is_auto_stop(RecorderInput::Released, Decision::Finish));
        assert!(!is_auto_stop(RecorderInput::Pressed, Decision::Finish));
        let ignored = decide(stop(StopReason::EndOfSpeech), held(2_000), None);
        assert!(!is_auto_stop(stop(StopReason::EndOfSpeech), ignored));
    }

    #[test]
    fn cancel_always_cancels_and_workers_can_trigger_a_notice() {
        assert_eq!(decide(RecorderInput::Cancel, None, None), Decision::Cancel);
        assert_eq!(
            decide(RecorderInput::Cancel, held(100), None),
            Decision::Cancel
        );
        assert_eq!(
            decide(RecorderInput::WorkerDone, None, None),
            Decision::ShowNotice
        );
    }
}
