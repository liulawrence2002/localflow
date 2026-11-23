//! Scores short frames of microphone audio for "is this speech?".
//!
//! [`SpeechDetector`] is the boundary; [`super::turn_segmenter`] turns its probabilities into
//! turn boundaries. Two detectors exist:
//!
//! - [`SileroDetector`]: Silero VAD v6.2 run through the speech library that is already
//!   linked. It is a small neural network trained to tell speech from other sound, so a
//!   fan, a keyboard or music does not start a turn. Installed by `scripts/Get-VadModel.ps1`.
//! - [`EnergyDetector`]: loudness against an adaptive noise floor. No model needed, and
//!   fooled by any loud sound. The fallback when the Silero model is not installed.

use std::{path::Path, time::Duration};

use transcribe_cpp::{Model, VadSession};

/// Every detector takes mono audio at this rate.
pub const DETECTOR_SAMPLE_RATE: u32 = 16_000;
/// File name of the Silero model in the runtime `models` folder.
pub const SILERO_MODEL_FILE: &str = "silero-vad-v6.2-F32.gguf";

pub trait SpeechDetector: Send {
    /// Stable identifier for diagnostics, for example `silero`.
    fn id(&self) -> &'static str;

    /// Length of audio behind each probability.
    fn frame(&self) -> Duration;

    /// Add mono 16 kHz audio and append one probability in `[0, 1]` to `probabilities` for
    /// each frame completed. Audio short of a frame is kept for the next call.
    fn feed(&mut self, pcm: &[f32], probabilities: &mut Vec<f32>) -> Result<(), String>;

    /// Forget buffered audio and any internal state, ready for unrelated audio.
    fn reset(&mut self);
}

/// Samples per frame of the energy detector: 32 ms, the same as Silero.
const ENERGY_FRAME_SAMPLES: usize = 512;
/// Quietest level ever treated as speech, whatever the room. Roughly -42 dBFS.
const ENERGY_MIN_SPEECH_RMS: f32 = 0.008;
/// Speech must be this many times louder than the background.
const ENERGY_SPEECH_OVER_FLOOR: f32 = 3.0;
/// The background estimate is never allowed above this, so sustained speech cannot raise the
/// bar until it no longer counts as speech.
const ENERGY_MAX_FLOOR: f32 = 0.02;

/// Loudness against a slowly adapting estimate of the background level.
#[derive(Debug, Clone)]
pub struct EnergyDetector {
    pending: Vec<f32>,
    noise_floor: f32,
}

impl Default for EnergyDetector {
    fn default() -> Self {
        Self {
            pending: Vec::with_capacity(ENERGY_FRAME_SAMPLES * 2),
            noise_floor: ENERGY_MIN_SPEECH_RMS / ENERGY_SPEECH_OVER_FLOOR,
        }
    }
}

impl EnergyDetector {
    /// Probability for one frame at `rms`: 0.5 exactly at the speech threshold, rising to 1
    /// at twice the distance from the background.
    fn score(&mut self, rms: f32) -> f32 {
        let threshold = (self.noise_floor * ENERGY_SPEECH_OVER_FLOOR).max(ENERGY_MIN_SPEECH_RMS);
        let span = (threshold - self.noise_floor).max(f32::EPSILON);
        let probability = (0.5 * (rms - self.noise_floor) / span).clamp(0.0, 1.0);

        // Follow the background quickly while the sound is clearly not speech, and slowly while
        // it is borderline, so a steady fan that starts out above the threshold becomes the
        // background within a few seconds. Clear speech never moves the estimate.
        let follow = if probability < 0.35 {
            0.05
        } else if probability < 0.75 {
            0.02
        } else {
            0.0
        };
        self.noise_floor = ((1.0 - follow) * self.noise_floor + follow * rms).min(ENERGY_MAX_FLOOR);
        probability
    }
}

