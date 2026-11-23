//! Removing the assistant's own voice from the microphone.
//!
//! On laptop speakers the microphone can hear the assistant as loudly as it hears the user,
//! so a voice detector fed the raw microphone fires on the assistant's speech. An echo
//! canceller is told what is being played, works out how that sound arrives back at the
//! microphone (how late, how loud, with which reflections) and subtracts it, leaving the
//! user's voice.
//!
//! [`Aec3EchoCanceller`] wraps the `sonora` crate, a pure-Rust port of the WebRTC "AEC3"
//! canceller: a delay estimator, an adaptive linear filter, and a suppressor for what the
//! filter cannot model (reverberation tail, speaker distortion). The wrapper adds what the
//! conversation code needs around it:
//!
//! - **Framing.** The engine works on 10 ms frames; callers pass whatever they have. Up to one
//!   frame of microphone audio is held back and comes out on a later call.
//! - **Pacing the reference.** The engine expects played and heard audio to arrive at the same
//!   rate. Audio passed to [`EchoCanceller::played`] waits in a queue here and is handed to
//!   the engine one frame per microphone frame, so it does not matter whether the caller
//!   reports playback every few milliseconds or in larger pieces. The remaining offset (device
//!   buffering, the path through the air) is the delay the engine estimates and tracks.
//! - **Standing aside when there is no echo.** Some audio drivers already remove the speaker
//!   from the microphone signal. What they leave behind is faint and bears no fixed relation
//!   to what was played, so the engine cannot model it; it then falls back on turning the
//!   microphone down whenever the assistant is loud, which removes the user too. When the
//!   microphone is found to be far quieter than the speaker (see [`EchoPresence`]), the
//!   engine is told nothing is playing and the microphone passes through.
//!
//! Two things are asked of the caller. `heard` must be given all microphone audio for as
//! long as anything is playing, or the reference queue and the microphone drift apart. And
//! in each pass of its loop it should report playback (`played`) before the microphone
//! (`heard`), so the reference is never behind the sound it describes.
//!
//! The cleaned audio comes out about 28 ms after it went in (20 ms held back on purpose, see
//! [`MICROPHONE_HELD_FRAMES`], and 8 ms inside the engine), plus up to 10 ms of framing.
//!
//! Nothing here touches the network or the disk, and no audio reaches a log.

use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
};

use sonora::{
    config::{EchoCanceller as Aec3Settings, EchoCanceller3Config},
    AudioProcessing, Config, StreamConfig,
};

/// Sample rate of everything passed to an echo canceller.
pub const ECHO_SAMPLE_RATE: u32 = 16_000;

/// The engine's frame: 10 ms.
const FRAME: usize = (ECHO_SAMPLE_RATE / 100) as usize;
/// Played audio kept waiting for the microphone to catch up. Longer than the speaker's own
/// queue (20 s), so reaching it means `heard` stopped being called; the oldest audio is
/// dropped then, since it has long since left the speaker.
const REFERENCE_SECONDS: usize = 30;
/// Frames after the last played audio during which echo may still be arriving: the longest
/// delay we expect (250 ms) plus the room's tail.
const ECHO_TAIL_FRAMES: u32 = 40;
/// Smoothing of the level meter per frame; about half a second.
const METER_SMOOTHING: f32 = 0.98;
/// Mean-square level (about -70 dBFS) above which a played frame counts as sound.
const PLAYING_MEAN_SQUARE: f32 = 1e-7;
/// Upper end of the reported reduction: beyond this the residual is far below any detector.
const MAX_REPORTED_REDUCTION_DB: f32 = 60.0;
/// How loud the engine assumes the echo is, relative to what is played, before it has
/// measured the real path. The engine's own default is 1 (as loud as the speaker), under
/// which it silences a user who speaks more quietly than the assistant whenever it has no
/// model of the path. A tenth still covers an ordinary laptop until the filter has settled,
/// about a second; much lower and echo leaks in that second.
const ASSUMED_ECHO_GAIN: f32 = 0.1;

/// Length of the stretch over which speaker and microphone levels are compared: 1 s.
const PRESENCE_WINDOW: usize = 100;
/// The newest frames of played audio are left out of the comparison, because their echo
/// may not have reached the microphone yet: 300 ms.
const PRESENCE_LAG: usize = 30;
/// Mean-square level (-50 dBFS) the played audio must reach for a comparison to count.
const PRESENCE_PLAYED_MEAN_SQUARE: f32 = 1e-5;
/// The microphone being this much quieter than the speaker means there is no echo worth
/// removing: speech played at a normal level then arrives below -60 dBFS, under any detector.
const ABSENT_LOSS_DB: f32 = 40.0;
/// Below this the echo counts as present again. The gap stops the two states alternating.
const PRESENT_LOSS_DB: f32 = 34.0;
/// How fast old evidence of a quiet microphone is forgotten: 1 dB per second of playback.
/// Slow enough to ride through the user talking over the assistant for several seconds,
/// fast enough to notice within a quarter of a minute that an echo has appeared.
const LOSS_FORGOTTEN_PER_FRAME_DB: f32 = 0.01;
/// How long the microphone is held back before it meets the reference: 20 ms. Playback is
/// reported after the fact, a few milliseconds late and in uneven pieces; without a head
/// start the report of a sound could reach the engine after the sound's own echo, which no
/// canceller can use.
const MICROPHONE_HELD_FRAMES: usize = 2;

pub trait EchoCanceller: Send {
    /// Stable identifier for diagnostics, e.g. "aec3" or "none".
    fn id(&self) -> &'static str;
    /// Audio that is being sent to the speaker now (mono, 16 kHz, any length). Call in playback order.
    fn played(&mut self, speaker: &[f32]);
    /// Microphone audio (mono, 16 kHz, any length). Appends the same audio with the speaker's
    /// contribution removed to `cleaned`. May hold back a few milliseconds for framing; what it
    /// holds back comes out on a later call, in order, with nothing lost or duplicated.
    fn heard(&mut self, microphone: &[f32], cleaned: &mut Vec<f32>);
    /// How strongly echo is currently being reduced, in dB (0 when unknown or nothing is playing).
    fn echo_reduction_db(&self) -> f32;
    /// Forget adaptation state (a new conversation, a device change).
    fn reset(&mut self);
}

/// Passes the microphone through untouched. Used with headphones and as the fallback.
pub struct NoEchoCancellation;

impl EchoCanceller for NoEchoCancellation {
    fn id(&self) -> &'static str {
        "none"
    }

    fn played(&mut self, _speaker: &[f32]) {}

    fn heard(&mut self, microphone: &[f32], cleaned: &mut Vec<f32>) {
        cleaned.extend_from_slice(microphone);
    }

    fn echo_reduction_db(&self) -> f32 {
        0.0
    }

    fn reset(&mut self) {}
}

/// The best canceller available. Never fails: returns `NoEchoCancellation` if nothing better can be built.
pub fn best_echo_canceller() -> Box<dyn EchoCanceller> {
    match Aec3EchoCanceller::new() {
        Some(canceller) => Box::new(canceller),
        None => Box::new(NoEchoCancellation),
    }
}

