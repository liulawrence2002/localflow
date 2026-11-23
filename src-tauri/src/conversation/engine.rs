//! Runs a conversation: microphone in, turn detection, transcription, reply, speech out.
//!
//! Everything here is written against traits (audio source and sink, speech detector,
//! speech-to-text, chat model, voice), so the whole lifecycle is exercised in tests with
//! scripted audio and fake models. The Tauri-facing wrapper in `mod.rs` supplies the real
//! devices and models.
//!
//! Three kinds of thread cooperate, all driven by the pure state machine in `state.rs`:
//!
//! - the **loop** owns the state. It applies events and carries out the effects.
//! - the **ear** reads the microphone while a conversation is on, finds the start and end of
//!   each user turn, and enforces the idle and maximum-length limits.
//! - a **turn worker** per user turn transcribes it, decides what it is, and for a question
//!   streams the model's reply into the voice and the voice into the speaker.
//!
//! Text is never logged and never placed in a [`Notice`]: notices carry phases, fixed
//! messages, counts and timings only.

use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use super::{
    assembler::{Action, AssemblerConfig, ReadinessJudge, TurnAssembler},
    chunker::SentenceChunker,
    echo_guard::is_echo_of,
    memory::ConversationMemory,
    router::{route_turn, ReadWhat, TurnIntent},
    speaking::{speak_pieces, Spoken},
    state::{transition, ConversationState, Effect, Event, Phase, StopReason, TurnOutcome},
};
use crate::{
    asr::provider::{SpeechToTextProvider, TranscriptionRequest},
    audio::{
        devices::{AudioSink, AudioSource},
        echo::{EchoCanceller, ECHO_SAMPLE_RATE},
        resample::StreamResampler,
        spectrum::{SpectrumAnalyzer, SpectrumFrame},
        speech_detector::{EnergyDetector, SpeechDetector, DETECTOR_SAMPLE_RATE},
        turn_segmenter::{SegmentEvent, SegmenterConfig, TurnSegmenter},
    },
    cancel::CancelFlag,
    llm::chat::{ChatProvider, LlmError},
    tts::{provider::TtsError, stream::StreamingTextToSpeech},
};

/// Everything a conversation needs, opened when it starts and closed when it ends.
pub struct Resources {
    pub source: Arc<dyn AudioSource>,
    pub sink: Arc<dyn AudioSink>,
    pub detector: Box<dyn SpeechDetector>,
    pub asr: Arc<dyn SpeechToTextProvider>,
    pub chat: Arc<dyn ChatProvider>,
    pub voice: Arc<dyn StreamingTextToSpeech>,
    /// Reads the focused window when the user asks for it.
    pub page: Arc<dyn PageSource>,
    /// Removes the assistant's own voice from the microphone. When it is a real canceller
    /// the user can interrupt by speaking; with the pass-through one, only by hotkey.
    pub echo: Box<dyn EchoCanceller>,
    /// Judges how finished a transcript reads, which sets how long a silence ends the turn.
    pub readiness: ReadinessJudge,
    /// The local model to answer with.
    pub model: String,
    /// Language code for speech recognition.
    pub language: String,
}

/// Limits and timings. Fixed in production; shortened in tests.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A conversation ends after this much silence while waiting for the user.
    pub idle: Duration,
    /// A conversation never lasts longer than this.
    pub max_conversation: Duration,
    /// One user turn is cut off at this length.
    pub max_utterance: Duration,
    /// A silence this long counts as a pause: the speech so far is sent for transcription.
    pub pause: Duration,
    /// Silence that ends a turn whose words read as finished.
    pub ready_wait: Duration,
    /// Silence that ends a turn when it is unclear whether the user has finished.
    pub unsure_wait: Duration,
    /// Silence that ends a turn whose words trail off mid-thought.
    pub not_ready_wait: Duration,
    /// How long the user must speak over the assistant before it gives way. Longer than
    /// the ordinary start of a turn, because a remnant of the assistant's own voice or a
    /// noise in the room must not cut a reply off.
    pub barge_in_speech: Duration,
    /// How sure the speech detector must be, from 0 to 1, for speech over the assistant.
    pub barge_in_threshold: f32,
    /// Audio kept from before speech was detected, so the first word is not clipped.
    pub pre_roll: Duration,
    /// Silence kept after the last word of a turn.
    pub tail: Duration,
    /// Microphone audio ignored right after the assistant stops speaking, so the last of its
    /// own voice still coming out of the speaker is not taken for the user.
    pub reopen_guard: Duration,
    /// Pause inserted between spoken sentences.
    pub sentence_gap: Duration,
    /// A microphone that delivers nothing for this long is treated as lost.
    pub device_silence: Duration,
    /// How often the ear wakes to read the microphone.
    pub poll: Duration,
    /// While the user speaks, the language model is nudged this often so it is awake and has
    /// the conversation cached by the time the question is ready.
    pub warm_interval: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            idle: Duration::from_secs(45),
            max_conversation: Duration::from_secs(10 * 60),
            max_utterance: Duration::from_secs(60),
            pause: Duration::from_millis(250),
            ready_wait: Duration::from_millis(400),
            unsure_wait: Duration::from_millis(700),
            not_ready_wait: Duration::from_millis(1_400),
            barge_in_speech: Duration::from_millis(288),
            barge_in_threshold: 0.65,
            pre_roll: Duration::from_millis(400),
            tail: Duration::from_millis(200),
            reopen_guard: Duration::from_millis(300),
            sentence_gap: Duration::from_millis(140),
            device_silence: Duration::from_secs(3),
            poll: Duration::from_millis(10),
            warm_interval: Duration::from_millis(400),
        }
    }
}

/// How long each stage of one answered turn took. Numbers only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnTimings {
    pub turn: u64,
    /// Length of the user's speech.
    pub speech_ms: u64,
    /// Silence waited for before deciding the turn had ended.
    pub endpoint_wait_ms: u64,
    pub transcribe_ms: u64,
    /// From sending the question to the first piece of the reply.
    pub model_first_text_ms: Option<u64>,
    /// Of that, how long the model server reported spending on reading the prompt and on
    /// loading the model. The rest is queueing and generating the first words.
    pub model_prompt_ms: Option<u64>,
    pub model_load_ms: Option<u64>,
    pub model_prompt_tokens: Option<u32>,
    /// From the first speakable piece to the first audio queued.
    pub voice_first_audio_ms: Option<u64>,
    /// From the end of the user's speech to the first audio queued: what the user feels.
    pub response_ms: Option<u64>,
    pub transcript_chars: usize,
    pub reply_chars: usize,
}

/// What the engine tells the outside world. Never contains anything that was said.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// The phase changed. `reason` is set when the conversation ended.
    Phase {
        phase: Phase,
        reason: Option<StopReason>,
    },
    /// Something went wrong. A fixed, user-facing sentence.
    Problem(String),
    /// A live picture of the microphone while listening, for the overlay.
    Microphone { frame: SpectrumFrame, level: f32 },
    /// Timings of a turn that was answered.
    Turn(TurnTimings),
}

/// Reads what is in the focused window so it can be spoken word for word.
///
/// The text it returns goes straight to the voice. It is never shown to the language model,
/// never added to the conversation's memory, and never placed in a notice or a log.
pub trait PageSource: Send + Sync {
    /// The pieces to speak, in order. An empty list means there is nothing further to read.
    /// An error is a fixed sentence to say instead, such as why the window may not be read.
    fn read(&self, what: ReadWhat) -> Result<Vec<String>, String>;

    /// The first `pieces` pieces of the last [`PageSource::read`] were heard to their end,
    /// so the reading position can move on exactly that far. A piece cut off part-way is
    /// not counted, so it is read again next time.
    fn heard(&self, pieces: usize);
}

/// A page source for hosts that cannot read windows.
pub struct NoPageSource;

impl PageSource for NoPageSource {
    fn read(&self, _what: ReadWhat) -> Result<Vec<String>, String> {
        Err("Reading the screen isn't available here.".to_string())
    }

    fn heard(&self, _pieces: usize) {}
}

/// Supplies devices and models, and receives notices.
pub trait ConversationHost: Send + Sync + 'static {
    /// Open the microphone, the speaker and the models. Called on a worker thread.
    fn open(&self) -> Result<Resources, String>;
    fn notify(&self, notice: Notice);
}

/// Spoken when the model cannot be reached, so the user is not left in silence.
const MODEL_DOWN_LINE: &str = "I can't reach the language model. Check that Ollama is running.";
const MODEL_MISSING_LINE: &str = "The language model isn't installed yet.";
const MODEL_FAILED_LINE: &str = "Sorry, I couldn't work out a reply.";
const NOTHING_TO_REPEAT_LINE: &str = "I haven't said anything yet.";
const END_OF_PAGE_LINE: &str = "That's the end of the page.";

enum Message {
    Event(Event),
    Opened {
        conversation: u64,
        result: Result<Resources, String>,
    },
}

/// One user turn's audio, handed from the ear to the turn worker.
struct Utterance {
    /// Everything the user said, already transcribed while they spoke.
    text: String,
    /// From the first word to the last.
    speech_ms: u64,
    /// Total time the recogniser spent on this turn's phrases.
    transcribe_ms: u64,
    /// When the user stopped speaking, as best the detector can tell.
    speech_ended_at: Instant,
    endpoint_wait: Duration,
}

