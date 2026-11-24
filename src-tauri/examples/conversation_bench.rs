//! End-to-end latency of conversation mode, measured without a person or a sound card.
//!
//! The real voice detector, speech recogniser, language model and voice run exactly as in
//! the app. Only the two devices are stand-ins: the "microphone" plays questions (spoken by
//! the conversation voice itself) in real time, and the "speaker" drains at playback speed
//! and notes when the first sample of each reply arrives.
//!
//! ```text
//! cargo run --release --example conversation_bench -- <runtime dir> [--model llama3.2:3b] [--passes 2]
//! ```
//!
//! `<runtime dir>` is `.localflow-runtime`. Ollama must be running with the model installed.
//! Only timings and character counts are printed.

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use localflow_lib::{
    asr::{
        provider::SpeechToTextProvider,
        transcribe_cpp::{TranscribeCppConfig, TranscribeCppProvider},
    },
    audio::{
        devices::{AudioSink, AudioSource},
        resample::resample,
        smart_turn::{best_turn_model, SMART_TURN_MODEL_FILE},
        speech_detector::{best_detector, SILERO_MODEL_FILE},
    },
    cancel::CancelFlag,
    conversation::{
        engine::{
            ConversationCore, ConversationHost, Limits, NoPageSource, Notice, Resources,
            TurnTimings,
        },
        state::{Event, Phase},
    },
    llm::chat::{ChatProvider, OllamaChat},
    tts::{
        phonon::{PhononConfig, PhononProvider, PHONON_MODEL_DIR},
        stream::StreamingTextToSpeech,
    },
};

const MIC_RATE: u32 = 48_000;
const SPEAKER_RATE: u32 = 48_000;
const QUESTIONS: &[&str] = &[
    "What is the capital of France?",
    "Can you explain what a mutex is in simple terms?",
    "Give me a quick idea for dinner tonight.",
    "I have been trying to stay focused while working from home, but I keep getting distracted by my phone and by chores around the house. What would you suggest?",
];

/// A microphone that delivers prepared audio at real-time pace, then silence.
struct PacedMic {
    pending: Mutex<Vec<f32>>,
    /// Set when the last sample of the current question has been delivered.
    speech_ended_at: Mutex<Option<Instant>>,
}

impl PacedMic {
    /// Deliver `speech` in 10 ms pieces as a real microphone would, with a short lead-in.
    fn say(self: &Arc<Self>, speech: Vec<f32>) {
        *self.speech_ended_at.lock().unwrap() = None;
        let mic = self.clone();
        thread::spawn(move || {
            let piece = MIC_RATE as usize / 100;
            let started = Instant::now();
            let mut sent = 0usize;
            let lead_in = vec![0.0f32; MIC_RATE as usize * 3 / 10];
            let total: Vec<f32> = lead_in.into_iter().chain(speech).collect();
            while sent < total.len() {
                let end = (sent + piece).min(total.len());
                mic.pending
                    .lock()
                    .unwrap()
                    .extend_from_slice(&total[sent..end]);
                sent = end;
                let due =
                    started + Duration::from_micros(sent as u64 * 1_000_000 / MIC_RATE as u64);
                thread::sleep(due.saturating_duration_since(Instant::now()));
            }
            *mic.speech_ended_at.lock().unwrap() = Some(Instant::now());
        });
    }
}

impl AudioSource for PacedMic {
    fn sample_rate(&self) -> u32 {
        MIC_RATE
    }

    fn read(&self, out: &mut Vec<f32>) {
        let mut pending = self.pending.lock().unwrap();
        if pending.is_empty() {
            // Between questions the room is silent, not dead: keep the audio clock running.
            drop(pending);
            out.extend(std::iter::repeat(0.0).take(MIC_RATE as usize / 100));
            return;
        }
        out.append(&mut pending);
    }

    fn healthy(&self) -> bool {
        true
    }
}

/// A speaker that drains at playback speed and records when each reply's audio first arrives.
struct TimedSpeaker {
    queued: AtomicU64,
    played: AtomicU64,
    first_write_at: Mutex<Option<Instant>>,
    flushed: AtomicBool,
}

