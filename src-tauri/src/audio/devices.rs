//! Long-lived microphone and speaker streams for conversation mode.
//!
//! Dictation opens the microphone for one utterance and plays one finished clip. A
//! conversation keeps both devices open for minutes, feeds audio in as it is generated, and
//! must be able to fall silent at once. The two stream types here do that:
//!
//! - [`MicStream`] delivers mono microphone audio through a [`SampleRing`]. Its callback
//!   picks one channel and writes samples; it never allocates or locks.
//! - [`SpeakerStream`] plays whatever is in its ring and can be told to stop: the callback
//!   ramps the sound down over a few milliseconds (no click), discards what was queued, and
//!   reports back.
//!
//! `cpal` streams cannot move between threads on Windows, so each stream lives on a small
//! thread of its own that is parked until the owner is dropped.
//!
//! [`AudioSource`] and [`AudioSink`] are the boundaries the conversation code sees, so its
//! logic is tested with scripted audio instead of real devices.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, StreamConfig,
};

use super::ring::SampleRing;

/// Microphone audio waiting to be read: ten seconds is far more than a healthy reader lets
/// accumulate, so reaching it means the reader stalled.
const MIC_RING_SECONDS: usize = 10;
/// Speech queued for playback. A voice that generates faster than real time fills this and
/// then waits, which also stops it racing far ahead of what has been heard.
const SPEAKER_RING_SECONDS: usize = 20;
/// How much played audio is kept for the echo canceller to collect. It is read every few
/// milliseconds; two seconds only matters if the reader stalls.
const PLAYED_COPY_SECONDS: usize = 2;
/// Length of the fade when playback is told to stop.
const STOP_FADE: Duration = Duration::from_millis(6);
/// Headroom on synthesised speech so a loud passage never clips the device.
const PLAYBACK_GAIN: f32 = 0.89;
/// Channels examined when choosing which microphone channel to use.
const MAX_SELECT_CHANNELS: usize = 8;
/// Channel energy (mean square) above which the channel choice is locked. About -50 dBFS.
const CHANNEL_LOCK_MEAN_SQUARE: f32 = 0.003 * 0.003;

/// A source of mono microphone audio.
pub trait AudioSource: Send + Sync {
    fn sample_rate(&self) -> u32;
    /// Append everything captured since the last call to `out`.
    fn read(&self, out: &mut Vec<f32>);
    /// False once the device has reported an error; the stream will deliver nothing more.
    fn healthy(&self) -> bool;
}

/// Somewhere to play mono speech.
pub trait AudioSink: Send + Sync {
    fn sample_rate(&self) -> u32;
    /// Queue as much of `samples` as fits and return how many were taken.
    fn write(&self, samples: &[f32]) -> usize;
    /// Samples queued but not yet played.
    fn queued(&self) -> usize;
    /// Total samples played since the stream opened.
    fn played(&self) -> u64;
    /// Append what the device has been given to play since the last call, silence included,
    /// at [`AudioSink::sample_rate`]. This is the reference an echo canceller needs: the
    /// sound as it left for the speaker, in step with real time. The default reports
    /// nothing, for sinks that make no sound.
    fn take_played(&self, _out: &mut Vec<f32>) {}
    /// Stop now: fade out, discard everything queued, and return once that has happened.
    /// A pause is left as it is.
    fn flush(&self);
    fn healthy(&self) -> bool;
    /// Hold playback where it is: fade out and keep everything queued, so that
    /// [`AudioSink::resume`] carries on from the same place. Audio written meanwhile queues
    /// behind it. Held audio is not played, so it never counts in [`AudioSink::played`].
    /// The default does nothing, for sinks that cannot pause: they keep playing.
    fn pause(&self) {}
    /// Carry on playing after [`AudioSink::pause`], fading back in. The default does nothing.
    fn resume(&self) {}
}

struct MicShared {
    ring: SampleRing,
    failed: AtomicBool,
    /// Samples dropped because the reader was not keeping up.
    overruns: AtomicU64,
}

