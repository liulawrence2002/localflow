//! Speaks a sequence of text pieces through a streaming voice into a speaker.
//!
//! Shared by conversation replies and by reading a page aloud. Pieces arrive on a channel so
//! the first can be heard while later ones are still being produced. Afterwards it reports
//! how many pieces the listener actually began to hear, which is what lets an interrupted
//! reply be remembered, and a page's reading position be advanced, only as far as it went.

use std::{sync::mpsc, thread, time::Duration};

use crate::{
    audio::{devices::AudioSink, resample::StreamResampler},
    cancel::CancelFlag,
    tts::{provider::TtsError, stream::StreamingTextToSpeech},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoken {
    /// Pieces that began playing, counted from the first.
    pub heard_pieces: usize,
    /// Pieces that played to their end, counted from the first. Never more than
    /// `heard_pieces`; fewer when playback was cut off part-way through a piece.
    pub finished_pieces: usize,
    /// Those pieces joined with single spaces.
    pub heard_text: String,
    /// Why the voice stopped, if it failed.
    pub error: Option<TtsError>,
}

/// Speak `pieces` in order until the channel closes, the voice fails, or `cancel` is set.
/// `on_first_audio` is called once, when the first audio has been queued. Returns after the
/// queued audio has played out, or at once when cancelled.
pub fn speak_pieces(
    voice: &dyn StreamingTextToSpeech,
    sink: &dyn AudioSink,
    pieces: mpsc::Receiver<String>,
    cancel: &CancelFlag,
    sentence_gap: Duration,
    on_first_audio: &mut dyn FnMut(),
) -> Spoken {
    let mut to_device = StreamResampler::new(voice.sample_rate(), sink.sample_rate());
    let gap = vec![0.0f32; (sentence_gap.as_secs_f64() * sink.sample_rate() as f64) as usize];
    let played_before = sink.played();
    let mut queued = 0u64;
    // Where in the queued audio each piece starts and, once all of its audio is queued, where
    // it ends: to work out afterwards what was heard, and what was heard to the end.
    let mut spans: Vec<(u64, Option<u64>, String)> = Vec::new();
    let mut scratch: Vec<f32> = Vec::new();
    let mut announced = false;
    let mut error = None;

    for piece in pieces {
        if cancel.is_cancelled() {
            break;
        }
        let start = queued;
        let result = voice.synthesize_stream(&piece, cancel, &mut |audio| {
            scratch.clear();
            to_device.process(audio, &mut scratch);
            queued += write_all(sink, &scratch, cancel);
            if !announced && !scratch.is_empty() {
                announced = true;
                on_first_audio();
            }
        });
        if let Err(failed) = result {
            spans.push((start, None, piece));
            error = Some(failed);
            break;
        }
        scratch.clear();
        to_device.flush(&mut scratch);
        queued += write_all(sink, &scratch, cancel);
        // A piece whose synthesis was cut short has no end: it can never count as finished.
        let end = (!cancel.is_cancelled()).then_some(queued);
        spans.push((start, end, piece));
        queued += write_all(sink, &gap, cancel);
    }

    if cancel.is_cancelled() {
        // Anything written in the instant between the stop and this thread noticing it.
        sink.flush();
    } else {
        // Let the queued speech play out before reporting what was heard.
        while sink.queued() > 0 && !cancel.is_cancelled() && sink.healthy() {
            thread::sleep(Duration::from_millis(10));
        }
    }

    let played = sink.played().saturating_sub(played_before);
    let heard: Vec<&str> = spans
        .iter()
        .filter(|(start, _, _)| *start < played)
        .map(|(_, _, piece)| piece.as_str())
        .collect();
    let finished_pieces = spans
        .iter()
        .take_while(|(_, end, _)| end.is_some_and(|end| end <= played))
        .count();
    Spoken {
        heard_pieces: heard.len(),
        finished_pieces,
        heard_text: heard.join(" "),
        error,
    }
}

/// Write all of `samples`, waiting while the speaker's queue is full. Returns how many
/// samples were written, which is fewer than asked only when cancelled or the device failed.
pub fn write_all(sink: &dyn AudioSink, samples: &[f32], cancel: &CancelFlag) -> u64 {
    let mut written = 0usize;
    while written < samples.len() {
        if cancel.is_cancelled() || !sink.healthy() {
            break;
        }
        let taken = sink.write(&samples[written..]);
        written += taken;
        if taken == 0 {
            thread::sleep(Duration::from_millis(5));
        }
    }
    written as u64
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::{audio::devices::mock::RecordingSpeaker, tts::stream::mock::MockVoice};

    fn pieces(texts: &[&str]) -> mpsc::Receiver<String> {
        let (tx, rx) = mpsc::channel();
        for text in texts {
            tx.send(text.to_string()).unwrap();
        }
        rx
    }

    #[test]
    fn every_piece_is_spoken_in_order_and_reported_as_heard() {
        let voice = MockVoice::default();
        let speaker = RecordingSpeaker::new(voice.sample_rate);
        let mut first_audio = 0;

        let spoken = speak_pieces(
            &voice,
            &speaker,
            pieces(&["One.", "Two.", "Three."]),
            &CancelFlag::default(),
            Duration::from_millis(10),
            &mut || first_audio += 1,
        );

        assert_eq!(spoken.heard_pieces, 3);
        assert_eq!(spoken.finished_pieces, 3);
        assert_eq!(spoken.heard_text, "One. Two. Three.");
        assert_eq!(spoken.error, None);
        assert_eq!(
            *voice.spoken.lock().unwrap(),
            vec!["One.", "Two.", "Three."]
        );
        assert_eq!(first_audio, 1, "first audio is announced exactly once");
    }

    #[test]
    fn pieces_still_queued_when_playback_is_cut_are_not_reported_as_heard() {
        let voice = MockVoice::default();
        let speaker = RecordingSpeaker::new(voice.sample_rate);
        // Nothing plays: audio piles up in the queue as on a real speaker.
        speaker.hold.store(true, Ordering::Relaxed);
        let cancel = CancelFlag::default();
        let mut cancel_on_audio = || cancel.cancel();

        let spoken = speak_pieces(
            &voice,
            &speaker,
            pieces(&["One.", "Two.", "Three."]),
            &cancel,
            Duration::ZERO,
            &mut cancel_on_audio,
        );

        assert_eq!(spoken.heard_pieces, 0);
        assert_eq!(spoken.finished_pieces, 0);
        assert_eq!(spoken.heard_text, "");
        assert!(speaker.flushes.load(Ordering::Relaxed) >= 1);
        assert_eq!(speaker.queued(), 0);
    }

    /// Plays everything written to it except the last `withheld` samples, as a speaker that
    /// was stopped just before the end would.
    struct NearlyPlayed {
        rate: u32,
        written: std::sync::atomic::AtomicU64,
        withheld: u64,
    }

    impl AudioSink for NearlyPlayed {
        fn sample_rate(&self) -> u32 {
            self.rate
        }

        fn write(&self, samples: &[f32]) -> usize {
            self.written
                .fetch_add(samples.len() as u64, Ordering::SeqCst);
            samples.len()
        }

        fn queued(&self) -> usize {
            0
        }

        fn played(&self) -> u64 {
            self.written
                .load(Ordering::SeqCst)
                .saturating_sub(self.withheld)
        }

        fn flush(&self) {}

        fn healthy(&self) -> bool {
            true
        }
    }

    #[test]
    fn a_piece_cut_off_before_its_end_was_heard_but_not_finished() {
        let voice = MockVoice::default();
        // Each piece is "One." = 4 characters of 160 samples, then a 10 ms gap of 160 samples.
        // Withholding more than the gap leaves the last piece's final samples unplayed.
        let speaker = NearlyPlayed {
            rate: voice.sample_rate,
            written: Default::default(),
            withheld: 400,
        };

        let spoken = speak_pieces(
            &voice,
            &speaker,
            pieces(&["One.", "Two."]),
            &CancelFlag::default(),
            Duration::from_millis(10),
            &mut || {},
        );

        assert_eq!(spoken.heard_pieces, 2, "both pieces began playing");
        assert_eq!(
            spoken.finished_pieces, 1,
            "the second was cut off before its end"
        );
    }

    #[test]
    fn a_voice_failure_stops_at_once_and_is_reported() {
        let voice = MockVoice {
            fail_with: Some(TtsError::Failed("voice stopped".to_string())),
            ..MockVoice::default()
        };
        let speaker = RecordingSpeaker::new(voice.sample_rate);

        let spoken = speak_pieces(
            &voice,
            &speaker,
            pieces(&["One.", "Two."]),
            &CancelFlag::default(),
            Duration::from_millis(10),
            &mut || {},
        );

        assert_eq!(
            spoken.error,
            Some(TtsError::Failed("voice stopped".to_string()))
        );
        assert_eq!(spoken.heard_pieces, 0);
        assert!(
            speaker.heard.lock().unwrap().is_empty(),
            "no stray silence after a failure"
        );
    }

    #[test]
    fn nothing_to_say_is_not_an_error() {
        let voice = MockVoice::default();
        let speaker = RecordingSpeaker::new(voice.sample_rate);

        let spoken = speak_pieces(
            &voice,
            &speaker,
            pieces(&[]),
            &CancelFlag::default(),
            Duration::from_millis(10),
            &mut || panic!("no audio expected"),
        );

        assert_eq!(
            spoken,
            Spoken {
                heard_pieces: 0,
                finished_pieces: 0,
                heard_text: String::new(),
                error: None
            }
        );
    }
}