/// What stays open for the length of one conversation.
struct Session {
    conversation: u64,
    stop: CancelFlag,
    /// Whether the ear acts on microphone audio right now.
    listening: Arc<AtomicBool>,
    /// Incremented each time the floor is handed back to the user. The ear starts a fresh
    /// turn when it sees a new value, so it cannot miss a hand-back however brief the gap.
    listen_epoch: Arc<AtomicU64>,
    /// Whether the language model should be kept warm: true from the moment the user starts
    /// speaking until their question is sent to it.
    keep_model_warm: Arc<AtomicBool>,
    /// True while a reply is being spoken, which is when the user may talk over it.
    assistant_speaking: Arc<AtomicBool>,
    /// The reply being spoken, as text, so its echo can be told from an interruption.
    speaking_text: Arc<Mutex<String>>,
    utterance: Arc<Mutex<Option<Utterance>>>,
    source: Arc<dyn AudioSource>,
    sink: Arc<dyn AudioSink>,
    asr: Arc<dyn SpeechToTextProvider>,
    chat: Arc<dyn ChatProvider>,
    voice: Arc<dyn StreamingTextToSpeech>,
    page: Arc<dyn PageSource>,
    model: String,
    language: String,
}

pub struct ConversationCore {
    tx: Mutex<mpsc::Sender<Message>>,
    phase: Arc<Mutex<Phase>>,
}

impl ConversationCore {
    pub fn start(host: Arc<dyn ConversationHost>, limits: Limits) -> Self {
        let (tx, rx) = mpsc::channel();
        let phase = Arc::new(Mutex::new(Phase::Off));
        let loop_tx = tx.clone();
        let loop_phase = phase.clone();
        thread::Builder::new()
            .name("localflow-conversation".to_string())
            .spawn(move || run_loop(host, limits, rx, loop_tx, loop_phase))
            .expect("the conversation thread should start");
        Self {
            tx: Mutex::new(tx),
            phase,
        }
    }

    pub fn send(&self, event: Event) {
        let tx = self
            .tx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = tx.send(Message::Event(event));
    }

    pub fn phase(&self) -> Phase {
        *self
            .phase
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn run_loop(
    host: Arc<dyn ConversationHost>,
    limits: Limits,
    rx: mpsc::Receiver<Message>,
    tx: mpsc::Sender<Message>,
    shared_phase: Arc<Mutex<Phase>>,
) {
    let mut state = ConversationState::default();
    let mut session: Option<Session> = None;
    let mut turn_cancel: Option<CancelFlag> = None;
    let memory = Arc::new(Mutex::new(ConversationMemory::default()));

    for message in rx {
        let event = match message {
            Message::Event(event) => event,
            Message::Opened {
                conversation,
                result,
            } => {
                if state.phase != Phase::Starting || state.conversation != conversation {
                    // The conversation was cancelled while it was starting. Dropping the
                    // resources closes the devices.
                    continue;
                }
                match result {
                    Ok(resources) => {
                        session = Some(open_session(
                            conversation,
                            resources,
                            &host,
                            limits,
                            &tx,
                            memory.clone(),
                        ));
                        Event::Ready { conversation }
                    }
                    Err(problem) => {
                        host.notify(Notice::Problem(problem));
                        Event::StartFailed { conversation }
                    }
                }
            }
        };

        let (next, effects) = transition(state, event);
        let mut stop_reason = None;
        for effect in effects {
            match effect {
                Effect::Start { conversation } => {
                    let host = host.clone();
                    let tx = tx.clone();
                    thread::spawn(move || {
                        let result = host.open();
                        let _ = tx.send(Message::Opened {
                            conversation,
                            result,
                        });
                    });
                }
                Effect::BeginTurn { turn } => {
                    let Some(active) = session.as_ref() else {
                        continue;
                    };
                    let utterance = active
                        .utterance
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .take();
                    let Some(utterance) = utterance else {
                        let _ = tx.send(Message::Event(Event::ReplyFailed { turn }));
                        continue;
                    };
                    let cancel = CancelFlag::default();
                    turn_cancel = Some(cancel.clone());
                    let context = TurnContext {
                        turn,
                        utterance,
                        cancel,
                        sink: active.sink.clone(),
                        chat: active.chat.clone(),
                        voice: active.voice.clone(),
                        page: active.page.clone(),
                        speaking_text: active.speaking_text.clone(),
                        model: active.model.clone(),
                        memory: memory.clone(),
                        keep_model_warm: active.keep_model_warm.clone(),
                        host: host.clone(),
                        tx: tx.clone(),
                        limits,
                    };
                    thread::spawn(move || run_turn(context));
                }
                Effect::CancelTurn { .. } => {
                    if let Some(cancel) = turn_cancel.take() {
                        cancel.cancel();
                    }
                    if let Some(active) = session.as_ref() {
                        active.sink.flush();
                    }
                }
                Effect::Stop { reason } => {
                    stop_reason = Some(reason);
                    if let Some(cancel) = turn_cancel.take() {
                        cancel.cancel();
                    }
                    if let Some(closing) = session.take() {
                        closing.stop.cancel();
                        closing.listening.store(false, Ordering::SeqCst);
                        closing.sink.flush();
                        tracing::info!(
                            conversation = closing.conversation,
                            reason = ?reason,
                            healthy_microphone = closing.source.healthy(),
                            "conversation ended"
                        );
                        // Dropping the session closes the microphone and the speaker.
                    }
                    memory
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .clear();
                }
            }
        }

        if let Some(active) = session.as_ref() {
            if next.phase == Phase::Listening && state.phase != Phase::Listening {
                active.listen_epoch.fetch_add(1, Ordering::SeqCst);
            }
            active
                .listening
                .store(next.phase.listens(), Ordering::SeqCst);
            // Warm from the first word; the turn worker switches it off when it asks the
            // question, and any phase other than "thinking" switches it off too.
            active
                .assistant_speaking
                .store(next.phase == Phase::Speaking, Ordering::SeqCst);
            match next.phase {
                Phase::UserSpeaking => active.keep_model_warm.store(true, Ordering::SeqCst),
                Phase::Thinking => {}
                _ => active.keep_model_warm.store(false, Ordering::SeqCst),
            }
        }
        if next.phase != state.phase {
            *shared_phase
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = next.phase;
            host.notify(Notice::Phase {
                phase: next.phase,
                reason: stop_reason,
            });
        }
        state = next;
    }
}

fn open_session(
    conversation: u64,
    resources: Resources,
    host: &Arc<dyn ConversationHost>,
    limits: Limits,
    tx: &mpsc::Sender<Message>,
    memory: Arc<Mutex<ConversationMemory>>,
) -> Session {
    let Resources {
        source,
        sink,
        detector,
        asr,
        chat,
        voice,
        page,
        echo,
        readiness,
        model,
        language,
    } = resources;
    let session = Session {
        conversation,
        stop: CancelFlag::default(),
        listening: Arc::new(AtomicBool::new(false)),
        listen_epoch: Arc::new(AtomicU64::new(0)),
        keep_model_warm: Arc::new(AtomicBool::new(false)),
        assistant_speaking: Arc::new(AtomicBool::new(false)),
        speaking_text: Arc::new(Mutex::new(String::new())),
        utterance: Arc::new(Mutex::new(None)),
        source,
        sink,
        asr,
        chat,
        voice,
        page,
        model,
        language,
    };

    let ear = Ear {
        conversation,
        source: session.source.clone(),
        detector,
        stop: session.stop.clone(),
        listening: session.listening.clone(),
        listen_epoch: session.listen_epoch.clone(),
        utterance: session.utterance.clone(),
        asr: session.asr.clone(),
        language: session.language.clone(),
        readiness,
        echo,
        sink: session.sink.clone(),
        assistant_speaking: session.assistant_speaking.clone(),
        speaking_text: session.speaking_text.clone(),
        host: host.clone(),
        tx: tx.clone(),
        limits,
    };
    thread::Builder::new()
        .name("localflow-ear".to_string())
        .spawn(move || ear.run())
        .expect("the listening thread should start");

    // Load the language model in the background so the first reply does not wait for it. A
    // failure is not fatal here: the first turn reports it aloud.
    let chat = session.chat.clone();
    let model = session.model.clone();
    thread::spawn(move || {
        if let Err(error) = chat.prepare(&model) {
            tracing::info!(error = %error, "language model warm-up skipped");
        }
    });

    // Keep the model awake while the user is speaking (see `ChatProvider::keep_warm`).
    let warmer = ModelWarmer {
        chat: session.chat.clone(),
        model: session.model.clone(),
        memory,
        wanted: session.keep_model_warm.clone(),
        stop: session.stop.clone(),
        interval: limits.warm_interval,
    };
    thread::Builder::new()
        .name("localflow-model-warmer".to_string())
        .spawn(move || warmer.run())
        .expect("the model warm-up thread should start");

    session
}

struct ModelWarmer {
    chat: Arc<dyn ChatProvider>,
    model: String,
    memory: Arc<Mutex<ConversationMemory>>,
    wanted: Arc<AtomicBool>,
    stop: CancelFlag,
    interval: Duration,
}

impl ModelWarmer {
    fn run(self) {
        let tick = Duration::from_millis(20).min(self.interval);
        let mut last: Option<Instant> = None;
        while !self.stop.is_cancelled() {
            thread::sleep(tick);
            if !self.wanted.load(Ordering::SeqCst) {
                // Start promptly the next time the user speaks.
                last = None;
                continue;
            }
            if last.is_some_and(|at| at.elapsed() < self.interval) {
                continue;
            }
            let messages = self
                .memory
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .messages();
            self.chat.keep_warm(&self.model, &messages);
            last = Some(Instant::now());
        }
    }
}

struct Ear {
    conversation: u64,
    source: Arc<dyn AudioSource>,
    detector: Box<dyn SpeechDetector>,
    stop: CancelFlag,
    listening: Arc<AtomicBool>,
    listen_epoch: Arc<AtomicU64>,
    utterance: Arc<Mutex<Option<Utterance>>>,
    asr: Arc<dyn SpeechToTextProvider>,
    language: String,
    readiness: ReadinessJudge,
    echo: Box<dyn EchoCanceller>,
    sink: Arc<dyn AudioSink>,
    assistant_speaking: Arc<AtomicBool>,
    speaking_text: Arc<Mutex<String>>,
    host: Arc<dyn ConversationHost>,
    tx: mpsc::Sender<Message>,
    limits: Limits,
}

/// The audio of the turn in progress and the bookkeeping that goes with it.
#[derive(Default)]
struct TurnAudio {
    /// 16 kHz audio not yet released. `samples[0]` is position `base` on the audio clock.
    samples: Vec<f32>,
    base: u64,
    /// Where the turn's audio began, for measuring how long the user spoke.
    turn_start: u64,
    /// Recogniser time spent on this turn so far.
    transcribe_ms: u64,
    /// The turn began by speaking over the assistant.
    barged: bool,
}

impl TurnAudio {
    fn clear(&mut self) {
        self.samples.clear();
        self.base = 0;
        self.turn_start = 0;
        self.transcribe_ms = 0;
        self.barged = false;
    }

    /// A copy of the audio in `from..to`, clamped to what is still held.
    fn slice(&self, from: u64, to: u64) -> Vec<f32> {
        let start = from
            .saturating_sub(self.base)
            .min(self.samples.len() as u64) as usize;
        let end = to.saturating_sub(self.base).min(self.samples.len() as u64) as usize;
        self.samples[start..end.max(start)].to_vec()
    }

    /// Drop audio before `position`.
    fn release(&mut self, position: u64) {
        let drop = position
            .saturating_sub(self.base)
            .min(self.samples.len() as u64) as usize;
        self.samples.drain(..drop);
        self.base += drop as u64;
    }

    /// Keep only the last `keep` samples.
    fn keep_last(&mut self, keep: usize) {
        if self.samples.len() > keep {
            let drop = self.samples.len() - keep;
            self.samples.drain(..drop);
            self.base += drop as u64;
        }
    }
}

/// What the ear is doing with the microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EarMode {
    /// Ignoring it: the assistant is thinking, or speaking with no way to tell its own voice
    /// from the user's.
    Deaf,
    /// The user has the floor: finding the start and end of their turn.
    Listening,
    /// The assistant is speaking; listening only for the user talking over it.
    Guarding,
}

/// One phrase for the recogniser: the request id and its audio.
type TranscribeJob = (u64, Vec<f32>);
/// The recogniser's answer: the id, the text (or failure) and how long it took.
type TranscribeDone = (u64, Result<String, ()>, u64);

impl Ear {
    fn send(&self, event: Event) {
        let _ = self.tx.send(Message::Event(event));
    }