pub struct MicStream {
    shared: Arc<MicShared>,
    sample_rate: u32,
    device_name: String,
    /// Dropping this wakes the stream's thread, which then closes the device.
    _close: mpsc::Sender<()>,
}

impl MicStream {
    /// Open the default microphone and start capturing.
    pub fn open() -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(Arc<MicShared>, u32, String), String>>();
        let (close_tx, close_rx) = mpsc::channel::<()>();

        thread::Builder::new()
            .name("localflow-mic".to_string())
            .spawn(move || {
                let stream = match build_mic_stream() {
                    Ok((stream, shared, rate, name)) => {
                        let _ = ready_tx.send(Ok((shared, rate, name)));
                        stream
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                // Parked until the owner drops its sender; then the device is closed.
                let _ = close_rx.recv();
                drop(stream);
            })
            .map_err(|error| format!("Could not start the microphone thread: {error}"))?;

        let (shared, sample_rate, device_name) =
            ready_rx
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "The microphone did not open in time.".to_string())??;
        Ok(Self {
            shared,
            sample_rate,
            device_name,
            _close: close_tx,
        })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn overruns(&self) -> u64 {
        self.shared.overruns.load(Ordering::Relaxed)
    }
}

impl AudioSource for MicStream {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn read(&self, out: &mut Vec<f32>) {
        self.shared.ring.pop_all(out);
    }

    fn healthy(&self) -> bool {
        !self.shared.failed.load(Ordering::Relaxed)
    }
}

fn build_mic_stream() -> Result<(cpal::Stream, Arc<MicShared>, u32, String), String> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default microphone is available.".to_string())?;
    let name = device
        .name()
        .unwrap_or_else(|_| "Unknown input device".to_string());
    let supported = device
        .default_input_config()
        .map_err(|error| format!("Could not read the microphone format: {error}"))?;
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = config.channels as usize;

    let shared = Arc::new(MicShared {
        ring: SampleRing::new(sample_rate as usize * MIC_RING_SECONDS),
        failed: AtomicBool::new(false),
        overruns: AtomicU64::new(0),
    });

    let on_error = {
        let shared = shared.clone();
        move |error| {
            tracing::warn!(error = %error, "conversation microphone stream error");
            shared.failed.store(true, Ordering::Relaxed);
        }
    };

    let stream = match sample_format {
        SampleFormat::F32 => {
            let shared = shared.clone();
            let mut selector = MicChannel::default();
            device.build_input_stream(
                &config,
                move |data: &[f32], _| {
                    capture(data, channels, &mut selector, &shared, |sample| sample)
                },
                on_error,
                None,
            )
        }
        SampleFormat::I16 => {
            let shared = shared.clone();
            let mut selector = MicChannel::default();
            device.build_input_stream(
                &config,
                move |data: &[i16], _| {
                    capture(data, channels, &mut selector, &shared, |sample| {
                        sample as f32 / i16::MAX as f32
                    })
                },
                on_error,
                None,
            )
        }
        SampleFormat::U16 => {
            let shared = shared.clone();
            let mut selector = MicChannel::default();
            device.build_input_stream(
                &config,
                move |data: &[u16], _| {
                    capture(data, channels, &mut selector, &shared, |sample| {
                        (sample as f32 - 32768.0) / 32768.0
                    })
                },
                on_error,
                None,
            )
        }
        other => return Err(format!("Unsupported microphone sample format: {other:?}")),
    }
    .map_err(|error| format!("Could not open the microphone: {error}"))?;

    stream
        .play()
        .map_err(|error| format!("Could not start the microphone: {error}"))?;
    Ok((stream, shared, sample_rate, name))
}

/// Which channel of a multi-channel microphone to use.
///
/// Channels are never averaged, because array microphones can deliver phase-inverted
/// channels that cancel to silence. Until a channel carries real signal the loudest one of
/// each callback is used; after that the choice is locked, so it cannot flip mid-word.
#[derive(Debug, Default)]
struct MicChannel {
    locked: Option<usize>,
}