/// WebRTC's AEC3, through the pure-Rust `sonora` port.
pub struct Aec3EchoCanceller {
    engine: AudioProcessing,
    /// Played audio the engine has not been given yet, oldest first.
    reference: VecDeque<f32>,
    /// Microphone audio short of a full frame.
    pending: Vec<f32>,
    /// Whole microphone frames being held back, oldest first.
    held: VecDeque<f32>,
    meter: ReductionMeter,
    presence: EchoPresence,
    /// The engine misbehaved and could not be rebuilt; the microphone passes through.
    failed: bool,
}

impl Aec3EchoCanceller {
    /// `None` if the engine cannot be built.
    pub fn new() -> Option<Self> {
        // Room for the frame coming in on top of those held, so the queue never grows.
        let mut held = VecDeque::with_capacity((MICROPHONE_HELD_FRAMES + 1) * FRAME);
        held.resize(MICROPHONE_HELD_FRAMES * FRAME, 0.0);
        Some(Self {
            engine: build_engine()?,
            reference: VecDeque::with_capacity(REFERENCE_SECONDS * ECHO_SAMPLE_RATE as usize),
            pending: Vec::with_capacity(FRAME),
            held,
            meter: ReductionMeter::default(),
            presence: EchoPresence::default(),
            failed: false,
        })
    }

    /// The delay between `played` and the microphone as the engine currently has it, in
    /// milliseconds, to within a few milliseconds; `None` when nothing is playing or the
    /// engine has not found the echo.
    pub fn delay_ms(&self) -> Option<u32> {
        if self.failed || !self.meter.playing() {
            return None;
        }
        // The engine sees the microphone after its head start. Until it finds the echo it
        // reports its starting guess of 12 ms, which the subtraction turns into "unknown".
        let held_ms = (MICROPHONE_HELD_FRAMES * 10) as i32;
        self.engine
            .statistics()
            .delay_ms
            .map(|delay| delay - held_ms)
            .filter(|delay| *delay > 0)
            .map(|delay| delay as u32)
    }

    /// True while the microphone is passed through because it barely hears the speaker.
    pub fn microphone_already_clean(&self) -> bool {
        self.presence.absent
    }

    /// Run one 10 ms microphone frame, with the reference audio that goes with it.
    fn frame(&mut self, microphone: &[f32], cleaned: &mut Vec<f32>) {
        debug_assert_eq!(microphone.len(), FRAME);
        // One bad sample would otherwise live on in the adaptive filter for good.
        self.held
            .extend(microphone.iter().map(|sample| clean_sample(*sample)));
        let mut heard = [0.0f32; FRAME];
        for slot in &mut heard {
            *slot = self.held.pop_front().unwrap_or(0.0);
        }
        if self.failed {
            cleaned.extend_from_slice(&heard);
            return;
        }

        // Silence stands in when nothing is queued: the speaker plays silence then too.
        let mut played = [0.0f32; FRAME];
        for slot in &mut played {
            match self.reference.pop_front() {
                Some(sample) => *slot = sample,
                None => break,
            }
        }
        self.presence
            .frame(mean_square(&played), mean_square(&heard));
        if self.presence.absent {
            // With nothing to cancel the engine only does harm; told that nothing is playing
            // it passes the microphone through, with the same delay as before, so switching
            // makes no click.
            played = [0.0; FRAME];
        }

        let mut played_out = [0.0f32; FRAME];
        let mut output = [0.0f32; FRAME];
        let engine = &mut self.engine;
        let ran = catch_unwind(AssertUnwindSafe(|| {
            engine
                .process_render_f32(&[&played[..]], &mut [&mut played_out[..]])
                .is_ok()
                && engine
                    .process_capture_f32(&[&heard[..]], &mut [&mut output[..]])
                    .is_ok()
        }));

        if matches!(ran, Ok(true)) && output.iter().all(|sample| sample.is_finite()) {
            self.meter.frame(&played, &heard, &output);
            cleaned.extend_from_slice(&output);
            return;
        }

        // Hearing the assistant is better than hearing nothing: pass this frame through and
        // start again with a fresh engine, or without one if that fails too.
        tracing::warn!("echo canceller engine failed; rebuilding it");
        cleaned.extend_from_slice(&heard);
        self.meter = ReductionMeter::default();
        match build_engine() {
            Some(engine) => self.engine = engine,
            None => self.failed = true,
        }
    }
}

impl EchoCanceller for Aec3EchoCanceller {
    fn id(&self) -> &'static str {
        if self.failed {
            "none"
        } else {
            "aec3"
        }
    }

    fn played(&mut self, speaker: &[f32]) {
        let capacity = self.reference.capacity();
        // Only the newest audio can still matter when more arrives than the queue holds.
        let speaker = &speaker[speaker.len().saturating_sub(capacity)..];
        let overflow = (self.reference.len() + speaker.len()).saturating_sub(capacity);
        self.reference.drain(..overflow);
        self.reference
            .extend(speaker.iter().map(|sample| clean_sample(*sample)));
    }

    fn heard(&mut self, microphone: &[f32], cleaned: &mut Vec<f32>) {
        let mut rest = microphone;

        if !self.pending.is_empty() {
            let take = (FRAME - self.pending.len()).min(rest.len());
            self.pending.extend_from_slice(&rest[..take]);
            rest = &rest[take..];
            if self.pending.len() < FRAME {
                return;
            }
            let mut frame = [0.0f32; FRAME];
            frame.copy_from_slice(&self.pending);
            self.pending.clear();
            self.frame(&frame, cleaned);
        }

        let mut frames = rest.chunks_exact(FRAME);
        for frame in &mut frames {
            self.frame(frame, cleaned);
        }
        self.pending.extend_from_slice(frames.remainder());
    }

    fn echo_reduction_db(&self) -> f32 {
        if self.failed {
            0.0
        } else {
            self.meter.reduction_db()
        }
    }

    fn reset(&mut self) {
        // Microphone audio already taken in is not adaptation state: it still has to come out.
        self.reference.clear();
        self.meter = ReductionMeter::default();
        self.presence = EchoPresence::default();
        match build_engine() {
            Some(engine) => {
                self.engine = engine;
                self.failed = false;
            }
            None => self.failed = true,
        }
    }
}

fn build_engine() -> Option<AudioProcessing> {
    let config = Config {
        // Only the echo canceller: noise suppression and gain control would change the
        // user's voice on its way to the recogniser.
        echo_canceller: Some(Aec3Settings::default()),
        ..Default::default()
    };
    let mut tuning = EchoCanceller3Config::default();
    tuning.ep_strength.default_gain = ASSUMED_ECHO_GAIN;
    let stream = StreamConfig::new(ECHO_SAMPLE_RATE, 1);
    catch_unwind(|| {
        AudioProcessing::builder()
            .config(config)
            .echo_canceller3_config(tuning)
            .capture_config(stream)
            .render_config(stream)
            .build()
    })
    .ok()
}

fn clean_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