    /// Transcribes phrases on its own thread, one at a time, so listening never stalls.
    fn start_transcriber(&self) -> (mpsc::Sender<TranscribeJob>, mpsc::Receiver<TranscribeDone>) {
        let (job_tx, job_rx) = mpsc::channel::<TranscribeJob>();
        let (done_tx, done_rx) = mpsc::channel::<TranscribeDone>();
        let asr = self.asr.clone();
        let language = self.language.clone();
        thread::Builder::new()
            .name("localflow-transcriber".to_string())
            .spawn(move || {
                for (id, samples) in job_rx {
                    let started = Instant::now();
                    let result = asr
                        .transcribe(&TranscriptionRequest {
                            samples: &samples,
                            language: &language,
                            initial_prompt: None,
                        })
                        .map(|transcription| {
                            // The recogniser's marker for "no speech" is not something said.
                            if crate::asr::whisper_text::is_blank_transcript(&transcription.text) {
                                String::new()
                            } else {
                                transcription.text
                            }
                        })
                        .map_err(|error| {
                            tracing::warn!(error = %error, "conversation phrase could not be transcribed");
                        });
                    if done_tx
                        .send((id, result, started.elapsed().as_millis() as u64))
                        .is_err()
                    {
                        return;
                    }
                }
            })
            .expect("the transcription thread should start");
        (job_tx, done_rx)
    }

    fn run(mut self) {
        let native_rate = self.source.sample_rate().max(1);
        let detector_rate = DETECTOR_SAMPLE_RATE as usize;
        let samples_16k =
            |duration: Duration| (duration.as_secs_f64() * detector_rate as f64) as usize;
        let samples_native =
            |duration: Duration| (duration.as_secs_f64() * native_rate as f64) as usize;
        // With a real echo canceller the microphone stays useful while the assistant speaks.
        let can_barge_in = self.echo.id() != "none";

        // The segmenter reports short pauses; the assembler decides which pause ends the turn.
        let segmenter_config = |frame: Duration, pause: Duration| SegmenterConfig {
            end_silence: pause,
            ..SegmenterConfig::conversation(frame)
        };
        // Speaking over the assistant has to be surer and last longer than starting a turn.
        let guard_config = |frame: Duration, limits: &Limits| SegmenterConfig {
            start_threshold: limits.barge_in_threshold,
            min_speech: limits.barge_in_speech,
            ..SegmenterConfig::conversation(frame)
        };
        let mut config = segmenter_config(self.detector.frame(), self.limits.pause);
        let mut segmenter = TurnSegmenter::new(config);
        let mut guard = TurnSegmenter::new(guard_config(self.detector.frame(), &self.limits));
        let mut frame_samples = samples_16k(config.frame).max(1);
        let mut assembler = TurnAssembler::new(
            AssemblerConfig {
                sample_rate: DETECTOR_SAMPLE_RATE,
                pre_roll: self.limits.pre_roll,
                tail: self.limits.tail,
                ready_wait: self.limits.ready_wait,
                unsure_wait: self.limits.unsure_wait,
                not_ready_wait: self.limits.not_ready_wait,
                max_turn: self.limits.max_utterance,
            },
            self.readiness.clone(),
        );
        let (jobs, done) = self.start_transcriber();
        let mut to_16k = StreamResampler::new(native_rate, DETECTOR_SAMPLE_RATE);
        let mut played_to_16k = StreamResampler::new(self.sink.sample_rate(), ECHO_SAMPLE_RATE);
        let mut analyzer = SpectrumAnalyzer::new(native_rate);

        let mut native: Vec<f32> = Vec::new();
        let mut pcm: Vec<f32> = Vec::new();
        let mut cleaned: Vec<f32> = Vec::new();
        let mut played: Vec<f32> = Vec::new();
        let mut played_16k: Vec<f32> = Vec::new();
        let mut probabilities: Vec<f32> = Vec::new();
        // 16 kHz audio the detector has been given but has not scored yet.
        let mut staging: Vec<f32> = Vec::new();
        let mut turn = TurnAudio::default();
        // Position on the audio clock: 16 kHz samples scored since the mode last changed.
        let mut clock = 0u64;
        let mut mode = EarMode::Deaf;
        // The last "floor is yours" signal acted on. Zero means none yet.
        let mut seen_epoch = 0u64;
        // A finished turn has been handed over; ignore the microphone until the loop gives
        // the floor back.
        let mut handed_off = false;
        // The user spoke over the assistant and the loop has not yet caught up: keep
        // listening to them meanwhile.
        let mut barging = false;
        let mut guard_remaining = 0usize;
        // Audio-clock counters: they advance with samples received, not with wall time.
        let mut idle_samples = 0usize;
        let mut total_native = 0u64;
        let mut picture_samples = 0usize;
        let mut max_reported = false;
        let mut last_audio_at = Instant::now();

        loop {
            if self.stop.is_cancelled() {
                return;
            }
            thread::sleep(self.limits.poll);

            // Transcripts first: they can end a turn even when no new audio has arrived.
            while let Ok((id, result, took_ms)) = done.try_recv() {
                if handed_off || mode != EarMode::Listening {
                    continue;
                }
                let actions = match result {
                    Ok(text) => {
                        turn.transcribe_ms += took_ms;
                        assembler.transcribed(id, &text, clock)
                    }
                    Err(()) => {
                        let actions = assembler.transcription_failed(id);
                        if !actions.is_empty() {
                            self.host.notify(Notice::Problem(
                                "I couldn't make out that audio.".to_string(),
                            ));
                        }
                        actions
                    }
                };
                handed_off |= self.carry_out(actions, &mut turn, &jobs);
            }

            native.clear();
            self.source.read(&mut native);
            if !self.source.healthy() {
                self.send(Event::DeviceLost {
                    conversation: self.conversation,
                });
                return;
            }
            if native.is_empty() {
                if last_audio_at.elapsed() >= self.limits.device_silence {
                    self.send(Event::DeviceLost {
                        conversation: self.conversation,
                    });
                    return;
                }
                continue;
            }
            last_audio_at = Instant::now();

            total_native += native.len() as u64;
            if !max_reported && total_native >= samples_native(self.limits.max_conversation) as u64
            {
                max_reported = true;
                self.send(Event::MaxDuration {
                    conversation: self.conversation,
                });
            }

            // The echo canceller hears everything, in every mode: what the speaker was given
            // and what the microphone picked up. It can only stay adapted if it is never
            // starved, and its output replaces the raw microphone from here on.
            played.clear();
            self.sink.take_played(&mut played);
            if !played.is_empty() {
                played_16k.clear();
                played_to_16k.process(&played, &mut played_16k);
                self.echo.played(&played_16k);
            }
            pcm.clear();
            to_16k.process(&native, &mut pcm);
            cleaned.clear();
            self.echo.heard(&pcm, &mut cleaned);

            // Which mode applies now.
            let listening = self.listening.load(Ordering::SeqCst);
            if listening {
                barging = false;
            }
            let wanted = if listening || barging {
                EarMode::Listening
            } else if can_barge_in && self.assistant_speaking.load(Ordering::SeqCst) {
                EarMode::Guarding
            } else {
                EarMode::Deaf
            };
            let epoch = self.listen_epoch.load(Ordering::SeqCst);
            let floor_returned = listening && epoch != seen_epoch;
            if floor_returned {
                seen_epoch = epoch;
            }
            if floor_returned || (wanted != mode && !(barging && wanted == EarMode::Listening)) {
                // A fresh start: the floor has been handed to the user, or the assistant has
                // begun speaking. A transcription still running belongs to the old turn.
                segmenter.reset();
                guard.reset();
                assembler.reset();
                self.detector.reset();
                analyzer.reset();
                staging.clear();
                turn.clear();
                clock = 0;
                handed_off = false;
                idle_samples = 0;
                // Without echo cancellation, skip the tail of the assistant's own voice.
                guard_remaining = if floor_returned && !can_barge_in {
                    samples_16k(self.limits.reopen_guard)
                } else {
                    0
                };
            }
            mode = wanted;
            if mode == EarMode::Deaf || handed_off {
                continue;
            }

            let mut fresh = &cleaned[..];
            if guard_remaining > 0 {
                let skip = guard_remaining.min(fresh.len());
                fresh = &fresh[skip..];
                guard_remaining -= skip;
            }
            if fresh.is_empty() {
                continue;
            }

            // A live picture for the overlay while the user has the floor.
            if mode == EarMode::Listening {
                analyzer.push(&native);
                picture_samples += native.len();
                if picture_samples >= native_rate as usize / 35 {
                    picture_samples = 0;
                    let level = (native.iter().map(|sample| sample * sample).sum::<f32>()
                        / native.len() as f32)
                        .sqrt();
                    self.host.notify(Notice::Microphone {
                        frame: analyzer.analyze(),
                        level,
                    });
                }
            }

            staging.extend_from_slice(fresh);
            probabilities.clear();
            if let Err(problem) = self.detector.feed(fresh, &mut probabilities) {
                // Keep the conversation going on the simple detector rather than ending it.
                tracing::warn!(
                    detector = self.detector.id(),
                    "speech detector failed; using the loudness rule"
                );
                self.host.notify(Notice::Problem(problem));
                self.detector = Box::new(EnergyDetector::default());
                config = segmenter_config(self.detector.frame(), self.limits.pause);
                segmenter = TurnSegmenter::new(config);
                guard = TurnSegmenter::new(guard_config(self.detector.frame(), &self.limits));
                frame_samples = samples_16k(config.frame).max(1);
                staging.clear();
                if assembler.in_turn() {
                    assembler.reset();
                    self.send(Event::TurnAbandoned {
                        conversation: self.conversation,
                    });
                }
                turn.clear();
                clock = 0;
                continue;
            }

            // Each probability scores the next frame of staged audio, in order. Moving the
            // audio across one frame at a time keeps the captured turn exactly in step with
            // the detector, however much audio arrived at once.
            let mut consumed = 0usize;
            for probability in probabilities.iter().copied() {
                let frame_end = (consumed + frame_samples).min(staging.len());
                turn.samples
                    .extend_from_slice(&staging[consumed..frame_end]);
                clock += (frame_end - consumed) as u64;
                consumed = frame_end;
                let frame = frame_samples as u64;

                if mode == EarMode::Guarding {
                    // Only one thing matters here: is the user talking over the reply?
                    turn.keep_last(
                        samples_16k(self.limits.pre_roll)
                            + samples_16k(self.limits.barge_in_speech),
                    );
                    if let Some(SegmentEvent::SpeechStarted { frames_ago }) =
                        guard.observe(probability)
                    {
                        self.send(Event::BargeIn {
                            conversation: self.conversation,
                        });
                        // Their words so far are the start of the next turn.
                        let started = clock.saturating_sub((frames_ago as u64 + 1) * frame);
                        let actions = assembler.speech_started(started);
                        self.carry_out(actions, &mut turn, &jobs);
                        turn.barged = true;
                        segmenter.assume_speaking();
                        barging = true;
                        mode = EarMode::Listening;
                    }
                    continue;
                }

                let actions = match segmenter.observe(probability) {
                    Some(SegmentEvent::SpeechStarted { frames_ago }) => assembler
                        .speech_started(clock.saturating_sub((frames_ago as u64 + 1) * frame)),
                    Some(SegmentEvent::SpeechEnded { frames_ago }) => assembler
                        .pause_started(clock.saturating_sub(frames_ago as u64 * frame), clock),
                    None => assembler.tick(clock),
                };
                handed_off |= self.carry_out(actions, &mut turn, &jobs);
                if handed_off {
                    break;
                }

                if !assembler.in_turn() {
                    // Waiting: keep only a short lead-in, and count the silence.
                    turn.keep_last(samples_16k(self.limits.pre_roll));
                    idle_samples += frame_samples;
                    if idle_samples >= samples_16k(self.limits.idle) {
                        idle_samples = 0;
                        self.send(Event::IdleTimeout {
                            conversation: self.conversation,
                        });
                    }
                } else {
                    idle_samples = 0;
                }
            }
            if handed_off {
                staging.clear();
            } else {
                staging.drain(..consumed);
            }
        }
    }