impl MicChannel {
    fn choose<T: Copy>(
        &mut self,
        data: &[T],
        channels: usize,
        convert: impl Fn(T) -> f32,
    ) -> usize {
        if channels <= 1 {
            return 0;
        }
        if let Some(channel) = self.locked {
            if channel < channels {
                return channel;
            }
        }

        // Fixed-size scratch: nothing is allocated on the audio thread.
        let considered = channels.min(MAX_SELECT_CHANNELS);
        let mut energy = [0.0f32; MAX_SELECT_CHANNELS];
        let mut frames = 0usize;
        for frame in data.chunks_exact(channels) {
            for (channel, slot) in energy.iter_mut().enumerate().take(considered) {
                let sample = convert(frame[channel]);
                *slot += sample * sample;
            }
            frames += 1;
        }
        let mut best = 0;
        for channel in 1..considered {
            if energy[channel] > energy[best] {
                best = channel;
            }
        }
        if frames > 0 && energy[best] / frames as f32 >= CHANNEL_LOCK_MEAN_SQUARE {
            self.locked = Some(best);
        }
        best
    }
}

/// The microphone callback body: write one channel of `data` into the ring.
fn capture<T: Copy>(
    data: &[T],
    channels: usize,
    selector: &mut MicChannel,
    shared: &MicShared,
    convert: impl Fn(T) -> f32 + Copy,
) {
    if channels == 0 {
        return;
    }
    let channel = selector.choose(data, channels, convert);
    let mut dropped = 0u64;
    for frame in data.chunks_exact(channels) {
        if !shared.ring.push(convert(frame[channel]).clamp(-1.0, 1.0)) {
            dropped += 1;
        }
    }
    if dropped > 0 {
        shared.overruns.fetch_add(dropped, Ordering::Relaxed);
    }
}

struct SpeakerShared {
    ring: SampleRing,
    /// A copy of every sample sent to the device, for echo cancellation. When nobody is
    /// reading it, it fills and further samples are dropped, which costs nothing.
    played_copy: SampleRing,
    failed: AtomicBool,
    played: AtomicU64,
    /// Incremented by [`AudioSink::flush`]; the callback fades out and discards when it sees
    /// a value it has not handled yet.
    flush_requested: AtomicU32,
    /// The last flush request the callback has completed.
    flush_done: AtomicU32,
    /// Set by [`AudioSink::pause`]: the callback fades out, then plays silence and leaves the
    /// queue alone until it is cleared.
    paused: AtomicBool,
}

pub struct SpeakerStream {
    shared: Arc<SpeakerShared>,
    sample_rate: u32,
    _close: mpsc::Sender<()>,
}

impl SpeakerStream {
    /// Open the default output device. Nothing is heard until audio is written.
    pub fn open() -> Result<Self, String> {
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(Arc<SpeakerShared>, u32), String>>();
        let (close_tx, close_rx) = mpsc::channel::<()>();

        thread::Builder::new()
            .name("localflow-speaker".to_string())
            .spawn(move || {
                let stream = match build_speaker_stream() {
                    Ok((stream, shared, rate)) => {
                        let _ = ready_tx.send(Ok((shared, rate)));
                        stream
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let _ = close_rx.recv();
                drop(stream);
            })
            .map_err(|error| format!("Could not start the speaker thread: {error}"))?;

        let (shared, sample_rate) = ready_rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "The speaker did not open in time.".to_string())??;
        Ok(Self {
            shared,
            sample_rate,
            _close: close_tx,
        })
    }
}

impl AudioSink for SpeakerStream {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn write(&self, samples: &[f32]) -> usize {
        self.shared.ring.push_slice(samples)
    }

    fn queued(&self) -> usize {
        self.shared.ring.len()
    }

    fn played(&self) -> u64 {
        self.shared.played.load(Ordering::Relaxed)
    }

    fn take_played(&self, out: &mut Vec<f32>) {
        self.shared.played_copy.pop_all(out);
    }