/// Decides whether the microphone hears the speaker at all.
///
/// Whatever the delay (up to 300 ms) and the room, sound that left the speaker during a
/// stretch of time has arrived at the microphone by the end of it. So if the microphone's
/// level over a second is far below the level of what was played in that second, the echo is
/// at most that faint. The user talking only makes the microphone louder, which hides the
/// evidence for a while but never fakes it, so the quietest comparison seen recently is kept.
#[derive(Debug)]
struct EchoPresence {
    /// Mean square of the last `PRESENCE_WINDOW` played and heard frames.
    played: [f32; PRESENCE_WINDOW],
    heard: [f32; PRESENCE_WINDOW],
    /// Where the next frame goes; once full this is also the oldest frame.
    next: usize,
    filled: usize,
    /// How much quieter than the speaker the microphone has recently been seen to be.
    loss_db: f32,
    absent: bool,
}

impl Default for EchoPresence {
    fn default() -> Self {
        Self {
            played: [0.0; PRESENCE_WINDOW],
            heard: [0.0; PRESENCE_WINDOW],
            next: 0,
            filled: 0,
            loss_db: 0.0,
            absent: false,
        }
    }
}

impl EchoPresence {
    fn frame(&mut self, played: f32, heard: f32) {
        self.played[self.next] = played;
        self.heard[self.next] = heard;
        self.next = (self.next + 1) % PRESENCE_WINDOW;
        self.filled = (self.filled + 1).min(PRESENCE_WINDOW);
        if self.filled < PRESENCE_WINDOW {
            return;
        }

        let compared = PRESENCE_WINDOW - PRESENCE_LAG;
        let played_level = (0..compared)
            .map(|age| self.played[(self.next + age) % PRESENCE_WINDOW])
            .sum::<f32>()
            / compared as f32;
        if played_level < PRESENCE_PLAYED_MEAN_SQUARE {
            return;
        }
        let heard_level = (self.heard.iter().sum::<f32>() / PRESENCE_WINDOW as f32).max(1e-12);
        let loss_db = 10.0 * (played_level / heard_level).log10();

        self.loss_db = loss_db.max(self.loss_db - LOSS_FORGOTTEN_PER_FRAME_DB);
        if self.loss_db >= ABSENT_LOSS_DB {
            self.absent = true;
        } else if self.loss_db < PRESENT_LOSS_DB {
            self.absent = false;
        }
    }
}

/// Compares the microphone's level before and after cancellation while audio is playing.
#[derive(Debug, Default)]
struct ReductionMeter {
    before: f32,
    after: f32,
    /// Frames left in which echo of the last played audio may still arrive.
    tail: u32,
}

impl ReductionMeter {
    fn frame(&mut self, played: &[f32], heard: &[f32], cleaned: &[f32]) {
        if mean_square(played) > PLAYING_MEAN_SQUARE {
            self.tail = ECHO_TAIL_FRAMES;
        } else if self.tail > 0 {
            self.tail -= 1;
            if self.tail == 0 {
                self.before = 0.0;
                self.after = 0.0;
            }
        }
        if self.tail > 0 {
            self.before += (1.0 - METER_SMOOTHING) * (mean_square(heard) - self.before);
            self.after += (1.0 - METER_SMOOTHING) * (mean_square(cleaned) - self.after);
        }
    }

    fn playing(&self) -> bool {
        self.tail > 0
    }

    fn reduction_db(&self) -> f32 {
        if !self.playing() || self.before <= 0.0 {
            return 0.0;
        }
        // The floor keeps a silent output from reading as infinite reduction.
        let ratio = self.before / self.after.max(self.before * 1e-6);
        (10.0 * ratio.log10()).clamp(0.0, MAX_REPORTED_REDUCTION_DB)
    }
}

fn mean_square(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32
}

#[cfg(test)]
mod tests {
    //! Everything here is synthetic and deterministic: a speech-like "assistant", a
    //! speech-like "user", and a simulated room between speaker and microphone.

    use super::*;

    const RATE: usize = ECHO_SAMPLE_RATE as usize;

    /// A small repeatable random source, so every run sees the same signals.
    struct Noise(u64);

    impl Noise {
        /// Uniform in [0, 1).
        fn unit(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            ((self.0 >> 40) as f32) / (1u64 << 24) as f32
        }

        /// Uniform in [-1, 1).
        fn signed(&mut self) -> f32 {
            2.0 * self.unit() - 1.0
        }

        fn between(&mut self, low: f32, high: f32) -> f32 {
            low + (high - low) * self.unit()
        }
    }

    /// A two-pole resonance, the building block of a vowel's formants.
    struct Resonator {
        a1: f32,
        a2: f32,
        gain: f32,
        y1: f32,
        y2: f32,
    }

    impl Resonator {
        fn new() -> Self {
            Self {
                a1: 0.0,
                a2: 0.0,
                gain: 1.0,
                y1: 0.0,
                y2: 0.0,
            }
        }

        fn tune(&mut self, frequency: f32, bandwidth: f32) {
            let radius = (-std::f32::consts::PI * bandwidth / RATE as f32).exp();
            let angle = 2.0 * std::f32::consts::PI * frequency / RATE as f32;
            self.a1 = 2.0 * radius * angle.cos();
            self.a2 = -radius * radius;
            self.gain = 1.0 - radius;
        }

        fn run(&mut self, input: f32) -> f32 {
            let output = self.gain * input + self.a1 * self.y1 + self.a2 * self.y2;
            self.y2 = self.y1;
            self.y1 = output;
            output
        }
    }

    /// Speech-like audio: syllables of a pitched buzz shaped by three moving formants, with
    /// some noisy consonants and pauses between phrases. `level` is the overall RMS.
    fn speech_like(seconds: f32, pitch: f32, seed: u64, level: f32) -> Vec<f32> {
        let total = (seconds * RATE as f32) as usize;
        let mut noise = Noise(seed);
        let mut formants = [Resonator::new(), Resonator::new(), Resonator::new()];
        let mut out = Vec::with_capacity(total);
        let mut phase = 0.0f32;
        let mut syllables_left = 3 + (noise.unit() * 5.0) as usize;

        while out.len() < total {
            if syllables_left == 0 {
                let pause = (noise.between(0.15, 0.4) * RATE as f32) as usize;
                out.extend(std::iter::repeat_n(0.0, pause));
                syllables_left = 3 + (noise.unit() * 5.0) as usize;
                continue;
            }
            syllables_left -= 1;

            let length = (noise.between(0.12, 0.3) * RATE as f32) as usize;
            let voiced = noise.unit() < 0.8;
            let loudness = noise.between(0.5, 1.0);
            let ramp = (0.03 * RATE as f32) as usize;
            if voiced {
                formants[0].tune(noise.between(300.0, 800.0), 90.0);
                formants[1].tune(noise.between(900.0, 2_300.0), 130.0);
                formants[2].tune(noise.between(2_400.0, 3_300.0), 180.0);
            } else {
                formants[0].tune(noise.between(2_500.0, 3_500.0), 600.0);
                formants[1].tune(noise.between(4_000.0, 5_000.0), 800.0);
                formants[2].tune(noise.between(5_500.0, 6_500.0), 900.0);
            }
            let start_pitch = pitch * noise.between(0.85, 1.2);
            let end_pitch = pitch * noise.between(0.85, 1.2);

            for index in 0..length {
                let position = index as f32 / length as f32;
                let envelope = (index.min(length - index) as f32 / ramp as f32).min(1.0);
                let source = if voiced {
                    phase += (start_pitch + (end_pitch - start_pitch) * position) / RATE as f32;
                    if phase >= 1.0 {
                        phase -= 1.0;
                    }
                    // A falling ramp per pitch period: a buzz rich in harmonics, like the voice.
                    (1.0 - 2.0 * phase) * 6.0
                } else {
                    noise.signed() * 1.5
                };
                let shaped = formants[0].run(source)
                    + 0.7 * formants[1].run(source)
                    + 0.4 * formants[2].run(source);
                out.push(shaped * envelope * loudness);
            }
        }
        out.truncate(total);

        let scale = level / rms(&out).max(1e-9);
        out.iter()
            .map(|sample| (sample * scale).clamp(-1.0, 1.0))
            .collect()
    }