impl SpeechDetector for EnergyDetector {
    fn id(&self) -> &'static str {
        "energy"
    }

    fn frame(&self) -> Duration {
        Duration::from_micros(ENERGY_FRAME_SAMPLES as u64 * 1_000_000 / DETECTOR_SAMPLE_RATE as u64)
    }

    fn feed(&mut self, pcm: &[f32], probabilities: &mut Vec<f32>) -> Result<(), String> {
        self.pending.extend_from_slice(pcm);
        let mut consumed = 0;
        while self.pending.len() - consumed >= ENERGY_FRAME_SAMPLES {
            let frame = &self.pending[consumed..consumed + ENERGY_FRAME_SAMPLES];
            let rms = (frame.iter().map(|sample| sample * sample).sum::<f32>()
                / frame.len() as f32)
                .sqrt();
            consumed += ENERGY_FRAME_SAMPLES;
            let probability = self.score(rms);
            probabilities.push(probability);
        }
        self.pending.drain(..consumed);
        Ok(())
    }

    fn reset(&mut self) {
        self.pending.clear();
        // The background level is a property of the room, not of one turn: keep it.
    }
}

/// Silero VAD through `transcribe-cpp`.
pub struct SileroDetector {
    session: VadSession,
    frame: Duration,
    // The session keeps the native model alive itself; this is held so the model is not
    // loaded twice if more sessions are ever opened.
    _model: Model,
}

impl SileroDetector {
    /// Load the model at `model_path`. Errors name the kind of failure, not the path.
    pub fn load(model_path: &Path) -> Result<Self, String> {
        if !model_path.is_file() {
            return Err(
                "The voice-activity model is not installed. Run scripts\\Get-VadModel.ps1."
                    .to_string(),
            );
        }
        let model = Model::load(model_path).map_err(|error| {
            format!(
                "Could not load the voice-activity model: {:?} error",
                error.kind()
            )
        })?;
        let info = model.vad_info().map_err(|error| {
            format!(
                "The voice-activity model is not usable: {:?} error",
                error.kind()
            )
        })?;
        if info.sample_rate != DETECTOR_SAMPLE_RATE as i32 || info.frame_samples <= 0 {
            return Err("The voice-activity model expects a different audio format.".to_string());
        }
        let session = model.vad_session().map_err(|error| {
            format!(
                "Could not start voice-activity detection: {:?} error",
                error.kind()
            )
        })?;

        Ok(Self {
            session,
            frame: Duration::from_micros(
                info.frame_samples as u64 * 1_000_000 / DETECTOR_SAMPLE_RATE as u64,
            ),
            _model: model,
        })
    }
}

impl SpeechDetector for SileroDetector {
    fn id(&self) -> &'static str {
        "silero"
    }

    fn frame(&self) -> Duration {
        self.frame
    }

    fn feed(&mut self, pcm: &[f32], probabilities: &mut Vec<f32>) -> Result<(), String> {
        if pcm.is_empty() {
            return Ok(());
        }
        let result = self.session.stream_feed(pcm).map_err(|error| {
            format!("Voice-activity detection failed: {:?} error", error.kind())
        })?;
        probabilities.extend_from_slice(&result.probs);
        Ok(())
    }

    fn reset(&mut self) {
        self.session.stream_reset();
    }
}