    fn flush(&self) {
        let request = self
            .shared
            .flush_requested
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        // The callback runs every few milliseconds. Wait for it to finish the fade, but not
        // forever: a device that has stopped calling back cannot be making sound either.
        let deadline = Instant::now() + Duration::from_millis(250);
        while self.shared.flush_done.load(Ordering::SeqCst) != request && Instant::now() < deadline
        {
            if !self.healthy() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        if self.shared.flush_done.load(Ordering::SeqCst) != request {
            // The callback is not running, so nothing else is reading the ring right now.
            self.shared.ring.discard_all();
            self.shared.flush_done.store(request, Ordering::SeqCst);
        }
    }

    fn healthy(&self) -> bool {
        !self.shared.failed.load(Ordering::Relaxed)
    }

    fn pause(&self) {
        self.shared.paused.store(true, Ordering::SeqCst);
    }

    fn resume(&self) {
        self.shared.paused.store(false, Ordering::SeqCst);
    }
}

/// State owned by the speaker callback between calls.
struct Renderer {
    shared: Arc<SpeakerShared>,
    /// Gain on the queued speech: 1 while playing, 0 while paused, in between while fading
    /// out for a stop or a pause, or back in after a pause.
    gain: f32,
    fade_step: f32,
    /// The flush request being faded out, if any.
    fading: Option<u32>,
}

impl Renderer {
    fn new(shared: Arc<SpeakerShared>, sample_rate: u32) -> Self {
        let fade_samples = (sample_rate as f32 * STOP_FADE.as_secs_f32()).max(1.0);
        Self {
            shared,
            gain: 1.0,
            fade_step: 1.0 / fade_samples,
            fading: None,
        }
    }

    /// Take the next queued sample, counting it as played, at the current gain.
    fn play_next(&mut self) -> Option<f32> {
        let sample = self.shared.ring.pop()?;
        self.shared.played.fetch_add(1, Ordering::Relaxed);
        Some(sample * PLAYBACK_GAIN * self.gain)
    }

    /// The next mono sample to play.
    fn next(&mut self) -> f32 {
        if self.fading.is_none() {
            let requested = self.shared.flush_requested.load(Ordering::Relaxed);
            if requested != self.shared.flush_done.load(Ordering::Relaxed) {
                self.fading = Some(requested);
            }
        }

        if let Some(request) = self.fading {
            self.gain -= self.fade_step;
            let sample = if self.gain > 0.0 {
                self.play_next()
            } else {
                None
            };
            return match sample {
                Some(sample) => sample,
                None => {
                    // Faded to silence, or nothing was playing: drop the rest and report.
                    self.shared.ring.discard_all();
                    // A paused speaker stays silent; otherwise the next audio plays at once.
                    self.gain = if self.shared.paused.load(Ordering::Relaxed) {
                        0.0
                    } else {
                        1.0
                    };
                    self.fading = None;
                    self.shared.flush_done.store(request, Ordering::SeqCst);
                    0.0
                }
            };
        }

        if self.shared.paused.load(Ordering::Relaxed) {
            // Fade out, then hold: nothing more is taken from the queue, so nothing more
            // counts as played, and resuming carries on from exactly here.
            self.gain = (self.gain - self.fade_step).max(0.0);
            if self.gain <= 0.0 {
                return 0.0;
            }
            return self.play_next().unwrap_or(0.0);
        }

        if self.gain < 1.0 && !self.shared.ring.is_empty() {
            // Back from a pause: fade in rather than jump.
            self.gain = (self.gain + self.fade_step).min(1.0);
        }
        match self.play_next() {
            Some(sample) => sample,
            None => {
                // Nothing to play: whatever comes next starts at full level, as a reply does.
                self.gain = 1.0;
                0.0
            }
        }
    }

    /// Fill one interleaved device buffer, the same mono signal on every channel.
    fn fill<T: Copy>(&mut self, output: &mut [T], channels: usize, convert: impl Fn(f32) -> T) {
        for frame in output.chunks_mut(channels.max(1)) {
            let sample = self.next().clamp(-1.0, 1.0);
            // Lock-free and allocation-free, like everything else on this thread.
            self.shared.played_copy.push(sample);
            frame.fill(convert(sample));
        }
    }
}

fn build_speaker_stream() -> Result<(cpal::Stream, Arc<SpeakerShared>, u32), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "No default speaker is available.".to_string())?;
    let supported = device
        .default_output_config()
        .map_err(|error| format!("Could not read the speaker format: {error}"))?;
    let sample_rate = supported.sample_rate().0;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let channels = config.channels as usize;