    /// The path from the speaker, through the room, to the microphone.
    struct Room {
        /// Everything between `played` and the first arrival: device buffering plus air.
        delay: usize,
        /// Arrivals after the first: (extra samples, gain).
        reflections: Vec<(usize, f32)>,
        gain: f32,
        /// Drive into a soft clipper, mimicking a small speaker played loud.
        distortion: Option<f32>,
    }

    impl Room {
        fn new(delay_ms: usize, gain: f32) -> Self {
            let mut noise = Noise(0x5eed_0001);
            // 250 arrivals over 200 ms, fading by 60 dB in a quarter of a second.
            let reflections = (0..250)
                .map(|_| {
                    let offset = 8 + (noise.unit() * 0.2 * RATE as f32) as usize;
                    let seconds = offset as f32 / RATE as f32;
                    let decay = (-6.9 * seconds / 0.25).exp();
                    (offset, 0.25 * noise.signed() * decay)
                })
                .collect();
            Self {
                delay: delay_ms * RATE / 1_000,
                reflections,
                gain,
                distortion: None,
            }
        }

        fn distorted(mut self, drive: f32) -> Self {
            self.distortion = Some(drive);
            self
        }

        /// What the microphone picks up of `speaker`.
        fn echo(&self, speaker: &[f32]) -> Vec<f32> {
            // Small speakers have no bass: a gentle high-pass around 300 Hz.
            let coefficient = (-2.0 * std::f32::consts::PI * 300.0 / RATE as f32).exp();
            let mut low = 0.0f32;
            let driven: Vec<f32> = speaker
                .iter()
                .map(|sample| {
                    let sample = match self.distortion {
                        Some(drive) => (sample * drive).tanh() / drive,
                        None => *sample,
                    };
                    low = coefficient * low + (1.0 - coefficient) * sample;
                    sample - low
                })
                .collect();

            let mut out = vec![0.0f32; speaker.len()];
            for (index, slot) in out.iter_mut().enumerate() {
                if index < self.delay {
                    continue;
                }
                let at = index - self.delay;
                let mut sum = driven[at];
                for (offset, gain) in &self.reflections {
                    if at >= *offset {
                        sum += gain * driven[at - offset];
                    }
                }
                *slot = self.gain * sum;
            }
            out
        }
    }

    fn room_noise(length: usize, level: f32, seed: u64) -> Vec<f32> {
        let mut noise = Noise(seed);
        // Uniform noise in [-1, 1) has an RMS of 1/sqrt(3).
        (0..length)
            .map(|_| noise.signed() * level * 3f32.sqrt())
            .collect()
    }

    fn add(a: &[f32], b: &[f32]) -> Vec<f32> {
        a.iter().zip(b).map(|(a, b)| a + b).collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        mean_square(samples).sqrt()
    }

    fn db(ratio: f32) -> f32 {
        20.0 * ratio.max(1e-9).log10()
    }

    fn dbfs(samples: &[f32]) -> f32 {
        db(rms(samples))
    }

    /// How far the cleaned audio trails the microphone: the head start plus the engine's own
    /// 8 ms.
    const OUTPUT_DELAY: usize = MICROPHONE_HELD_FRAMES * FRAME + 128;

    /// Like [`run_untrimmed`], with the output shifted back so that sample `n` of the result
    /// is sample `n` of the microphone.
    fn run(
        canceller: &mut dyn EchoCanceller,
        speaker: &[f32],
        microphone: &[f32],
        chunk: usize,
    ) -> Vec<f32> {
        let mut cleaned = run_untrimmed(canceller, speaker, microphone, chunk);
        cleaned.drain(..OUTPUT_DELAY.min(cleaned.len()));
        cleaned
    }

    /// Feed `speaker` and `microphone` the way the conversation loop does, `chunk` samples at
    /// a time, and return everything that came out.
    fn run_untrimmed(
        canceller: &mut dyn EchoCanceller,
        speaker: &[f32],
        microphone: &[f32],
        chunk: usize,
    ) -> Vec<f32> {
        let mut cleaned = Vec::new();
        let mut at = 0;
        while at < microphone.len() {
            let end = (at + chunk).min(microphone.len());
            canceller.played(&speaker[at.min(speaker.len())..end.min(speaker.len())]);
            canceller.heard(&microphone[at..end], &mut cleaned);
            at = end;
        }
        cleaned
    }

    /// Echo reduction for each whole second: microphone level over cleaned level.
    fn reduction_by_second(microphone: &[f32], cleaned: &[f32]) -> Vec<f32> {
        microphone
            .chunks_exact(RATE)
            .zip(cleaned.chunks_exact(RATE))
            .map(|(before, after)| db(rms(before) / rms(after).max(1e-9)))
            .collect()
    }

    /// Seconds from `from` until the cleaned signal drops below `limit_dbfs` and stays there
    /// (measured in 100 ms windows) to the end of `cleaned`.
    fn settle_seconds(cleaned: &[f32], from: usize, limit_dbfs: f32) -> f32 {
        let window = RATE / 10;
        let mut settled_at = from;
        let mut at = from;
        while at + window <= cleaned.len() {
            if dbfs(&cleaned[at..at + window]) > limit_dbfs {
                settled_at = at + window;
            }
            at += window;
        }
        (settled_at - from) as f32 / RATE as f32
    }

    /// Level of the loudest 100 ms in `samples`: what a voice detector would react to.
    fn loudest_window_dbfs(samples: &[f32]) -> f32 {
        samples
            .chunks_exact(RATE / 10)
            .map(dbfs)
            .fold(f32::MIN, f32::max)
    }