    /// Do what the assembler asked. Returns true when the turn was handed to the loop, after
    /// which the microphone is ignored until the floor comes back.
    fn carry_out(
        &self,
        actions: Vec<Action>,
        turn: &mut TurnAudio,
        jobs: &mpsc::Sender<TranscribeJob>,
    ) -> bool {
        let mut handed_off = false;
        for action in actions {
            match action {
                Action::TurnStarted => {
                    turn.turn_start = turn.base;
                    turn.transcribe_ms = 0;
                    self.send(Event::SpeechStarted {
                        conversation: self.conversation,
                    });
                }
                Action::Transcribe { id, from, to } => {
                    let _ = jobs.send((id, turn.slice(from, to)));
                }
                Action::Release { before } => turn.release(before),
                Action::Commit {
                    text,
                    speech_end,
                    waited,
                } => {
                    // An "interruption" that is only the assistant's own words, heard back
                    // through the microphone, is not a turn.
                    let own_voice = turn.barged && {
                        let speaking = self
                            .speaking_text
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner());
                        is_echo_of(&text, &speaking)
                    };
                    turn.barged = false;
                    if own_voice {
                        tracing::info!("an interruption was the assistant's own voice; ignored");
                        self.send(Event::TurnAbandoned {
                            conversation: self.conversation,
                        });
                        continue;
                    }
                    let utterance = Utterance {
                        text,
                        speech_ms: speech_end.saturating_sub(turn.turn_start) * 1_000
                            / DETECTOR_SAMPLE_RATE as u64,
                        transcribe_ms: turn.transcribe_ms,
                        speech_ended_at: Instant::now()
                            .checked_sub(waited)
                            .unwrap_or_else(Instant::now),
                        endpoint_wait: waited,
                    };
                    *self
                        .utterance
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(utterance);
                    self.send(Event::SpeechEnded {
                        conversation: self.conversation,
                    });
                    handed_off = true;
                }
                Action::Abandon => {
                    turn.barged = false;
                    self.send(Event::TurnAbandoned {
                        conversation: self.conversation,
                    });
                }
            }
        }
        handed_off
    }
}

struct TurnContext {
    turn: u64,
    utterance: Utterance,
    cancel: CancelFlag,
    sink: Arc<dyn AudioSink>,
    chat: Arc<dyn ChatProvider>,
    voice: Arc<dyn StreamingTextToSpeech>,
    page: Arc<dyn PageSource>,
    speaking_text: Arc<Mutex<String>>,
    model: String,
    memory: Arc<Mutex<ConversationMemory>>,
    keep_model_warm: Arc<AtomicBool>,
    host: Arc<dyn ConversationHost>,
    tx: mpsc::Sender<Message>,
    limits: Limits,
}

impl TurnContext {
    fn send(&self, event: Event) {
        let _ = self.tx.send(Message::Event(event));
    }

    fn memory(&self) -> std::sync::MutexGuard<'_, ConversationMemory> {
        self.memory
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn run_turn(context: TurnContext) {
    let turn = context.turn;
    let mut timings = TurnTimings {
        turn,
        speech_ms: context.utterance.speech_ms,
        endpoint_wait_ms: context.utterance.endpoint_wait.as_millis() as u64,
        transcribe_ms: context.utterance.transcribe_ms,
        ..Default::default()
    };
    // Transcribed phrase by phrase while the user was speaking.
    let transcript = context.utterance.text.clone();
    timings.transcript_chars = transcript.chars().count();

    let intent = if crate::asr::whisper_text::is_blank_transcript(&transcript) {
        TurnIntent::Nothing
    } else {
        route_turn(&transcript)
    };

    let outcome = match intent {
        TurnIntent::Nothing | TurnIntent::Quiet => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::NoReply,
            });
            return;
        }
        TurnIntent::End => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::End,
            });
            return;
        }
        TurnIntent::Repeat => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            let line = context
                .memory()
                .last_reply()
                .map(str::to_string)
                .unwrap_or_else(|| NOTHING_TO_REPEAT_LINE.to_string());
            // Said again exactly as it was said; the model is not involved.
            speak_reply(&context, &mut timings, Reply::Fixed(line))
        }
        TurnIntent::Read(what) => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            // The model is not needed for reading; stop nudging it.
            context.keep_model_warm.store(false, Ordering::SeqCst);
            match context.page.read(what) {
                Ok(pieces) if pieces.is_empty() => speak_reply(
                    &context,
                    &mut timings,
                    Reply::Fixed(END_OF_PAGE_LINE.to_string()),
                ),
                Ok(pieces) => {
                    let outcome = speak_reply(&context, &mut timings, Reply::Verbatim(pieces));
                    // Move the reading position on by what was heard to the end.
                    context.page.heard(outcome.finished_pieces);
                    SpokenOutcome {
                        // Page text is not kept: it must not reach a notice or the memory.
                        heard: String::new(),
                        ..outcome
                    }
                }
                // Why the window cannot be read, said aloud.
                Err(sentence) => speak_reply(&context, &mut timings, Reply::Fixed(sentence)),
            }
        }
        TurnIntent::Ask(question) => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            context.memory().push_user(&question);
            let messages = context.memory().messages();
            let outcome = speak_reply(&context, &mut timings, Reply::FromModel(messages));
            // Remember what was actually heard, even when the reply was cut short.
            context.memory().push_assistant(&outcome.heard);
            outcome
        }
    };

    if context.cancel.is_cancelled() {
        return;
    }
    timings.reply_chars = outcome.heard.chars().count();
    context.host.notify(Notice::Turn(timings));
    match outcome.failure {
        Some(problem) => {
            context.host.notify(Notice::Problem(problem));
            context.send(Event::ReplyFailed { turn });
        }
        None => context.send(Event::ReplyFinished { turn }),
    }
}