    let shared = Arc::new(SpeakerShared {
        ring: SampleRing::new(sample_rate as usize * SPEAKER_RING_SECONDS),
        played_copy: SampleRing::new(sample_rate as usize * PLAYED_COPY_SECONDS),
        failed: AtomicBool::new(false),
        played: AtomicU64::new(0),
        flush_requested: AtomicU32::new(0),
        flush_done: AtomicU32::new(0),
        paused: AtomicBool::new(false),
    });

    let on_error = {
        let shared = shared.clone();
        move |error| {
            tracing::warn!(error = %error, "conversation speaker stream error");
            shared.failed.store(true, Ordering::Relaxed);
        }
    };

    let mut renderer = Renderer::new(shared.clone(), sample_rate);
    let stream = match sample_format {
        SampleFormat::F32 => device.build_output_stream(
            &config,
            move |output: &mut [f32], _| renderer.fill(output, channels, |sample| sample),
            on_error,
            None,
        ),
        SampleFormat::I16 => device.build_output_stream(
            &config,
            move |output: &mut [i16], _| {
                renderer.fill(output, channels, |sample| (sample * i16::MAX as f32) as i16)
            },
            on_error,
            None,
        ),
        SampleFormat::U16 => device.build_output_stream(
            &config,
            move |output: &mut [u16], _| {
                renderer.fill(output, channels, |sample| {
                    ((sample * 32767.0) + 32768.0) as u16
                })
            },
            on_error,
            None,
        ),
        other => return Err(format!("Unsupported speaker sample format: {other:?}")),
    }
    .map_err(|error| format!("Could not open the speaker: {error}"))?;

    stream
        .play()
        .map_err(|error| format!("Could not start the speaker: {error}"))?;
    Ok((stream, shared, sample_rate))
}

#[cfg(test)]
pub mod mock {
    //! Scripted devices for tests: a microphone that plays back prepared audio and a speaker
    //! that records what it was given.

    use std::sync::Mutex;

    use super::*;

    /// A microphone fed by the test.
    pub struct ScriptedMic {
        pub rate: u32,
        pending: Mutex<Vec<f32>>,
        pub healthy: AtomicBool,
    }

    impl ScriptedMic {
        pub fn new(rate: u32) -> Self {
            Self {
                rate,
                pending: Mutex::new(Vec::new()),
                healthy: AtomicBool::new(true),
            }
        }

        /// Make `samples` available to the next read.
        pub fn deliver(&self, samples: &[f32]) {
            self.pending.lock().unwrap().extend_from_slice(samples);
        }
    }

    impl AudioSource for ScriptedMic {
        fn sample_rate(&self) -> u32 {
            self.rate
        }

        fn read(&self, out: &mut Vec<f32>) {
            out.append(&mut self.pending.lock().unwrap());
        }

        fn healthy(&self) -> bool {
            self.healthy.load(Ordering::Relaxed)
        }
    }

    /// A speaker that "plays" instantly unless told to hold audio in its queue, or paused.
    pub struct RecordingSpeaker {
        pub rate: u32,
        /// Everything written and not flushed away, in order.
        pub heard: Mutex<Vec<f32>>,
        /// When true, written audio stays queued (as if playback were slow) until released.
        pub hold: AtomicBool,
        queue: Mutex<Vec<f32>>,
        pub flushes: AtomicU32,
        paused: AtomicBool,
        pub pauses: AtomicU32,
        pub resumes: AtomicU32,
    }

    impl RecordingSpeaker {
        pub fn new(rate: u32) -> Self {
            Self {
                rate,
                heard: Mutex::new(Vec::new()),
                hold: AtomicBool::new(false),
                queue: Mutex::new(Vec::new()),
                flushes: AtomicU32::new(0),
                paused: AtomicBool::new(false),
                pauses: AtomicU32::new(0),
                resumes: AtomicU32::new(0),
            }
        }