    fn correlation(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(a, b)| a * b).sum();
        dot / (rms(a) * rms(b) * a.len().min(b.len()) as f32).max(1e-12)
    }

    /// What the canceller makes of `microphone` when nothing is playing. The engine's
    /// high-pass filter turns the phase of low frequencies, which the ear and a recogniser
    /// ignore but a sample-by-sample comparison does not; comparing against this instead of
    /// the raw signal leaves only the difference that playing audio makes.
    fn through_idle_canceller(microphone: &[f32]) -> Vec<f32> {
        run(&mut aec3(), &[], microphone, FRAME)
    }

    fn tone(frequency: f32, seconds: f32, amplitude: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|index| {
                amplitude
                    * (2.0 * std::f32::consts::PI * frequency * index as f32 / RATE as f32).sin()
            })
            .collect()
    }

    fn aec3() -> Aec3EchoCanceller {
        Aec3EchoCanceller::new().expect("the engine builds")
    }

    #[test]
    fn the_assistant_alone_is_removed_within_a_second() {
        let speaker = speech_like(10.0, 120.0, 1, 0.1);
        let noise = room_noise(speaker.len(), 0.001, 9);

        for (label, room) in [
            ("clean speaker", Room::new(120, 0.5)),
            (
                "mildly distorting speaker",
                Room::new(120, 0.5).distorted(2.0),
            ),
        ] {
            let microphone = add(&room.echo(&speaker), &noise);
            let mut canceller = aec3();
            let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);

            let by_second = reduction_by_second(&microphone, &cleaned);
            let after_first = db(rms(&microphone[RATE..]) / rms(&cleaned[RATE..]));
            let settle = settle_seconds(&cleaned, 0, -45.0);
            let loudest = loudest_window_dbfs(&cleaned[RATE..]);
            println!(
                "far end only, {label}: mic {:.1} dBFS, cleaned after 1 s {:.1} dBFS (loudest \
                 100 ms {loudest:.1} dBFS), reduction after 1 s {after_first:.1} dB, below \
                 -45 dBFS for good after {settle:.2} s, reported {:.1} dB, delay {:?} ms",
                dbfs(&microphone[RATE..]),
                dbfs(&cleaned[RATE..]),
                canceller.echo_reduction_db(),
                canceller.delay_ms(),
            );
            println!("  per second: {by_second:.1?}");

            assert!(after_first > 25.0, "{label}: only {after_first:.1} dB");
            assert!(loudest < -45.0, "{label}: {loudest:.1} dBFS got through");
            assert!(settle < 1.0, "{label}: settled after {settle:.2} s");
            assert!(canceller.echo_reduction_db() > 20.0, "{label}");
            assert!(!canceller.microphone_already_clean(), "{label}");
        }
    }

    #[test]
    fn a_badly_distorting_speaker_is_still_reduced_but_less() {
        // Hard clipping is beyond what a linear filter can model; the suppressor has to carry
        // it. This pins how much is lost, so a regression (or an improvement) shows up.
        let speaker = speech_like(10.0, 120.0, 1, 0.1);
        let microphone = add(
            &Room::new(120, 0.5).distorted(4.0).echo(&speaker),
            &room_noise(speaker.len(), 0.001, 9),
        );

        let mut canceller = aec3();
        let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);

        let after_first = db(rms(&microphone[RATE..]) / rms(&cleaned[RATE..]));
        let loudest = loudest_window_dbfs(&cleaned[RATE..]);
        println!(
            "far end only, badly distorting speaker: mic {:.1} dBFS, cleaned after 1 s {:.1} \
             dBFS (loudest 100 ms {loudest:.1} dBFS), reduction after 1 s {after_first:.1} dB",
            dbfs(&microphone[RATE..]),
            dbfs(&cleaned[RATE..]),
        );
        assert!(after_first > 15.0, "only {after_first:.1} dB");
        assert!(dbfs(&cleaned[RATE..]) < -45.0);
    }

    #[test]
    fn it_finds_the_echo_at_short_medium_and_long_delays() {
        let speaker = speech_like(8.0, 120.0, 2, 0.1);
        let noise = room_noise(speaker.len(), 0.001, 9);

        for delay_ms in [40usize, 120, 250] {
            let microphone = add(&Room::new(delay_ms, 0.5).echo(&speaker), &noise);
            let mut canceller = aec3();
            let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);

            let after_first = db(rms(&microphone[RATE..]) / rms(&cleaned[RATE..]));
            let settle = settle_seconds(&cleaned, 0, -45.0);
            let estimate = canceller.delay_ms();
            let loudest = loudest_window_dbfs(&cleaned[RATE..]);
            println!(
                "delay {delay_ms} ms: reduction after 1 s {after_first:.1} dB, cleaned \
                 {:.1} dBFS (loudest 100 ms {loudest:.1} dBFS), below -45 dBFS for good after \
                 {settle:.2} s, estimated delay {estimate:?} ms",
                dbfs(&cleaned[RATE..]),
            );

            assert!(after_first > 25.0, "{delay_ms} ms: {after_first:.1} dB");
            // The odd 100 ms around -40 dBFS gets through in the first few seconds; nothing
            // louder may.
            assert!(
                loudest < -38.0,
                "{delay_ms} ms: {loudest:.1} dBFS got through"
            );
            // The engine aligns a little early on purpose, to keep the echo inside its filter.
            let estimate = estimate.expect("a delay estimate while playing") as i64;
            assert!(
                (estimate - delay_ms as i64).abs() <= 24,
                "{delay_ms} ms estimated as {estimate} ms"
            );
        }
    }

    #[test]
    fn it_recovers_when_the_delay_changes_mid_stream() {
        let speaker = speech_like(14.0, 120.0, 3, 0.1);
        let noise = room_noise(speaker.len(), 0.001, 9);
        let change = 6 * RATE;

        for (before_ms, after_ms) in [(120usize, 200usize), (200, 80)] {
            let mut echo = Room::new(before_ms, 0.5).echo(&speaker);
            let later = Room::new(after_ms, 0.5).echo(&speaker);
            echo[change..].copy_from_slice(&later[change..]);
            let microphone = add(&echo, &noise);

            let mut canceller = aec3();
            let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);

            let recovery = settle_seconds(&cleaned, change, -45.0);
            let by_second = reduction_by_second(&microphone, &cleaned);
            let tail =
                db(rms(&microphone[change + 3 * RATE..]) / rms(&cleaned[change + 3 * RATE..]));
            println!(
                "delay {before_ms} -> {after_ms} ms at 6 s: below -45 dBFS again after \
                 {recovery:.2} s, reduction from 3 s after the change {tail:.1} dB, \
                 estimated delay {:?} ms",
                canceller.delay_ms()
            );
            println!("  per second: {by_second:.1?}");

            assert!(recovery < 3.0, "recovered after {recovery:.2} s");
            assert!(tail > 25.0, "only {tail:.1} dB after the change");
        }
    }

    #[test]
    fn the_user_is_still_heard_while_the_assistant_speaks() {
        let speaker = speech_like(12.0, 120.0, 4, 0.1);
        let echo = Room::new(120, 0.5).echo(&speaker);
        let noise = room_noise(speaker.len(), 0.001, 9);
        // The user starts talking four seconds in, well after the canceller has settled.
        let start = 4 * RATE;
        let mut user = vec![0.0f32; speaker.len()];
        user[start..].copy_from_slice(&speech_like(8.0, 210.0, 77, 0.06));
        let microphone = add(&add(&echo, &user), &noise);

        let mut canceller = aec3();
        let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);
        let user_alone = through_idle_canceller(&user);
        let raw = through_idle_canceller(&microphone);

        let range = start..speaker.len() - RATE;
        let wanted = &user_alone[range.clone()];
        let kept = &cleaned[range.clone()];
        let level_change = db(rms(kept) / rms(wanted));
        let similarity = correlation(kept, wanted);
        let raw_similarity = correlation(&raw[range.clone()], wanted);
        // What is left besides the user: the cleaned signal minus its best fit to the user.
        let fit = kept.iter().zip(wanted).map(|(a, b)| a * b).sum::<f32>()
            / wanted.iter().map(|b| b * b).sum::<f32>();
        let rest: Vec<f32> = kept.iter().zip(wanted).map(|(a, b)| a - fit * b).collect();
        let user_to_rest = db(fit * rms(wanted) / rms(&rest));
        let user_to_echo = db(rms(&user[range.clone()]) / rms(&echo[range.clone()]));
        println!(
            "double talk: user {:.1} dBFS over echo {:.1} dBFS; user level change \
             {level_change:.1} dB, correlation with the user alone {similarity:.3} (uncancelled \
             {raw_similarity:.3}), user-to-everything-else {user_to_rest:.1} dB (uncancelled \
             {user_to_echo:.1} dB), reported reduction {:.1} dB",
            dbfs(&user[range.clone()]),
            dbfs(&echo[range.clone()]),
            canceller.echo_reduction_db(),
        );

        assert!(
            level_change.abs() < 3.0,
            "user level changed by {level_change:.1} dB"
        );
        assert!(similarity > 0.9, "correlation {similarity:.3}");
        assert!(
            similarity > raw_similarity,
            "no better than the raw microphone"
        );
        assert!(
            user_to_rest > user_to_echo + 3.0,
            "{user_to_rest:.1} vs {user_to_echo:.1} dB"
        );
    }

    #[test]
    fn with_nothing_playing_the_microphone_passes_through() {
        let user = speech_like(6.0, 210.0, 5, 0.06);
        let microphone = add(&user, &room_noise(user.len(), 0.001, 9));

        let mut canceller = aec3();
        let cleaned = run(&mut canceller, &[], &microphone, FRAME);

        // The engine delays the signal: find by how much with a click.
        let mut click = vec![0.0f32; RATE];
        click[RATE / 2] = 0.5;
        let clicked = run_untrimmed(&mut aec3(), &[], &click, FRAME);
        let peak = (0..clicked.len())
            .max_by(|a, b| clicked[*a].abs().total_cmp(&clicked[*b].abs()))
            .unwrap();
        let delay = peak - RATE / 2;

        // Loudness over time, 10 ms at a time: the shape a voice detector follows.
        let levels =
            |samples: &[f32]| -> Vec<f32> { samples.chunks_exact(FRAME).map(rms).collect() };
        let before = levels(&microphone[RATE..user.len() - RATE]);
        let after = levels(&cleaned[RATE..user.len() - RATE]);
        let mean = |values: &[f32]| values.iter().sum::<f32>() / values.len() as f32;
        let centred = |values: &[f32]| -> Vec<f32> {
            let mean = mean(values);
            values.iter().map(|value| value - mean).collect()
        };
        let shape = correlation(&centred(&before), &centred(&after));
        let level_change = db(rms(&cleaned[RATE..]) / rms(&microphone[RATE..]));
        let first_second = db(rms(&cleaned[..RATE]) / rms(&microphone[..RATE]));
        println!(
            "nothing played: level change {level_change:.2} dB (first second \
             {first_second:.2} dB), loudness-over-time correlation {shape:.4}, engine delay \
             {delay} samples ({:.1} ms)",
            delay as f32 * 1_000.0 / RATE as f32
        );
        assert!(level_change.abs() < 1.0, "{level_change:.2} dB");
        assert!(first_second.abs() < 1.5, "{first_second:.2} dB");
        assert!(shape > 0.99, "loudness correlation {shape:.4}");
        assert_eq!(delay, OUTPUT_DELAY);
        assert_eq!(canceller.echo_reduction_db(), 0.0);
        assert_eq!(canceller.delay_ms(), None);

        // Every part of the speech band comes through at the level it went in.
        for frequency in [150.0, 300.0, 1_000.0, 3_000.0, 6_000.0, 7_500.0] {
            let input = tone(frequency, 2.0, 0.1);
            let output = through_idle_canceller(&input);
            let gain = db(rms(&output[RATE..]) / rms(&input[RATE..]));
            println!("  {frequency} Hz tone: {gain:.2} dB");
            assert!(gain.abs() < 1.0, "{frequency} Hz changed by {gain:.2} dB");
        }
    }

    #[test]
    fn the_output_does_not_depend_on_how_the_audio_is_cut_up() {
        let speaker = speech_like(4.0, 120.0, 6, 0.1);
        let microphone = add(
            &Room::new(90, 0.5).echo(&speaker),
            &room_noise(speaker.len(), 0.001, 9),
        );
        let reference = run_untrimmed(&mut aec3(), &speaker, &microphone, FRAME);

        for sizes in [&[1usize][..], &[7, 480, 3, 161], &[1_024], &[160, 159, 1]] {
            let mut canceller = aec3();
            let mut cleaned = Vec::new();
            let mut at = 0;
            let mut turn = 0;
            while at < microphone.len() {
                let end = (at + sizes[turn % sizes.len()]).min(microphone.len());
                canceller.played(&speaker[at..end]);
                canceller.heard(&microphone[at..end], &mut cleaned);
                at = end;
                turn += 1;
            }
            assert!(cleaned == reference, "{sizes:?} gave different audio");
        }

        // Queueing the whole reply ahead, as the speaker's queue is fed, changes nothing either.
        let mut canceller = aec3();
        let mut cleaned = Vec::new();
        canceller.played(&speaker);
        for chunk in microphone.chunks(333) {
            canceller.heard(chunk, &mut cleaned);
        }
        assert!(cleaned == reference, "queueing ahead gave different audio");
    }

    #[test]
    fn nothing_is_lost_or_added_over_a_long_run_and_memory_stays_put() {
        let speaker = speech_like(5.0, 120.0, 7, 0.1);
        let microphone = Room::new(60, 0.5).echo(&speaker);
        let mut canceller = aec3();
        let reference_capacity = canceller.reference.capacity();
        let mut cleaned = Vec::new();
        let mut given = 0usize;
        let mut noise = Noise(11);

        // Five minutes in uneven pieces, with the reply queued up to a second ahead.
        for _ in 0..60 {
            let mut at = 0;
            while at < microphone.len() {
                let end = (at + 1 + (noise.unit() * 700.0) as usize).min(microphone.len());
                let ahead = (end + RATE).min(speaker.len());
                canceller.played(&speaker[at.max(ahead.saturating_sub(end - at))..ahead]);
                cleaned.clear();
                canceller.heard(&microphone[at..end], &mut cleaned);
                given += end - at;
                // Everything given has come out, except less than one frame still held.
                let held = canceller.pending.len();
                assert!(held < FRAME);
                assert_eq!((given - held) % FRAME, 0);
                assert_eq!(cleaned.len() % FRAME, 0);
                at = end;
            }
        }

        assert_eq!(given, 60 * microphone.len());
        assert_eq!(canceller.reference.capacity(), reference_capacity);
        assert_eq!(canceller.pending.capacity(), FRAME);
        assert!(canceller.reference.len() <= reference_capacity);
    }

    #[test]
    fn every_sample_comes_out_exactly_once() {
        let microphone = speech_like(3.0, 180.0, 8, 0.05);
        let mut canceller = aec3();
        let mut cleaned = Vec::new();
        let mut given = 0;
        for chunk in microphone.chunks(77) {
            canceller.heard(chunk, &mut cleaned);
            given += chunk.len();
            assert_eq!(cleaned.len(), given / FRAME * FRAME);
        }
    }

    #[test]
    fn played_audio_that_is_never_heard_does_not_pile_up() {
        let mut canceller = aec3();
        let capacity = canceller.reference.capacity();
        let second = vec![0.1f32; RATE];

        for _ in 0..REFERENCE_SECONDS + 5 {
            canceller.played(&second);
        }
        canceller.played(&vec![0.2f32; capacity + 10]);

        assert_eq!(canceller.reference.len(), capacity);
        assert_eq!(canceller.reference.capacity(), capacity);
        assert!(canceller.reference.iter().all(|sample| *sample == 0.2));
    }

    #[test]
    fn reset_starts_again_from_nothing() {
        let mut canceller = aec3();
        let other = speech_like(4.0, 140.0, 13, 0.1);
        run(
            &mut canceller,
            &other,
            &Room::new(200, 0.3).echo(&other),
            FRAME,
        );
        assert!(canceller.delay_ms().is_some());
        canceller.played(&other);

        canceller.reset();

        assert_eq!(canceller.echo_reduction_db(), 0.0);
        assert_eq!(canceller.delay_ms(), None);
        assert!(canceller.reference.is_empty());
        assert!(!canceller.microphone_already_clean());

        // A different room is learned as quickly as by a new canceller, and what was queued
        // before the reset does not come back.
        let speaker = speech_like(4.0, 120.0, 12, 0.1);
        let microphone = Room::new(60, 0.5).echo(&speaker);
        let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);
        let fresh = run(&mut aec3(), &speaker, &microphone, FRAME);
        let again = db(rms(&microphone[RATE..]) / rms(&cleaned[RATE..]));
        let new = db(rms(&microphone[RATE..]) / rms(&fresh[RATE..]));
        println!("after a reset: {again:.1} dB after 1 s (a new canceller: {new:.1} dB)");
        assert!(
            (again - new).abs() < 3.0,
            "{again:.1} dB against {new:.1} dB"
        );
        let estimate = canceller.delay_ms().expect("a delay estimate") as i64;
        assert!((estimate - 60).abs() <= 24, "estimated {estimate} ms");
    }

    #[test]
    fn an_interrupted_reply_does_not_spoil_the_next_one() {
        let first = speech_like(6.0, 120.0, 14, 0.1);
        let second = speech_like(5.0, 120.0, 15, 0.1);
        let room = Room::new(120, 0.5);
        let stop = 4 * RATE;
        let gap = RATE / 2;

        // What really left the speaker: four seconds of the first reply, a pause, the second.
        let mut speaker = first[..stop].to_vec();
        speaker.extend(std::iter::repeat_n(0.0, gap));
        speaker.extend_from_slice(&second);
        let microphone = add(&room.echo(&speaker), &room_noise(speaker.len(), 0.001, 9));

        let mut canceller = aec3();
        let cleaned = run(&mut canceller, &speaker, &microphone, 3 * FRAME);

        let next = stop + gap;
        let first_second =
            db(rms(&microphone[next..next + RATE]) / rms(&cleaned[next..next + RATE]));
        let settle = settle_seconds(&cleaned, next, -45.0);
        println!(
            "after an interruption: next reply reduced by {first_second:.1} dB in its first \
             second, below -45 dBFS for good after {settle:.2} s"
        );
        assert!(first_second > 25.0, "only {first_second:.1} dB");
        assert!(settle < 0.3, "settled after {settle:.2} s");
    }

    #[test]
    fn bad_samples_do_not_poison_later_audio() {
        let speaker = speech_like(4.0, 120.0, 16, 0.1);
        let mut microphone = Room::new(80, 0.5).echo(&speaker);
        let mut bad_speaker = speaker.clone();
        microphone[5_000] = f32::NAN;
        microphone[5_001] = f32::INFINITY;
        bad_speaker[7_000] = f32::NAN;
        bad_speaker[7_001] = -40.0;

        let mut canceller = aec3();
        let cleaned = run(&mut canceller, &bad_speaker, &microphone, FRAME);

        assert!(cleaned.iter().all(|sample| sample.is_finite()));
        assert_eq!(canceller.id(), "aec3");
        let late = db(rms(&microphone[2 * RATE..]) / rms(&cleaned[2 * RATE..]));
        assert!(late > 25.0, "only {late:.1} dB after bad samples");
    }

    #[test]
    fn the_fallback_passes_audio_through_exactly() {
        let mut none = NoEchoCancellation;
        let mut cleaned = vec![9.0];
        none.played(&[0.5; 100]);
        none.heard(&[0.1, -0.2, 0.3], &mut cleaned);
        none.reset();

        assert_eq!(cleaned, vec![9.0, 0.1, -0.2, 0.3]);
        assert_eq!(none.id(), "none");
        assert_eq!(none.echo_reduction_db(), 0.0);
    }

    /// What a microphone delivers when the audio driver has already removed the speaker:
    /// faint leftovers that follow the loudness of what was played but not its waveform.
    fn driver_leftovers(speaker: &[f32], level: f32) -> Vec<f32> {
        let mut noise = Noise(0xfeed);
        let mut sign = 1.0;
        let delay = 20 * RATE / 1_000;
        let scrambled: Vec<f32> = (0..speaker.len())
            .map(|index| {
                if index % 80 == 0 {
                    sign = if noise.unit() < 0.5 { -1.0 } else { 1.0 };
                }
                if index < delay {
                    0.0
                } else {
                    sign * speaker[index - delay]
                }
            })
            .collect();
        let scale = level / rms(&scrambled);
        scrambled.iter().map(|sample| sample * scale).collect()
    }

    /// For each 100 ms in which the user is speaking, how much quieter (in dB) the user is in
    /// `cleaned` than in `wanted`, sorted from most lost to least.
    fn user_losses(wanted: &[f32], cleaned: &[f32]) -> Vec<f32> {
        let speaking = dbfs(wanted) - 10.0;
        let mut losses: Vec<f32> = wanted
            .chunks_exact(RATE / 10)
            .zip(cleaned.chunks_exact(RATE / 10))
            .filter(|(wanted, _)| dbfs(wanted) > speaking)
            .map(|(wanted, cleaned)| dbfs(cleaned) - dbfs(wanted))
            .collect();
        losses.sort_by(f32::total_cmp);
        losses
    }

    #[test]
    fn a_microphone_the_driver_already_cleaned_is_left_alone() {
        // Levels as measured on a laptop whose driver cancels echo itself: speech played at
        // -21 dBFS came back at about -67 dBFS, unrelated to the played waveform.
        let speaker = speech_like(12.0, 120.0, 31, 0.089);
        let leftovers = driver_leftovers(&speaker, 0.00045);
        let noise = room_noise(speaker.len(), 0.0001, 9);
        let start = 4 * RATE;

        for user_level in [0.1f32, 0.05, 0.025] {
            let mut user = vec![0.0f32; speaker.len()];
            user[start..].copy_from_slice(&speech_like(8.0, 210.0, 77, user_level));
            let microphone = add(&add(&leftovers, &user), &noise);

            let mut canceller = aec3();
            let cleaned = run(&mut canceller, &speaker, &microphone, FRAME);
            let user_alone = through_idle_canceller(&user);

            let losses = user_losses(&user_alone[start..], &cleaned[start..]);
            let kept = losses.iter().filter(|loss| **loss > -6.0).count();
            let similarity = correlation(&cleaned[start..], &user_alone[start..]);
            println!(
                "driver-cleaned microphone, user at {:.1} dBFS: {kept}/{} speaking windows \
                 within 6 dB, median {:.1} dB, worst tenth {:.1} dB, correlation with the \
                 user alone {similarity:.3}; assistant alone before that: cleaned {:.1} dBFS \
                 (loudest 100 ms {:.1})",
                dbfs(&user[start..]),
                losses.len(),
                losses[losses.len() / 2],
                losses[losses.len() / 10],
                dbfs(&cleaned[RATE..start]),
                loudest_window_dbfs(&cleaned[RATE..start]),
            );

            assert!(canceller.microphone_already_clean());
            assert_eq!(canceller.echo_reduction_db(), 0.0);
            assert_eq!(kept, losses.len(), "the user was turned down");
            assert!(similarity > 0.99, "correlation {similarity:.3}");
            assert!(loudest_window_dbfs(&cleaned[RATE..start]) < -55.0);
        }
    }

    #[test]
    fn a_real_echo_is_never_mistaken_for_a_clean_microphone() {
        // A quiet speaker in a loud room: echo 30 dB below what is played, the user on top.
        let speaker = speech_like(10.0, 120.0, 32, 0.1);
        let echo = Room::new(150, 0.03).echo(&speaker);
        let mut user = vec![0.0f32; speaker.len()];
        user[5 * RATE..].copy_from_slice(&speech_like(5.0, 210.0, 78, 0.05));
        let microphone = add(&add(&echo, &user), &room_noise(speaker.len(), 0.0003, 9));

        let mut canceller = aec3();
        let mut cleaned = Vec::new();
        for (played, heard) in speaker.chunks(FRAME).zip(microphone.chunks(FRAME)) {
            canceller.played(played);
            canceller.heard(heard, &mut cleaned);
            assert!(!canceller.microphone_already_clean());
        }

        let reduction = db(rms(&echo[RATE..5 * RATE]) / rms(&cleaned[RATE..5 * RATE]));
        println!(
            "faint real echo ({:.1} dBFS): reduced by {reduction:.1} dB, cleaned {:.1} dBFS",
            dbfs(&echo[RATE..5 * RATE]),
            dbfs(&cleaned[RATE..5 * RATE])
        );
        assert!(reduction > 10.0, "only {reduction:.1} dB");
    }

    #[test]
    fn playback_reported_late_and_in_bursts_still_cancels() {
        // The conversation loop learns what was played after it was played, a few
        // milliseconds late and in pieces that do not line up with the microphone's. The
        // report includes the silence before the reply. The room here answers within 15 ms,
        // less than the lateness of the reports.
        let mut speaker = vec![0.0f32; RATE];
        speaker.extend(speech_like(8.0, 120.0, 33, 0.1));
        let microphone = add(
            &Room::new(15, 0.5).echo(&speaker),
            &room_noise(speaker.len(), 0.001, 9),
        );
        let mut canceller = aec3();
        let mut cleaned = Vec::new();
        let mut noise = Noise(5);
        let mut reported = 0;
        let mut at = 0;
        while at < microphone.len() {
            let end = (at + 160 + (noise.unit() * 480.0) as usize).min(microphone.len());
            // Up to 10 ms of what has been played is not reported yet.
            let upto = end
                .saturating_sub((noise.unit() * 160.0) as usize)
                .max(reported);
            canceller.played(&speaker[reported..upto]);
            reported = upto;
            canceller.heard(&microphone[at..end], &mut cleaned);
            at = end;
        }

        let from = 2 * RATE;
        let after_first = db(rms(&microphone[from..cleaned.len()]) / rms(&cleaned[from..]));
        let loudest = loudest_window_dbfs(&cleaned[from..]);
        println!(
            "playback reported late and in bursts: reduction after 1 s {after_first:.1} dB, \
             loudest 100 ms after 1 s {loudest:.1} dBFS, reference queue {} samples",
            canceller.reference.len()
        );
        assert!(after_first > 25.0, "only {after_first:.1} dB");
        // A short burst around -40 dBFS in the first seconds is the engine's normal
        // behaviour, here and with tidy reports alike.
        assert!(loudest < -35.0, "{loudest:.1} dBFS got through");
        assert!(canceller.reference.len() < 4 * FRAME);
    }

    #[test]
    fn the_best_canceller_is_the_real_one() {
        let canceller = best_echo_canceller();
        assert_eq!(canceller.id(), "aec3");
        assert_eq!(canceller.echo_reduction_db(), 0.0);
    }

    /// Cost guard, run in release with `cargo test --release canceller_speed -- --ignored
    /// --nocapture`. One 10 ms frame must cost a small fraction of 10 ms.
    #[test]
    #[ignore = "timing test; run in release"]
    fn canceller_speed() {
        let speaker = speech_like(30.0, 120.0, 20, 0.1);
        let microphone = add(
            &Room::new(120, 0.5).echo(&speaker),
            &speech_like(30.0, 210.0, 21, 0.05),
        );
        let mut canceller = aec3();
        let mut cleaned = Vec::with_capacity(microphone.len());

        let started = std::time::Instant::now();
        for (played, heard) in speaker.chunks(FRAME).zip(microphone.chunks(FRAME)) {
            canceller.played(played);
            canceller.heard(heard, &mut cleaned);
        }
        let per_frame = started.elapsed().as_secs_f64() * 1e6 / (microphone.len() / FRAME) as f64;

        let quiet = vec![0.0f32; microphone.len()];
        let started = std::time::Instant::now();
        cleaned.clear();
        for heard in quiet.chunks(FRAME) {
            canceller.heard(heard, &mut cleaned);
        }
        let idle = started.elapsed().as_secs_f64() * 1e6 / (quiet.len() / FRAME) as f64;

        println!("{per_frame:.1} microseconds per 10 ms frame while playing, {idle:.1} idle");
        assert!(per_frame < 1_000.0, "{per_frame:.1} microseconds per frame");
    }
}