impl TimedSpeaker {
    fn start() -> Arc<Self> {
        let speaker = Arc::new(Self {
            queued: AtomicU64::new(0),
            played: AtomicU64::new(0),
            first_write_at: Mutex::new(None),
            flushed: AtomicBool::new(false),
        });
        let drain = speaker.clone();
        thread::spawn(move || {
            let step = Duration::from_millis(5);
            let per_step = SPEAKER_RATE as u64 * 5 / 1_000;
            loop {
                thread::sleep(step);
                let queued = drain.queued.load(Ordering::SeqCst);
                let take = queued.min(per_step);
                if take > 0 {
                    drain.queued.fetch_sub(take, Ordering::SeqCst);
                    drain.played.fetch_add(take, Ordering::SeqCst);
                }
            }
        });
        speaker
    }
}

impl AudioSink for TimedSpeaker {
    fn sample_rate(&self) -> u32 {
        SPEAKER_RATE
    }

    fn write(&self, samples: &[f32]) -> usize {
        if !samples.is_empty() {
            self.first_write_at
                .lock()
                .unwrap()
                .get_or_insert_with(Instant::now);
        }
        self.queued
            .fetch_add(samples.len() as u64, Ordering::SeqCst);
        samples.len()
    }

    fn queued(&self) -> usize {
        self.queued.load(Ordering::SeqCst) as usize
    }

    fn played(&self) -> u64 {
        self.played.load(Ordering::SeqCst)
    }

    fn flush(&self) {
        self.flushed.store(true, Ordering::SeqCst);
        self.queued.store(0, Ordering::SeqCst);
    }

    fn healthy(&self) -> bool {
        true
    }
}

struct BenchHost {
    runtime: PathBuf,
    model: String,
    mic: Arc<PacedMic>,
    speaker: Arc<TimedSpeaker>,
    asr: Arc<TranscribeCppProvider>,
    chat: Arc<OllamaChat>,
    voice: Arc<PhononProvider>,
    turns: Mutex<Vec<TurnTimings>>,
    problems: Mutex<Vec<String>>,
}

impl ConversationHost for BenchHost {
    fn open(&self) -> Result<Resources, String> {
        let silero = self.runtime.join("models").join(SILERO_MODEL_FILE);
        let (detector, fallback) = best_detector(Some(&silero));
        if let Some(reason) = fallback {
            return Err(reason);
        }
        Ok(Resources {
            source: self.mic.clone(),
            sink: self.speaker.clone(),
            detector,
            asr: self.asr.clone(),
            chat: self.chat.clone(),
            voice: self.voice.clone(),
            page: Arc::new(NoPageSource),
            echo: Box::new(localflow_lib::audio::echo::NoEchoCancellation),
            readiness: localflow_lib::conversation::default_readiness(),
            // As in the app: the turn model when it is installed, the words alone otherwise.
            turn_model: best_turn_model(Some(
                &self.runtime.join("models").join(SMART_TURN_MODEL_FILE),
            ))
            .0,
            model: self.model.clone(),
            language: "en".to_string(),
        })
    }

    fn notify(&self, notice: Notice) {
        match notice {
            Notice::Turn(timings) => self.turns.lock().unwrap().push(timings),
            Notice::Problem(problem) => self.problems.lock().unwrap().push(problem),
            _ => {}
        }
    }
}

fn wait_for(what: &str, limit: Duration, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + limit;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        thread::sleep(Duration::from_millis(5));
    }
}

fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values[values.len() / 2]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let runtime = PathBuf::from(
        args.next()
            .expect("usage: conversation_bench <runtime dir>"),
    );
    let mut model = "llama3.2:3b".to_string();
    let mut passes = 2usize;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--model" => model = args.next().expect("--model needs a value"),
            "--passes" => passes = args.next().and_then(|v| v.parse().ok()).unwrap_or(passes),
            other => panic!("unknown argument {other}"),
        }
    }

    let voice = Arc::new(PhononProvider::new(PhononConfig {
        model_dir: runtime.join("models").join(PHONON_MODEL_DIR),
    }));
    voice.prepare().expect("the conversation voice should load");
    let asr = Arc::new(TranscribeCppProvider::new(TranscribeCppConfig {
        model_path: runtime
            .join("models")
            .join("parakeet-tdt-0.6b-v3-Q5_K_M.gguf"),
    }));
    // As the app does when a conversation opens: load and warm up.
    asr.warm_up().expect("the speech model should load");
    let chat = Arc::new(OllamaChat::new().expect("chat client"));
    chat.prepare(&model)
        .expect("Ollama should be running with the model installed");

    // The questions, spoken by the conversation voice and resampled to the microphone rate.
    let questions: Vec<Vec<f32>> = QUESTIONS
        .iter()
        .map(|question| {
            let mut pcm = Vec::new();
            voice
                .synthesize_stream(question, &CancelFlag::default(), &mut |piece| {
                    pcm.extend_from_slice(piece)
                })
                .expect("question audio");
            resample(&pcm, voice.sample_rate(), MIC_RATE)
        })
        .collect();

    let host = Arc::new(BenchHost {
        runtime,
        model: model.clone(),
        mic: Arc::new(PacedMic {
            pending: Mutex::new(Vec::new()),
            speech_ended_at: Mutex::new(None),
        }),
        speaker: TimedSpeaker::start(),
        asr,
        chat,
        voice,
        turns: Mutex::new(Vec::new()),
        problems: Mutex::new(Vec::new()),
    });
    let core = ConversationCore::start(
        host.clone(),
        Limits {
            idle: Duration::from_secs(120),
            ..Limits::default()
        },
    );
    core.send(Event::Toggle);
    wait_for("listening", Duration::from_secs(30), || {
        core.phase() == Phase::Listening
    });

    println!("model={model}");
    println!("pass,question,speech_ms,endpoint_wait_ms,transcribe_ms,model_first_text_ms,model_prompt_ms,model_load_ms,model_prompt_tokens,voice_first_audio_ms,response_ms_estimated,response_ms_true,transcript_chars,reply_chars");
    let mut true_responses = Vec::new();
    let mut transcribes = Vec::new();
    for pass in 0..passes {
        for (index, speech) in questions.iter().enumerate() {
            // Let the previous reply finish and the microphone settle.
            wait_for("listening", Duration::from_secs(120), || {
                core.phase() == Phase::Listening
            });
            thread::sleep(Duration::from_millis(600));
            *host.speaker.first_write_at.lock().unwrap() = None;
            let answered = host.turns.lock().unwrap().len();

            host.mic.say(speech.clone());
            wait_for("the turn to be answered", Duration::from_secs(120), || {
                host.turns.lock().unwrap().len() > answered
                    || !host.problems.lock().unwrap().is_empty()
            });
            if let Some(problem) = host.problems.lock().unwrap().first() {
                panic!("the conversation reported a problem: {problem}");
            }

            let timings = *host.turns.lock().unwrap().last().unwrap();
            let ended = host
                .mic
                .speech_ended_at
                .lock()
                .unwrap()
                .expect("speech end time");
            let first_audio = host
                .speaker
                .first_write_at
                .lock()
                .unwrap()
                .expect("reply audio");
            let true_response = first_audio.saturating_duration_since(ended).as_millis() as u64;
            println!(
                "{pass},{index},{},{},{},{},{},{},{},{},{},{true_response},{},{}",
                timings.speech_ms,
                timings.endpoint_wait_ms,
                timings.transcribe_ms,
                timings.model_first_text_ms.unwrap_or(0),
                timings.model_prompt_ms.unwrap_or(0),
                timings.model_load_ms.unwrap_or(0),
                timings.model_prompt_tokens.unwrap_or(0),
                timings.voice_first_audio_ms.unwrap_or(0),
                timings.response_ms.unwrap_or(0),
                timings.transcript_chars,
                timings.reply_chars,
            );
            true_responses.push(true_response);
            transcribes.push(timings.transcribe_ms);
        }
    }

    println!(
        "median_response_ms_true={} max_response_ms_true={} median_transcribe_ms={}",
        median(&mut true_responses.clone()),
        true_responses.iter().max().unwrap(),
        median(&mut transcribes),
    );

    // Interruption: how long from the hotkey to the speaker being told to stop?
    wait_for("listening", Duration::from_secs(120), || {
        core.phase() == Phase::Listening
    });
    thread::sleep(Duration::from_millis(600));
    host.speaker.flushed.store(false, Ordering::SeqCst);
    host.mic.say(questions[3].clone());
    wait_for("speaking", Duration::from_secs(120), || {
        core.phase() == Phase::Speaking
    });
    thread::sleep(Duration::from_millis(400));
    let pressed = Instant::now();
    core.send(Event::Toggle);
    wait_for("the speaker to be silenced", Duration::from_secs(5), || {
        host.speaker.flushed.load(Ordering::SeqCst)
    });
    println!(
        "interrupt_to_silence_request_ms={}",
        pressed.elapsed().as_millis()
    );
    wait_for(
        "listening after the interruption",
        Duration::from_secs(10),
        || core.phase() == Phase::Listening,
    );
    core.send(Event::EndRequested);
    wait_for("off", Duration::from_secs(10), || {
        core.phase() == Phase::Off
    });
}