        /// Stop holding audio back as slow playback; play the queue unless paused.
        pub fn release(&self) {
            self.hold.store(false, Ordering::SeqCst);
            self.play_queue();
        }

        pub fn is_paused(&self) -> bool {
            self.paused.load(Ordering::SeqCst)
        }

        fn play_queue(&self) {
            let mut queue = self.queue.lock().unwrap();
            if !self.hold.load(Ordering::SeqCst) && !self.paused.load(Ordering::SeqCst) {
                self.heard.lock().unwrap().append(&mut queue);
            }
        }
    }

    impl AudioSink for RecordingSpeaker {
        fn sample_rate(&self) -> u32 {
            self.rate
        }

        fn write(&self, samples: &[f32]) -> usize {
            let mut queue = self.queue.lock().unwrap();
            if self.hold.load(Ordering::SeqCst) || self.paused.load(Ordering::SeqCst) {
                queue.extend_from_slice(samples);
            } else {
                // Anything still queued plays first, so the order is kept.
                let mut heard = self.heard.lock().unwrap();
                heard.append(&mut queue);
                heard.extend_from_slice(samples);
            }
            samples.len()
        }

        fn pause(&self) {
            self.pauses.fetch_add(1, Ordering::SeqCst);
            self.paused.store(true, Ordering::SeqCst);
        }

        fn resume(&self) {
            self.resumes.fetch_add(1, Ordering::SeqCst);
            self.paused.store(false, Ordering::SeqCst);
            self.play_queue();
        }

        fn queued(&self) -> usize {
            self.queue.lock().unwrap().len()
        }

        fn played(&self) -> u64 {
            self.heard.lock().unwrap().len() as u64
        }

        fn flush(&self) {
            self.flushes.fetch_add(1, Ordering::Relaxed);
            self.queue.lock().unwrap().clear();
        }