enum Reply {
    /// Speak this text as it is.
    Fixed(String),
    /// Speak these pieces of page text exactly as they are, in order.
    Verbatim(Vec<String>),
    /// Ask the model and speak its answer as it arrives.
    FromModel(Vec<crate::llm::chat::ChatMessage>),
}

struct SpokenOutcome {
    /// The pieces that began playing, joined. Empty when nothing was heard.
    heard: String,
    /// How many pieces played to their end.
    finished_pieces: usize,
    /// A fixed sentence describing what went wrong, when the reply could not be spoken.
    failure: Option<String>,
}

/// Produce the reply and speak it. The model (when used) runs on this thread and feeds
/// finished pieces to the voice, which runs on a second thread and feeds the speaker, so the
/// first sentence is heard while later ones are still being written.
fn speak_reply(context: &TurnContext, timings: &mut TurnTimings, reply: Reply) -> SpokenOutcome {
    let (piece_tx, piece_rx) = mpsc::channel::<String>();
    let first_piece_at: Mutex<Option<Instant>> = Mutex::new(None);
    let first_audio_at: Mutex<Option<Instant>> = Mutex::new(None);

    let (spoken, model_error) = thread::scope(|scope| {
        let first_audio = &first_audio_at;
        let voice_thread = scope.spawn(move || voice_pieces(context, piece_rx, first_audio));

        // A new reply: what was being said before no longer matters to the echo check.
        context
            .speaking_text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
        let send_piece = |piece: String| {
            first_piece_at
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get_or_insert_with(Instant::now);
            {
                let mut speaking = context
                    .speaking_text
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                speaking.push(' ');
                speaking.push_str(&piece);
            }
            let _ = piece_tx.send(piece);
        };

        let mut model_error: Option<LlmError> = None;
        match reply {
            Reply::Fixed(line) => send_piece(line),
            Reply::Verbatim(pieces) => {
                for piece in pieces {
                    send_piece(piece);
                }
            }
            Reply::FromModel(messages) => {
                // The real question takes over from the keep-warm requests.
                context.keep_model_warm.store(false, Ordering::SeqCst);
                let asked_at = Instant::now();
                let mut first_text_at: Option<Instant> = None;
                let mut chunker = SentenceChunker::default();
                let result = context.chat.stream_chat(
                    &context.model,
                    &messages,
                    &context.cancel,
                    &mut |delta| {
                        first_text_at.get_or_insert_with(Instant::now);
                        for piece in chunker.push(delta) {
                            send_piece(piece);
                        }
                    },
                );
                timings.model_first_text_ms =
                    first_text_at.map(|at| at.duration_since(asked_at).as_millis() as u64);
                match result {
                    Ok(stats) => {
                        timings.model_prompt_ms = Some(stats.prompt_eval_ms);
                        timings.model_load_ms = Some(stats.load_ms);
                        timings.model_prompt_tokens = Some(stats.prompt_tokens);
                        if !context.cancel.is_cancelled() {
                            if let Some(rest) = chunker.finish() {
                                send_piece(rest);
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "conversation reply could not be generated");
                        // Say what is wrong instead of leaving the user in silence.
                        send_piece(
                            match &error {
                                LlmError::Unavailable(_) => MODEL_DOWN_LINE,
                                LlmError::ModelRefused(_) => MODEL_MISSING_LINE,
                                LlmError::Failed(_) => MODEL_FAILED_LINE,
                            }
                            .to_string(),
                        );
                        model_error = Some(error);
                    }
                }
            }
        }
        // Closing the channel tells the voice there is nothing more to say.
        drop(piece_tx);

        let spoken = voice_thread.join().unwrap_or_else(|_| Spoken {
            heard_pieces: 0,
            finished_pieces: 0,
            heard_text: String::new(),
            error: Some(TtsError::Failed(
                "The voice stopped unexpectedly.".to_string(),
            )),
        });
        (spoken, model_error)
    });
    let Spoken {
        finished_pieces,
        heard_text: heard,
        error: voice_error,
        ..
    } = spoken;

    let first_piece = *first_piece_at
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let first_audio = *first_audio_at
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let (Some(piece), Some(audio)) = (first_piece, first_audio) {
        timings.voice_first_audio_ms =
            Some(audio.saturating_duration_since(piece).as_millis() as u64);
    }
    timings.response_ms = first_audio.map(|audio| {
        audio
            .saturating_duration_since(context.utterance.speech_ended_at)
            .as_millis() as u64
    });

    let failure = match (voice_error, model_error) {
        (Some(error), _) => {
            tracing::warn!(error = %error, "conversation reply could not be spoken");
            Some(error.to_string())
        }
        // The model failed but its fixed line was spoken: the user has been told.
        (None, Some(_)) => {
            return SpokenOutcome {
                // A fixed error line is not part of the conversation.
                heard: String::new(),
                finished_pieces: 0,
                failure: None,
            };
        }
        (None, None) => None,
    };
    SpokenOutcome {
        heard,
        finished_pieces,
        failure,
    }
}

/// The voice side of a reply: speak each piece into the speaker as it arrives, announcing
/// the first audio to the state machine.
fn voice_pieces(
    context: &TurnContext,
    pieces: mpsc::Receiver<String>,
    first_audio_at: &Mutex<Option<Instant>>,
) -> Spoken {
    speak_pieces(
        context.voice.as_ref(),
        context.sink.as_ref(),
        pieces,
        &context.cancel,
        context.limits.sentence_gap,
        &mut || {
            *first_audio_at
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
            context.send(Event::ReplyAudioStarted { turn: context.turn });
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::{
        asr::provider::{AsrError, Transcription},
        audio::devices::mock::{RecordingSpeaker, ScriptedMic},
        llm::chat::{mock::MockChat, ChatRole},
        tts::stream::mock::MockVoice,
    };

    const RATE: u32 = 16_000;

    /// Returns a fixed transcript for every utterance and counts how often it was asked.
    struct ScriptedAsr {
        replies: Mutex<Vec<Result<String, AsrError>>>,
        calls: AtomicUsize,
        samples_seen: Mutex<Vec<usize>>,
    }

    impl ScriptedAsr {
        fn saying(lines: &[&str]) -> Self {
            Self {
                replies: Mutex::new(
                    lines
                        .iter()
                        .rev()
                        .map(|line| Ok(line.to_string()))
                        .collect(),
                ),
                calls: AtomicUsize::new(0),
                samples_seen: Mutex::new(Vec::new()),
            }
        }
    }

    impl SpeechToTextProvider for ScriptedAsr {
        fn id(&self) -> &'static str {
            "scripted-asr"
        }

        fn describe(&self) -> String {
            "scripted".to_string()
        }

        fn transcribe(
            &self,
            request: &TranscriptionRequest<'_>,
        ) -> Result<Transcription, AsrError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.samples_seen
                .lock()
                .unwrap()
                .push(request.samples.len());
            let next = self
                .replies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or(Ok(String::new()));
            next.map(|text| Transcription {
                text,
                provider: "scripted-asr",
                fallback_reason: None,
            })
        }
    }

    /// A page whose paragraphs are given by the test. Each read returns what the reading
    /// position calls for and records the request.
    #[derive(Default)]
    struct ScriptedPage {
        paragraphs: Mutex<Vec<String>>,
        /// Index of the next unread paragraph.
        position: Mutex<usize>,
        /// How many paragraphs the last read returned.
        last_read: Mutex<usize>,
        requests: Mutex<Vec<ReadWhat>>,
        refuse_with: Mutex<Option<String>>,
    }

    impl ScriptedPage {
        fn showing(&self, paragraphs: &[&str]) {
            *self.paragraphs.lock().unwrap() = paragraphs.iter().map(|p| p.to_string()).collect();
        }
    }

    impl PageSource for ScriptedPage {
        fn read(&self, what: ReadWhat) -> Result<Vec<String>, String> {
            self.requests.lock().unwrap().push(what);
            if let Some(reason) = self.refuse_with.lock().unwrap().clone() {
                return Err(reason);
            }
            let paragraphs = self.paragraphs.lock().unwrap();
            let position = *self.position.lock().unwrap();
            let pieces: Vec<String> = match what {
                ReadWhat::FromStart => paragraphs.clone(),
                ReadWhat::FromHere => paragraphs[position.min(paragraphs.len())..].to_vec(),
                ReadWhat::Next => paragraphs.get(position).cloned().into_iter().collect(),
                _ => Vec::new(),
            };
            *self.last_read.lock().unwrap() = pieces.len();
            Ok(pieces)
        }

        fn heard(&self, pieces: usize) {
            *self.position.lock().unwrap() += pieces.min(*self.last_read.lock().unwrap());
        }
    }

    /// Stands in for a real echo canceller: reports itself as one, and passes the microphone
    /// through. The scripted microphone carries only what the test puts on it, so there is
    /// no echo to remove.
    struct PretendEchoCancellation;

    impl EchoCanceller for PretendEchoCancellation {
        fn id(&self) -> &'static str {
            "pretend"
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

    struct TestHost {
        mic: Arc<ScriptedMic>,
        speaker: Arc<RecordingSpeaker>,
        asr: Arc<ScriptedAsr>,
        chat: Arc<MockChat>,
        voice: Arc<MockVoice>,
        page: Arc<ScriptedPage>,
        /// Whether the conversation gets an echo canceller, which allows talking over it.
        cancel_echo: AtomicBool,
        fail_open: Option<String>,
        notices: Mutex<Vec<Notice>>,
        opens: AtomicUsize,
    }

    impl TestHost {
        fn new(asr: ScriptedAsr, chat: MockChat, voice: MockVoice) -> Arc<Self> {
            Arc::new(Self {
                mic: Arc::new(ScriptedMic::new(RATE)),
                speaker: Arc::new(RecordingSpeaker::new(RATE)),
                asr: Arc::new(asr),
                chat: Arc::new(chat),
                voice: Arc::new(voice),
                page: Arc::new(ScriptedPage::default()),
                cancel_echo: AtomicBool::new(false),
                fail_open: None,
                notices: Mutex::new(Vec::new()),
                opens: AtomicUsize::new(0),
            })
        }

        fn phases(&self) -> Vec<Phase> {
            self.notices
                .lock()
                .unwrap()
                .iter()
                .filter_map(|notice| match notice {
                    Notice::Phase { phase, .. } => Some(*phase),
                    _ => None,
                })
                .collect()
        }

        fn problems(&self) -> Vec<String> {
            self.notices
                .lock()
                .unwrap()
                .iter()
                .filter_map(|notice| match notice {
                    Notice::Problem(problem) => Some(problem.clone()),
                    _ => None,
                })
                .collect()
        }

        fn stop_reason(&self) -> Option<StopReason> {
            self.notices
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|notice| match notice {
                    Notice::Phase {
                        phase: Phase::Off,
                        reason,
                    } => *reason,
                    _ => None,
                })
        }

        /// What the user would hear: loud speech-level audio, as the energy detector hears it.
        fn user_speaks(&self, duration: Duration) {
            let samples = (duration.as_secs_f64() * RATE as f64) as usize;
            let speech: Vec<f32> = (0..samples)
                .map(|index| 0.3 * (index as f32 * 0.3).sin())
                .collect();
            self.mic.deliver(&speech);
        }

        fn silence(&self, duration: Duration) {
            self.mic
                .deliver(&vec![0.0; (duration.as_secs_f64() * RATE as f64) as usize]);
        }

        /// One complete user turn: a little quiet, speech, then enough silence to end it.
        fn user_turn(&self) {
            self.silence(Duration::from_millis(400));
            self.user_speaks(Duration::from_millis(900));
            self.silence(Duration::from_millis(900));
        }
    }

    impl ConversationHost for TestHost {
        fn open(&self) -> Result<Resources, String> {
            self.opens.fetch_add(1, Ordering::SeqCst);
            if let Some(problem) = &self.fail_open {
                return Err(problem.clone());
            }
            Ok(Resources {
                source: self.mic.clone(),
                sink: self.speaker.clone(),
                detector: Box::new(EnergyDetector::default()),
                asr: self.asr.clone(),
                chat: self.chat.clone(),
                voice: self.voice.clone(),
                page: self.page.clone(),
                echo: if self.cancel_echo.load(Ordering::Relaxed) {
                    Box::new(PretendEchoCancellation)
                } else {
                    Box::new(crate::audio::echo::NoEchoCancellation)
                },
                readiness: test_readiness(),
                model: "test-model".to_string(),
                language: "en".to_string(),
            })
        }

        fn notify(&self, notice: Notice) {
            if !matches!(notice, Notice::Microphone { .. }) {
                self.notices.lock().unwrap().push(notice);
            }
        }
    }

    /// Questions and sentences read as finished; anything ending in "and" as unfinished.
    fn test_readiness() -> ReadinessJudge {
        Arc::new(|text: &str| {
            if text.ends_with(['?', '.', '!']) {
                crate::conversation::assembler::Readiness::Ready
            } else if text.ends_with("and") {
                crate::conversation::assembler::Readiness::NotReady
            } else {
                crate::conversation::assembler::Readiness::Unsure
            }
        })
    }

    fn limits() -> Limits {
        Limits {
            ready_wait: Duration::from_millis(300),
            unsure_wait: Duration::from_millis(500),
            not_ready_wait: Duration::from_millis(800),
            idle: Duration::from_secs(4),
            max_conversation: Duration::from_secs(60),
            max_utterance: Duration::from_secs(10),
            reopen_guard: Duration::from_millis(50),
            sentence_gap: Duration::from_millis(10),
            device_silence: Duration::from_secs(30),
            poll: Duration::from_millis(2),
            warm_interval: Duration::from_millis(40),
            ..Limits::default()
        }
    }

    fn wait_for(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn started(host: &Arc<TestHost>) -> ConversationCore {
        let core = ConversationCore::start(host.clone(), limits());
        core.send(Event::Toggle);
        wait_for("listening", || core.phase() == Phase::Listening);
        core
    }

    #[test]
    fn a_question_is_transcribed_answered_and_spoken_in_order() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["What is a mutex?"]),
            MockChat::replying(&[
                "A mutex is a lock. ",
                "Only one thread ",
                "holds it at a time.",
            ]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the reply to finish", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        assert_eq!(
            host.phases(),
            vec![
                Phase::Starting,
                Phase::Listening,
                Phase::UserSpeaking,
                Phase::Thinking,
                Phase::Speaking,
                Phase::Listening
            ]
        );
        // The model saw the system prompt and the question, and nothing else.
        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].len(), 2);
        assert_eq!(requests[0][0].role, ChatRole::System);
        assert_eq!(requests[0][1].content, "What is a mutex?");
        // The voice spoke whole sentences, in order.
        assert_eq!(
            *host.voice.spoken.lock().unwrap(),
            vec!["A mutex is a lock.", "Only one thread holds it at a time."]
        );
        assert!(!host.speaker.heard.lock().unwrap().is_empty());
        // The recogniser got the speech with its lead-in, not the whole silence around it.
        let samples = host.asr.samples_seen.lock().unwrap()[0];
        assert!(
            samples > RATE as usize * 8 / 10 && samples < RATE as usize * 2,
            "{samples}"
        );
        assert!(host.problems().is_empty());
        core.send(Event::EndRequested);
    }

    #[test]
    fn the_next_question_carries_the_earlier_exchange_as_it_was_heard() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["First question?", "Second question?"]),
            MockChat::replying(&["An answer."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the first reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        host.user_turn();
        wait_for("the second question to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });

        let requests = host.chat.requests.lock().unwrap();
        let contents: Vec<&str> = requests[1][1..]
            .iter()
            .map(|message| message.content.as_str())
            .collect();
        assert_eq!(
            contents,
            vec!["First question?", "An answer.", "Second question?"]
        );
        core.send(Event::EndRequested);
    }

    #[test]
    fn silence_ends_the_conversation_without_calling_any_model() {
        let host = TestHost::new(
            ScriptedAsr::saying(&[]),
            MockChat::replying(&["unused"]),
            MockVoice::default(),
        );
        let core = started(&host);

        host.silence(Duration::from_secs(6));
        wait_for("the idle timeout", || core.phase() == Phase::Off);

        assert_eq!(host.stop_reason(), Some(StopReason::Idle));
        assert_eq!(host.asr.calls.load(Ordering::SeqCst), 0);
        assert!(host.chat.requests.lock().unwrap().is_empty());
        assert!(host.speaker.heard.lock().unwrap().is_empty());
    }

    #[test]
    fn noise_that_transcribes_to_nothing_gets_no_reply() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["", "[BLANK_AUDIO]"]),
            MockChat::replying(&["unused"]),
            MockVoice::default(),
        );
        let core = started(&host);

        for _ in 0..2 {
            host.user_turn();
            wait_for("return to listening", || {
                host.phases()
                    .ends_with(&[Phase::UserSpeaking, Phase::Listening])
            });
            host.notices.lock().unwrap().clear();
        }

        assert_eq!(host.asr.calls.load(Ordering::SeqCst), 2);
        assert!(host.chat.requests.lock().unwrap().is_empty());
        assert!(host.voice.spoken.lock().unwrap().is_empty());
        core.send(Event::EndRequested);
    }

    #[test]
    fn goodbye_ends_the_conversation_without_asking_the_model() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["OK, that's all, thanks."]),
            MockChat::replying(&["unused"]),
            MockVoice::default(),
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the conversation to end", || core.phase() == Phase::Off);

        assert_eq!(host.stop_reason(), Some(StopReason::Goodbye));
        assert!(host.chat.requests.lock().unwrap().is_empty());
    }

    #[test]
    fn repeat_says_the_last_reply_again_without_the_model() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["What is two plus two?", "Say that again."]),
            MockChat::replying(&["Four."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the first reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("the repeat", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        assert_eq!(*host.voice.spoken.lock().unwrap(), vec!["Four.", "Four."]);
        assert_eq!(host.chat.requests.lock().unwrap().len(), 1);
        core.send(Event::EndRequested);
    }

    #[test]
    fn the_hotkey_cuts_a_reply_short_and_only_what_was_heard_is_remembered() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Tell me a long story.", "And then?"]),
            MockChat::replying(&[
                "Once upon a time. ",
                "There was a very long story. ",
                "It went on and on.",
            ]),
            MockVoice {
                sample_rate: RATE,
                chunk_delay: Duration::from_millis(15),
                ..MockVoice::default()
            },
        );
        // Audio stays queued, as on a real speaker, so there is something to cut off.
        host.speaker.hold.store(true, Ordering::Relaxed);
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        core.send(Event::Toggle);
        wait_for("the interruption", || core.phase() == Phase::Listening);

        // The queued speech was thrown away and the voice stopped early.
        assert!(host.speaker.flushes.load(Ordering::Relaxed) >= 1);
        wait_for("the queue to empty", || host.speaker.queued() == 0);
        assert!(host.voice.spoken.lock().unwrap().len() < 3);
        assert_ne!(
            core.phase(),
            Phase::Off,
            "an interruption does not end the conversation"
        );

        // Nothing reached the user's ears, so the reply is not in the history: the next
        // question continues the user's own message.
        host.speaker.release();
        host.user_turn();
        wait_for("the follow-up to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        let requests = host.chat.requests.lock().unwrap();
        let history = &requests[1][1..];
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content, "Tell me a long story. And then?");
        core.send(Event::EndRequested);
    }

    #[test]
    fn the_assistant_does_not_hear_itself() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["A question?"]),
            MockChat::replying(&["A fairly long answer that takes a while to say out loud."]),
            MockVoice {
                sample_rate: RATE,
                chunk_delay: Duration::from_millis(10),
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        // The assistant's own voice comes back through the microphone, with a pause after it
        // that would end a turn if it were being listened to.
        host.user_speaks(Duration::from_millis(1_500));
        host.silence(Duration::from_millis(1_000));
        wait_for("the reply to finish", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        thread::sleep(Duration::from_millis(100));

        assert_eq!(
            host.asr.calls.load(Ordering::SeqCst),
            1,
            "echo started a turn"
        );
        assert_eq!(core.phase(), Phase::Listening);
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_model_that_is_not_running_is_reported_aloud_and_the_conversation_goes_on() {
        let mut chat = MockChat::replying(&[]);
        chat.fail_with = Some(LlmError::Unavailable("not running".to_string()));
        let host = TestHost::new(
            ScriptedAsr::saying(&["Hello there?", "Still there?"]),
            chat,
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the spoken notice", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        assert_eq!(*host.voice.spoken.lock().unwrap(), vec![MODEL_DOWN_LINE]);
        assert_eq!(core.phase(), Phase::Listening);
        // The fixed notice is not remembered as something the assistant said.
        host.user_turn();
        wait_for("the second attempt", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        let requests = host.chat.requests.lock().unwrap();
        assert!(requests[1]
            .iter()
            .all(|message| message.role != ChatRole::Assistant));
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_voice_failure_is_reported_and_returns_to_listening() {
        let voice = MockVoice {
            fail_with: Some(TtsError::Failed(
                "The voice stopped unexpectedly.".to_string(),
            )),
            ..MockVoice::default()
        };
        let host = TestHost::new(
            ScriptedAsr::saying(&["A question?"]),
            MockChat::replying(&["An answer."]),
            voice,
        );
        let core = started(&host);

        host.user_turn();
        wait_for("a problem report", || !host.problems().is_empty());
        wait_for("listening again", || core.phase() == Phase::Listening);

        assert_eq!(host.problems(), vec!["The voice stopped unexpectedly."]);
        assert!(host.speaker.heard.lock().unwrap().is_empty());
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_failed_transcription_is_reported_without_calling_the_model() {
        let asr = ScriptedAsr::saying(&[]);
        asr.replies
            .lock()
            .unwrap()
            .push(Err(AsrError::Failed("engine crashed".to_string())));
        let host = TestHost::new(asr, MockChat::replying(&["unused"]), MockVoice::default());
        let core = started(&host);

        host.user_turn();
        wait_for("a problem report", || !host.problems().is_empty());
        wait_for("listening again", || core.phase() == Phase::Listening);

        assert!(host.chat.requests.lock().unwrap().is_empty());
        core.send(Event::EndRequested);
    }

    #[test]
    fn losing_the_microphone_ends_the_conversation() {
        let host = TestHost::new(
            ScriptedAsr::saying(&[]),
            MockChat::replying(&[]),
            MockVoice::default(),
        );
        let core = started(&host);

        host.mic.healthy.store(false, Ordering::Relaxed);
        wait_for("the conversation to end", || core.phase() == Phase::Off);

        assert_eq!(host.stop_reason(), Some(StopReason::DeviceLost));
    }

    #[test]
    fn a_microphone_that_goes_quiet_without_an_error_is_also_treated_as_lost() {
        let host = TestHost::new(
            ScriptedAsr::saying(&[]),
            MockChat::replying(&[]),
            MockVoice::default(),
        );
        let core = ConversationCore::start(
            host.clone(),
            Limits {
                device_silence: Duration::from_millis(150),
                ..limits()
            },
        );
        core.send(Event::Toggle);

        // No audio is ever delivered.
        wait_for("the conversation to end", || host.stop_reason().is_some());

        assert_eq!(host.stop_reason(), Some(StopReason::DeviceLost));
    }

    #[test]
    fn a_conversation_that_cannot_start_reports_why_and_stays_off() {
        let mut host = TestHost::new(
            ScriptedAsr::saying(&[]),
            MockChat::replying(&[]),
            MockVoice::default(),
        );
        Arc::get_mut(&mut host).unwrap().fail_open =
            Some("No default microphone is available.".to_string());
        let core = ConversationCore::start(host.clone(), limits());

        core.send(Event::Toggle);
        wait_for("the failure", || host.stop_reason().is_some());

        assert_eq!(host.stop_reason(), Some(StopReason::StartFailed));
        assert_eq!(host.problems(), vec!["No default microphone is available."]);
        assert_eq!(core.phase(), Phase::Off);
    }

    #[test]
    fn the_maximum_length_ends_a_conversation_even_mid_reply() {
        let host = TestHost::new(
            ScriptedAsr::saying(&[]),
            MockChat::replying(&[]),
            MockVoice::default(),
        );
        let core = ConversationCore::start(
            host.clone(),
            Limits {
                max_conversation: Duration::from_secs(2),
                idle: Duration::from_secs(60),
                ..limits()
            },
        );
        core.send(Event::Toggle);
        wait_for("listening", || core.phase() == Phase::Listening);

        host.silence(Duration::from_secs(3));
        wait_for("the conversation to end", || core.phase() == Phase::Off);

        assert_eq!(host.stop_reason(), Some(StopReason::MaxDuration));
    }

    #[test]
    fn a_turn_that_never_pauses_is_cut_at_the_maximum_length() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["A very long monologue"]),
            MockChat::replying(&["Noted."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = ConversationCore::start(
            host.clone(),
            Limits {
                max_utterance: Duration::from_secs(2),
                ..limits()
            },
        );
        core.send(Event::Toggle);
        wait_for("listening", || core.phase() == Phase::Listening);

        host.user_speaks(Duration::from_secs(5));
        wait_for("the turn to be answered", || {
            host.asr.calls.load(Ordering::SeqCst) >= 1
        });

        let samples = host.asr.samples_seen.lock().unwrap()[0];
        assert!(samples <= RATE as usize * 5 / 2, "{samples}");
        core.send(Event::EndRequested);
    }

    #[test]
    fn switching_off_while_starting_never_opens_a_session() {
        let host = TestHost::new(
            ScriptedAsr::saying(&[]),
            MockChat::replying(&[]),
            MockVoice::default(),
        );
        let core = ConversationCore::start(host.clone(), limits());

        // On and straight off again, before the devices have reported in.
        core.send(Event::Toggle);
        core.send(Event::Toggle);
        wait_for("the open to be attempted", || {
            host.opens.load(Ordering::SeqCst) == 1
        });
        thread::sleep(Duration::from_millis(100));

        assert_eq!(core.phase(), Phase::Off);
        // Speech now goes nowhere: nothing is listening.
        host.user_turn();
        thread::sleep(Duration::from_millis(200));
        assert_eq!(host.asr.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_model_is_kept_warm_while_the_user_speaks_and_left_alone_otherwise() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["A question?"]),
            MockChat::replying(&["An answer."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);
        let warm = || host.chat.warm_calls.load(Ordering::SeqCst);

        // Waiting in silence: the model is not touched.
        for _ in 0..10 {
            host.silence(Duration::from_millis(30));
            thread::sleep(Duration::from_millis(30));
        }
        assert_eq!(warm(), 0);

        // The user talks for a while, delivered at roughly real-time pace.
        for _ in 0..20 {
            host.user_speaks(Duration::from_millis(30));
            thread::sleep(Duration::from_millis(30));
        }
        assert!(
            warm() >= 3,
            "only {} keep-warm requests while speaking",
            warm()
        );

        // They finish; the question is asked and answered.
        host.silence(Duration::from_millis(900));
        wait_for("the reply to finish", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        let after_turn = warm();

        // Back to waiting: no more nudges.
        for _ in 0..10 {
            host.silence(Duration::from_millis(30));
            thread::sleep(Duration::from_millis(30));
        }
        assert_eq!(warm(), after_turn);
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_page_is_read_word_for_word_without_the_model_or_the_memory() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Read this page to me.", "What did I just ask?"]),
            MockChat::replying(&["You asked me something."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        host.page.showing(&[
            "First paragraph, with **stars** and a [link](x).",
            "Second paragraph.",
        ]);
        let core = started(&host);

        host.user_turn();
        wait_for("the page to be read", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        // Spoken exactly as the page has it: no cleaning, no rewording.
        assert_eq!(
            *host.voice.spoken.lock().unwrap(),
            vec![
                "First paragraph, with **stars** and a [link](x).",
                "Second paragraph."
            ]
        );
        assert!(
            host.chat.requests.lock().unwrap().is_empty(),
            "the model was asked"
        );
        assert_eq!(
            *host.page.position.lock().unwrap(),
            2,
            "position moved past what was heard"
        );

        // The page text is not in what the model is told next.
        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("the follow-up", || {
            host.chat.requests.lock().unwrap().len() == 1
        });
        let requests = host.chat.requests.lock().unwrap();
        let sent = format!("{:?}", requests[0]);
        assert!(!sent.contains("paragraph"));
        core.send(Event::EndRequested);
    }

    #[test]
    fn page_text_never_appears_in_a_notice() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Read this page."]),
            MockChat::replying(&[]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        host.page
            .showing(&["The quarterly figures show a zebra-striped loss."]);
        let core = started(&host);

        host.user_turn();
        wait_for("the page to be read", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        core.send(Event::EndRequested);
        wait_for("off", || core.phase() == Phase::Off);

        let everything = format!("{:?}", host.notices.lock().unwrap());
        assert!(!everything.contains("zebra"));
        assert!(!everything.contains("quarterly"));
    }

    #[test]
    fn interrupting_a_reading_keeps_the_place_at_what_was_heard() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Read this page."]),
            MockChat::replying(&[]),
            MockVoice {
                sample_rate: RATE,
                chunk_delay: Duration::from_millis(15),
                ..MockVoice::default()
            },
        );
        host.page
            .showing(&["Paragraph one.", "Paragraph two.", "Paragraph three."]);
        // Nothing actually plays, so nothing counts as heard.
        host.speaker.hold.store(true, Ordering::Relaxed);
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        core.send(Event::Toggle);
        wait_for("the interruption", || core.phase() == Phase::Listening);
        thread::sleep(Duration::from_millis(150));

        assert_eq!(*host.page.position.lock().unwrap(), 0);
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_window_that_may_not_be_read_is_explained_aloud() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Read this page."]),
            MockChat::replying(&[]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        *host.page.refuse_with.lock().unwrap() =
            Some("I don't read private browsing windows.".to_string());
        let core = started(&host);

        host.user_turn();
        wait_for("the explanation", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        assert_eq!(
            *host.voice.spoken.lock().unwrap(),
            vec!["I don't read private browsing windows."]
        );
        assert_eq!(core.phase(), Phase::Listening);
        core.send(Event::EndRequested);
    }

    #[test]
    fn the_end_of_the_page_is_announced() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Next paragraph.", "Next."]),
            MockChat::replying(&[]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        host.page.showing(&["The only paragraph."]);
        let core = started(&host);

        host.user_turn();
        wait_for("the paragraph", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("the announcement", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        assert_eq!(
            *host.voice.spoken.lock().unwrap(),
            vec!["The only paragraph.", END_OF_PAGE_LINE]
        );
        assert_eq!(
            *host.page.requests.lock().unwrap(),
            vec![ReadWhat::Next, ReadWhat::Next]
        );
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_pause_mid_thought_keeps_the_turn_open_and_the_whole_thought_is_answered() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["I went to the shop and", "bought some bread."]),
            MockChat::replying(&["Nice."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        // First phrase, then a pause long enough to end a finished sentence but not this one.
        host.silence(Duration::from_millis(400));
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(600));
        wait_for("the first phrase to be transcribed", || {
            host.asr.calls.load(Ordering::SeqCst) == 1
        });
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            core.phase(),
            Phase::UserSpeaking,
            "the turn must still be open"
        );
        assert!(host.chat.requests.lock().unwrap().is_empty());

        // The user carries on and finishes.
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(900));
        wait_for("the reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(requests.len(), 1, "one turn, one question");
        assert_eq!(
            requests[0][1].content,
            "I went to the shop and bought some bread."
        );
        // Only the new phrase was transcribed the second time, not the whole turn again.
        let samples = host.asr.samples_seen.lock().unwrap();
        assert_eq!(samples.len(), 2);
        assert!(samples[1] < RATE as usize * 2, "{samples:?}");
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_finished_question_is_answered_sooner_than_something_unclear() {
        let answered_after = |transcript: &'static str| {
            let host = TestHost::new(
                ScriptedAsr::saying(&[transcript]),
                MockChat::replying(&["Reply."]),
                MockVoice {
                    sample_rate: RATE,
                    ..MockVoice::default()
                },
            );
            let core = started(&host);
            host.user_turn();
            wait_for("the reply", || {
                host.phases()
                    .ends_with(&[Phase::Speaking, Phase::Listening])
            });
            let waited = host
                .notices
                .lock()
                .unwrap()
                .iter()
                .find_map(|notice| match notice {
                    Notice::Turn(timings) => Some(timings.endpoint_wait_ms),
                    _ => None,
                })
                .expect("turn timings");
            core.send(Event::EndRequested);
            waited
        };

        // All the audio arrives at once, so the wait recorded is the silence that had been
        // heard when the transcript came back: at least the wait for its kind.
        assert!(answered_after("What time is it?") >= 300);
        assert!(answered_after("maybe the blue one") >= 500);
    }

    /// A host whose conversation can be talked over, with a slow voice so there is time to.
    fn interruptible(asr: &[&str], reply: &[&str]) -> Arc<TestHost> {
        let host = TestHost::new(
            ScriptedAsr::saying(asr),
            MockChat::replying(reply),
            MockVoice {
                sample_rate: RATE,
                chunk_delay: Duration::from_millis(15),
                ..MockVoice::default()
            },
        );
        host.cancel_echo.store(true, Ordering::Relaxed);
        // Audio stays queued, as on a real speaker, so there is a reply to cut off.
        host.speaker.hold.store(true, Ordering::Relaxed);
        host
    }

    #[test]
    fn speaking_over_the_reply_cuts_it_off_and_the_interruption_is_answered() {
        let host = interruptible(
            &["Tell me a long story.", "Actually, what time is it?"],
            &[
                "Once upon a time. ",
                "There was a very long story. ",
                "It went on and on.",
            ],
        );
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        let flushes_before = host.speaker.flushes.load(Ordering::Relaxed);

        // The user talks over the reply, then stops.
        host.user_speaks(Duration::from_millis(900));
        wait_for("the assistant to give way", || {
            core.phase() == Phase::UserSpeaking
        });
        assert!(host.speaker.flushes.load(Ordering::Relaxed) > flushes_before);
        host.speaker.release();
        host.silence(Duration::from_millis(900));

        wait_for("the interruption to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(
            requests[1].last().unwrap().content,
            "Tell me a long story. Actually, what time is it?",
            "nothing of the first reply was heard, so the user's two messages run together"
        );
        // The recogniser heard the interruption from its first word.
        let samples = host.asr.samples_seen.lock().unwrap();
        assert!(samples[1] >= RATE as usize * 9 / 10, "{samples:?}");
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_short_noise_over_the_reply_does_not_cut_it_off() {
        let host = interruptible(&["A question?"], &["A fairly long answer to hear out."]);
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        // A cough: 150 ms, shorter than an interruption must be.
        host.user_speaks(Duration::from_millis(150));
        host.silence(Duration::from_millis(400));
        thread::sleep(Duration::from_millis(150));

        assert_eq!(core.phase(), Phase::Speaking);
        assert_eq!(host.asr.calls.load(Ordering::SeqCst), 1);
        core.send(Event::EndRequested);
    }

    #[test]
    fn the_assistants_own_words_heard_back_are_not_taken_for_an_interruption() {
        // The second "transcript" is a stretch of the reply itself: what a recogniser makes
        // of echo the canceller let through.
        let host = interruptible(
            &["Explain it.", "is a lock that lets only one thread"],
            &["A mutex is a lock that lets only one thread touch the data at a time."],
        );
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        host.user_speaks(Duration::from_millis(900));
        wait_for("the apparent interruption", || {
            core.phase() == Phase::UserSpeaking
        });
        host.speaker.release();
        host.silence(Duration::from_millis(900));

        wait_for("it to be recognised as echo", || {
            core.phase() == Phase::Listening
        });
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            host.chat.requests.lock().unwrap().len(),
            1,
            "the assistant answered its own voice"
        );
        core.send(Event::EndRequested);
    }

    #[test]
    fn without_echo_cancellation_speech_over_the_reply_is_ignored() {
        let host = interruptible(&["A question?"], &["A fairly long answer to hear out."]);
        host.cancel_echo.store(false, Ordering::Relaxed);
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(400));
        thread::sleep(Duration::from_millis(150));

        assert_eq!(core.phase(), Phase::Speaking);
        assert_eq!(host.asr.calls.load(Ordering::SeqCst), 1);
        core.send(Event::EndRequested);
    }

    #[test]
    fn nothing_that_was_said_appears_in_any_notice() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["my password is hunter2"]),
            MockChat::replying(&["I will not repeat the secret zebra phrase."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the reply to finish", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        core.send(Event::EndRequested);
        wait_for("off", || core.phase() == Phase::Off);

        let everything = format!("{:?}", host.notices.lock().unwrap());
        assert!(!everything.contains("hunter2"));
        assert!(!everything.contains("zebra"));
        // The timings notice is there, with lengths but no text.
        assert!(host.notices.lock().unwrap().iter().any(|notice| matches!(
            notice,
            Notice::Turn(timings) if timings.transcript_chars == 22 && timings.reply_chars > 0
        )));
    }
}