/// The best detector available: Silero when its model is installed and loads, otherwise the
/// energy detector. Returns the detector and, when it fell back, why.
pub fn best_detector(silero_model: Option<&Path>) -> (Box<dyn SpeechDetector>, Option<String>) {
    match silero_model.map(SileroDetector::load) {
        Some(Ok(detector)) => (Box::new(detector), None),
        Some(Err(reason)) => (Box::new(EnergyDetector::default()), Some(reason)),
        None => (
            Box::new(EnergyDetector::default()),
            Some(
                "The voice-activity model is not installed. Run scripts\\Get-VadModel.ps1."
                    .to_string(),
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, samples: usize) -> Vec<f32> {
        (0..samples)
            .map(|index| amplitude * (index as f32 * 0.2).sin())
            .collect()
    }

    fn scores(detector: &mut dyn SpeechDetector, pcm: &[f32]) -> Vec<f32> {
        let mut probabilities = Vec::new();
        detector.feed(pcm, &mut probabilities).unwrap();
        probabilities
    }

    #[test]
    fn one_probability_per_complete_frame_however_the_audio_is_cut() {
        let audio = tone(0.1, ENERGY_FRAME_SAMPLES * 5 + 100);
        let whole = scores(&mut EnergyDetector::default(), &audio);

        let mut detector = EnergyDetector::default();
        let mut pieces = Vec::new();
        for chunk in audio.chunks(37) {
            detector.feed(chunk, &mut pieces).unwrap();
        }

        assert_eq!(whole.len(), 5);
        assert_eq!(pieces, whole);
    }

    #[test]
    fn silence_scores_low_and_clear_speech_level_scores_high() {
        let mut detector = EnergyDetector::default();

        let quiet = scores(&mut detector, &tone(0.0005, ENERGY_FRAME_SAMPLES * 20));
        let loud = scores(&mut detector, &tone(0.1, ENERGY_FRAME_SAMPLES * 20));

        assert!(quiet.iter().all(|probability| *probability < 0.35));
        assert!(loud.iter().all(|probability| *probability > 0.9));
    }

    #[test]
    fn steady_background_noise_raises_the_bar_so_it_is_not_taken_for_speech() {
        let mut detector = EnergyDetector::default();
        // A fan at a level that would count as speech in a silent room.
        let fan = tone(0.012, ENERGY_FRAME_SAMPLES);

        let mut last = 1.0;
        for _ in 0..400 {
            last = scores(&mut detector, &fan)[0];
        }
        // Once the level has been steady for a while it is the background.
        assert!(last < 0.35, "steady noise still scored {last}");

        // Speech clearly above that noise is still heard.
        assert!(scores(&mut detector, &tone(0.1, ENERGY_FRAME_SAMPLES))[0] > 0.9);
    }

    #[test]
    fn sustained_speech_does_not_become_the_background() {
        let mut detector = EnergyDetector::default();
        let speech = tone(0.1, ENERGY_FRAME_SAMPLES);

        for _ in 0..2_000 {
            assert!(scores(&mut detector, &speech)[0] > 0.9);
        }
    }

    #[test]
    fn a_missing_model_falls_back_to_the_energy_detector_and_says_why() {
        let (detector, reason) = best_detector(Some(Path::new("no-such-model.gguf")));
        assert_eq!(detector.id(), "energy");
        assert!(reason.unwrap().contains("Get-VadModel.ps1"));

        let (detector, reason) = best_detector(None);
        assert_eq!(detector.id(), "energy");
        assert!(reason.is_some());
    }

    /// Needs the real model: `scripts\Get-VadModel.ps1`, then
    /// `cargo test --release --lib real_silero -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs the Silero model in .localflow-runtime/models"]
    fn real_silero_scores_silence_low_and_frames_at_32_ms() {
        let model = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".localflow-runtime")
            .join("models")
            .join(SILERO_MODEL_FILE);
        let mut detector = SileroDetector::load(&model).expect("model should load");
        assert_eq!(detector.frame(), Duration::from_millis(32));

        // One second of silence, then one second of a steady hum: neither is speech.
        let silence = scores(&mut detector, &vec![0.0; 16_000]);
        let hum = scores(&mut detector, &tone(0.2, 16_000));

        assert_eq!(silence.len(), 31);
        assert!(silence.iter().all(|probability| *probability < 0.3));
        let hum_speech = hum
            .iter()
            .filter(|probability| **probability >= 0.5)
            .count();
        assert!(
            hum_speech <= hum.len() / 4,
            "a hum scored as speech in {hum_speech} frames"
        );
    }
}