        fn healthy(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speaker(rate: u32) -> (Renderer, Arc<SpeakerShared>) {
        let shared = Arc::new(SpeakerShared {
            ring: SampleRing::new(rate as usize),
            played_copy: SampleRing::new(rate as usize),
            failed: AtomicBool::new(false),
            played: AtomicU64::new(0),
            flush_requested: AtomicU32::new(0),
            flush_done: AtomicU32::new(0),
            paused: AtomicBool::new(false),
        });
        (Renderer::new(shared.clone(), rate), shared)
    }

    fn render(renderer: &mut Renderer, samples: usize) -> Vec<f32> {
        let mut output = vec![0.0f32; samples];
        renderer.fill(&mut output, 1, |sample| sample);
        output
    }

    #[test]
    fn a_pause_fades_out_and_keeps_the_rest_queued_without_counting_it_as_played() {
        let rate = 48_000;
        let (mut renderer, shared) = speaker(rate);
        shared.ring.push_slice(&vec![0.5; rate as usize / 2]);
        let before = render(&mut renderer, 480);

        shared.paused.store(true, Ordering::SeqCst);
        let during = render(&mut renderer, 960);

        // The level falls steadily to silence within the fade, as for a stop.
        let mut previous = *before.last().unwrap();
        for sample in &during {
            assert!(*sample <= previous + 1e-6, "level rose during the fade");
            assert!(previous - *sample < 0.02, "level dropped abruptly");
            previous = *sample;
        }
        let fade_samples = (rate as f32 * STOP_FADE.as_secs_f32()) as usize;
        assert!(during[fade_samples + 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        // Only what played before and during the fade counts; the rest waits in the queue.
        let played = shared.played.load(Ordering::Relaxed);
        assert!(played <= 480 + fade_samples as u64 + 1, "{played}");
        assert_eq!(shared.ring.len() as u64, rate as u64 / 2 - played);

        // Staying paused takes nothing more.
        render(&mut renderer, 4_800);
        assert_eq!(shared.played.load(Ordering::Relaxed), played);
        // No flush was involved.
        assert_eq!(shared.flush_done.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn resuming_carries_on_from_the_same_sample_and_fades_back_in() {
        let rate = 48_000;
        let (mut renderer, shared) = speaker(rate);
        // A ramp, so where playback resumes can be read off the samples.
        let ramp: Vec<f32> = (0..20_000).map(|index| index as f32 / 40_000.0).collect();
        shared.ring.push_slice(&ramp);
        render(&mut renderer, 100);
        shared.paused.store(true, Ordering::SeqCst);
        render(&mut renderer, 2_000);
        let played = shared.played.load(Ordering::Relaxed) as usize;

        shared.paused.store(false, Ordering::SeqCst);
        let after = render(&mut renderer, 2_000);

        // Nothing was skipped: the next sample out is the next one of the ramp, faded in.
        let fade_samples = (rate as f32 * STOP_FADE.as_secs_f32()) as usize;
        assert!(after[0] < ramp[played] * PLAYBACK_GAIN * 0.01 + 1e-6);
        let settled = fade_samples + 10;
        assert!((after[settled] - ramp[played + settled] * PLAYBACK_GAIN).abs() < 1e-6);
        assert_eq!(
            shared.played.load(Ordering::Relaxed) as usize,
            played + 2_000
        );
    }

    #[test]
    fn a_stop_while_paused_discards_at_once_and_the_pause_holds_until_resumed() {
        let (mut renderer, shared) = speaker(48_000);
        shared.ring.push_slice(&[0.5; 4_000]);
        shared.paused.store(true, Ordering::SeqCst);
        render(&mut renderer, 1_000);

        shared.flush_requested.fetch_add(1, Ordering::SeqCst);
        render(&mut renderer, 1);
        assert_eq!(shared.flush_done.load(Ordering::SeqCst), 1);
        assert!(shared.ring.is_empty());

        // The next reply is queued while still paused: it waits.
        shared.ring.push_slice(&[0.25; 1_000]);
        assert!(render(&mut renderer, 500)
            .iter()
            .all(|sample| *sample == 0.0));
        shared.paused.store(false, Ordering::SeqCst);
        let after = render(&mut renderer, 500);
        assert_eq!(after[499], 0.25 * PLAYBACK_GAIN);
    }

    #[test]
    fn queued_speech_is_played_in_order_then_silence() {
        let (mut renderer, shared) = speaker(48_000);
        shared.ring.push_slice(&[0.5, -0.5, 0.25]);
        let mut output = [0.0f32; 10];

        renderer.fill(&mut output, 2, |sample| sample);

        let gain = PLAYBACK_GAIN;
        assert_eq!(
            &output[..6],
            &[
                0.5 * gain,
                0.5 * gain,
                -0.5 * gain,
                -0.5 * gain,
                0.25 * gain,
                0.25 * gain
            ]
        );
        assert_eq!(&output[6..], &[0.0; 4]);
        assert_eq!(shared.played.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn a_stop_fades_out_within_a_few_milliseconds_and_discards_the_rest() {
        let rate = 48_000;
        let (mut renderer, shared) = speaker(rate);
        shared.ring.push_slice(&vec![0.5; rate as usize / 2]); // half a second queued
        let mut before = [0.0f32; 480];
        renderer.fill(&mut before, 1, |sample| sample);

        shared.flush_requested.fetch_add(1, Ordering::SeqCst);
        let mut after = [1.0f32; 960]; // 20 ms
        renderer.fill(&mut after, 1, |sample| sample);

        // The level falls steadily: no jump down (which would click) and no jump up.
        let mut previous = before[479];
        for sample in &after {
            assert!(*sample <= previous + 1e-6, "level rose during the fade");
            assert!(previous - *sample < 0.02, "level dropped abruptly");
            previous = *sample;
        }
        // Silent within the fade time, and it stays silent.
        let fade_samples = (rate as f32 * STOP_FADE.as_secs_f32()) as usize;
        assert!(after[fade_samples + 2..]
            .iter()
            .all(|sample| *sample == 0.0));
        // Everything that was queued is gone and the request is acknowledged.
        assert!(shared.ring.is_empty());
        assert_eq!(shared.flush_done.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn audio_written_after_a_stop_plays_normally() {
        let (mut renderer, shared) = speaker(48_000);
        shared.ring.push_slice(&[0.5; 2_000]);
        shared.flush_requested.fetch_add(1, Ordering::SeqCst);
        let mut scratch = [0.0f32; 1_000];
        renderer.fill(&mut scratch, 1, |sample| sample);
        assert_eq!(shared.flush_done.load(Ordering::SeqCst), 1);

        // The next reply.
        shared.ring.push_slice(&[0.25; 4]);
        let mut output = [0.0f32; 4];
        renderer.fill(&mut output, 1, |sample| sample);

        assert_eq!(output, [0.25 * PLAYBACK_GAIN; 4]);
    }

    #[test]
    fn everything_sent_to_the_device_can_be_collected_in_step() {
        let (mut renderer, shared) = speaker(48_000);
        shared.ring.push_slice(&[0.5, -0.5]);
        let mut output = [0.0f32; 8];
        renderer.fill(&mut output, 2, |sample| sample);

        let mut collected = Vec::new();
        shared.played_copy.pop_all(&mut collected);

        // One mono sample per device frame: the two queued samples, then the silence that
        // followed, exactly as the speaker received them.
        assert_eq!(
            collected,
            vec![0.5 * PLAYBACK_GAIN, -0.5 * PLAYBACK_GAIN, 0.0, 0.0]
        );
    }

    #[test]
    fn a_stop_with_nothing_playing_is_acknowledged_at_once() {
        let (mut renderer, shared) = speaker(48_000);
        shared.flush_requested.fetch_add(1, Ordering::SeqCst);
        let mut output = [1.0f32; 8];

        renderer.fill(&mut output, 1, |sample| sample);

        assert_eq!(output, [0.0; 8]);
        assert_eq!(shared.flush_done.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn the_microphone_callback_writes_one_channel_and_counts_what_it_drops() {
        let shared = MicShared {
            ring: SampleRing::new(4),
            failed: AtomicBool::new(false),
            overruns: AtomicU64::new(0),
        };
        let mut selector = MicChannel::default();
        // Stereo: silence on the left, signal on the right.
        let data = [
            0.0f32, 0.3, 0.0, 0.4, 0.0, 0.5, 0.0, 0.6, 0.0, 0.7, 0.0, 0.8,
        ];

        capture(&data, 2, &mut selector, &shared, |sample| sample);

        let mut out = Vec::new();
        shared.ring.pop_all(&mut out);
        assert_eq!(out, vec![0.3, 0.4, 0.5, 0.6]);
        // The ring holds four samples; two frames found it full.
        assert_eq!(shared.overruns.load(Ordering::Relaxed), 2);
        assert_eq!(selector.locked, Some(1));
    }

    #[test]
    fn the_channel_choice_locks_once_a_channel_carries_signal() {
        let mut selector = MicChannel::default();

        // Room noise only: the loudest channel is used but nothing is locked.
        assert_eq!(selector.choose(&[0.001f32, 0.0, 0.001, 0.0], 2, |s| s), 0);
        assert_eq!(selector.locked, None);
        // Speech on channel 1 locks it.
        assert_eq!(selector.choose(&[0.0f32, 0.25, 0.0, 0.5], 2, |s| s), 1);
        // A later callback where channel 0 is briefly louder does not flip the choice.
        assert_eq!(selector.choose(&[0.9f32, 0.1, 0.9, 0.2], 2, |s| s), 1);
        // Mono needs no choice.
        assert_eq!(MicChannel::default().choose(&[0.5f32, 0.5], 1, |s| s), 0);
    }

    #[test]
    fn sixteen_bit_capture_is_scaled_to_unit_range() {
        let shared = MicShared {
            ring: SampleRing::new(8),
            failed: AtomicBool::new(false),
            overruns: AtomicU64::new(0),
        };
        let mut selector = MicChannel::default();

        capture(
            &[i16::MAX, 0, i16::MIN],
            1,
            &mut selector,
            &shared,
            |sample| sample as f32 / i16::MAX as f32,
        );

        let mut out = Vec::new();
        shared.ring.pop_all(&mut out);
        assert_eq!(out, vec![1.0, 0.0, -1.0]);
    }
}
