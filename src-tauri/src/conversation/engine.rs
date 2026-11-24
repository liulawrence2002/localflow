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
//!   each user turn, and enforces the idle and maximum-length limits. It keeps listening
//!   after handing a turn over: speech while the reply is prepared or spoken holds the reply
//!   (see `barge_in`), and the ear judges from the words whether to cancel it or let it carry
//!   on.
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
    agents::{AgentDialog, AgentJob, AgentRuntime, HeardTurn, NoAgents, TurnPlan, STILL_WORKING},
    assembler::{Action, AssemblerConfig, ReadinessJudge, TurnAssembler},
    barge_in::{HeldReply, HoldConfig, HoldDecision},
    chunker::SentenceChunker,
    echo_guard::is_echo_of,
    memory::ConversationMemory,
    router::{route_turn, ReadWhat, TurnIntent},
    speaking::{speak_pieces, SpeakerGate, Spoken},
    state::{transition, ConversationState, Effect, Event, Phase, StopReason, TurnOutcome},
};
use crate::{
    asr::provider::{SpeechToTextProvider, TranscriptionRequest},
    audio::{
        devices::{AudioSink, AudioSource},
        echo::{EchoCanceller, ECHO_SAMPLE_RATE},
        resample::StreamResampler,
        smart_turn::TurnCompletionModel,
        spectrum::{SpectrumAnalyzer, SpectrumFrame},
        speech_detector::{EnergyDetector, SpeechDetector, DETECTOR_SAMPLE_RATE},
        turn_segmenter::{SegmentEvent, SegmenterConfig, TurnSegmenter},
    },
    cancel::CancelFlag,
    coding_agents::{speech as agent_speech, AgentCommand, ExecutionOutcome, CONFIRMATION_TIMEOUT},
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
    /// Hears from the voice whether a pause ends the turn (Smart Turn). Together with
    /// `readiness` it sets the wait; without one the words decide alone.
    pub turn_model: Option<Box<dyn TurnCompletionModel>>,
    /// The local model to answer with.
    pub model: String,
    /// Language code for speech recognition.
    pub language: String,
}

/// The turn model hears this much of the turn before a pause: Smart Turn's 8 seconds.
const TURN_MODEL_WINDOW: usize = 8 * DETECTOR_SAMPLE_RATE as usize;

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
    /// How long the user must speak over the assistant before its reply is held. Longer than
    /// the ordinary start of a turn, because a remnant of the assistant's own voice or a
    /// noise in the room must not pause a reply.
    pub barge_in_speech: Duration,
    /// How sure the speech detector must be, from 0 to 1, for speech over the assistant to
    /// begin counting.
    pub barge_in_threshold: f32,
    /// Once begun, speech over the assistant keeps counting while the detector is at least
    /// this sure (Silero's hysteresis: 0.15 below the threshold that started it).
    pub barge_in_keep_threshold: f32,
    /// A dip below the keep threshold no longer than this pauses the count instead of
    /// starting it over.
    pub barge_in_gap: Duration,
    /// A held reply carries on when the user has been quiet this long and their words have
    /// not shown a real interruption (LiveKit's `false_interruption_timeout`).
    pub false_interruption_timeout: Duration,
    /// A reply is never held longer than this. Someone still talking is interrupting.
    pub max_hold: Duration,
    /// Silence after a coding-agent command is read back (or a "which project?") that
    /// declines it.
    pub answer_timeout: Duration,
    /// An answer to a read-back must begin at least this long after the read-back finished,
    /// so the last of the assistant's own "Shall I?", still coming out of the speaker, is not
    /// taken for the user's answer. Kept short because people answer a question quickly; the
    /// read-backs contain no words that count as a yes, so this only guards against a stray
    /// "no".
    pub answer_guard: Duration,
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
            barge_in_keep_threshold: 0.5,
            barge_in_gap: Duration::from_millis(32),
            false_interruption_timeout: Duration::from_millis(1_500),
            max_hold: Duration::from_secs(6),
            answer_timeout: CONFIRMATION_TIMEOUT,
            answer_guard: Duration::from_millis(150),
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
    /// Whether the turn model heard the last pause as finished, coarsely: above or below its
    /// threshold. `None` when it had not answered by the end of the turn, or there is none.
    pub turn_model_finished: Option<bool>,
    /// How long the turn model took on the last pause it answered for.
    pub turn_model_ms: Option<u64>,
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
    /// The coding agents a conversation can drive by voice. None by default.
    fn agents(&self) -> Arc<dyn AgentRuntime> {
        Arc::new(NoAgents)
    }
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
    /// The user finished a turn, whose words are waiting under `utterance`.
    TurnEnded {
        conversation: u64,
        utterance: u64,
    },
}

/// What a turn handed from the ear to the turn worker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UtteranceKind {
    /// Something the user said.
    Speech,
    /// Nobody answered a coding-agent read-back in time. No words.
    AnswerTimedOut,
}

/// One user turn's audio, handed from the ear to the turn worker.
struct Utterance {
    kind: UtteranceKind,
    /// How the turn was heard, which decides whether it can answer a read-back.
    heard: HeardTurn,
    /// Everything the user said, already transcribed while they spoke.
    text: String,
    /// What the user said before this, when they carried on speaking while the reply to it
    /// was being prepared: that reply was cancelled unheard, and the two make one turn.
    continues: Option<String>,
    /// From the first word to the last.
    speech_ms: u64,
    /// Total time the recogniser spent on this turn's phrases.
    transcribe_ms: u64,
    /// When the user stopped speaking, as best the detector can tell.
    speech_ended_at: Instant,
    endpoint_wait: Duration,
    /// What the turn model heard at the last pause, coarsely, and how long it took.
    turn_model_finished: Option<bool>,
    turn_model_ms: Option<u64>,
}

/// Finished turns waiting for the loop, each under its own number.
///
/// The ear may finish a second turn before the loop has started the first (a quick
/// carrying-on), so a single slot could hand a turn the wrong words. Each turn's words are
/// taken by the number its [`Message::TurnEnded`] carried; anything older left behind (a turn
/// the loop did not start) is dropped then.
#[derive(Default)]
struct Utterances {
    next: u64,
    waiting: Vec<(u64, Utterance)>,
}

impl Utterances {
    fn put(&mut self, utterance: Utterance) -> u64 {
        self.next += 1;
        self.waiting.push((self.next, utterance));
        self.next
    }

    fn take(&mut self, id: u64) -> Option<Utterance> {
        let index = self.waiting.iter().position(|(waiting, _)| *waiting == id);
        let taken = index.map(|index| self.waiting.remove(index).1);
        self.waiting.retain(|(waiting, _)| *waiting > id);
        taken
    }
}

/// What stays open for the length of one conversation.
struct Session {
    conversation: u64,
    stop: CancelFlag,
    /// Whether the user has the floor right now (waiting for them or hearing them).
    user_floor: Arc<AtomicBool>,
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
    utterances: Arc<Mutex<Utterances>>,
    source: Arc<dyn AudioSource>,
    /// The speaker itself: paused, resumed and listened back to by the loop and the ear.
    sink: Arc<dyn AudioSink>,
    /// Which turn's reply may play; each reply writes through it.
    gate: Arc<SpeakerGate>,
    asr: Arc<dyn SpeechToTextProvider>,
    chat: Arc<dyn ChatProvider>,
    voice: Arc<dyn StreamingTextToSpeech>,
    page: Arc<dyn PageSource>,
    agents: Arc<dyn AgentRuntime>,
    /// The coding-agent question a reply is waiting on, while there is one.
    agent_job: Arc<Mutex<Option<AgentJob>>>,
    /// A coding-agent read-back has been heard and waits for its answer, so the ear runs the
    /// answer time limit while the user is silent.
    awaiting_answer: Arc<AtomicBool>,
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
    let dialog = Arc::new(Mutex::new(AgentDialog::default()));
    let lock_dialog = || {
        dialog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };

    for message in rx {
        // The words of the turn this message ended, if it ended one.
        let mut ended_utterance = None;
        let event = match message {
            Message::Event(event) => event,
            Message::TurnEnded {
                conversation,
                utterance,
            } => {
                ended_utterance = Some(utterance);
                Event::SpeechEnded { conversation }
            }
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

        // Whether the question a coding-agent read-back asked was heard to its end.
        let current_reply = |turn: u64| {
            matches!(state.phase, Phase::Thinking | Phase::Speaking) && turn == state.turn
        };
        match event {
            Event::ReplyFinished { turn } if current_reply(turn) => {
                lock_dialog().reply_finished(turn)
            }
            Event::ReplyFailed { turn } if current_reply(turn) => lock_dialog().reply_cut(turn),
            _ => {}
        }

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
                    // Exactly the words of the turn that just ended.
                    let utterance = ended_utterance.and_then(|id| {
                        active
                            .utterances
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take(id)
                    });
                    let Some(utterance) = utterance else {
                        let _ = tx.send(Message::Event(Event::ReplyFailed { turn }));
                        continue;
                    };
                    let cancel = CancelFlag::default();
                    turn_cancel = Some(cancel.clone());
                    active.gate.open(turn);
                    let context = TurnContext {
                        turn,
                        utterance,
                        cancel,
                        sink: Arc::new(active.gate.for_turn(turn)),
                        chat: active.chat.clone(),
                        voice: active.voice.clone(),
                        page: active.page.clone(),
                        agents: active.agents.clone(),
                        agent_job: active.agent_job.clone(),
                        dialog: dialog.clone(),
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
                Effect::CancelTurn { turn } => {
                    // Cancelling frees the model, and any coding agent the reply was waiting
                    // for, at once for whatever the user says next.
                    if let Some(cancel) = turn_cancel.take() {
                        cancel.cancel();
                    }
                    if let Some(active) = session.as_ref() {
                        active.gate.close(turn);
                    }
                    // A read-back that was cut off was not heard: it cannot be answered.
                    lock_dialog().reply_cut(turn);
                }
                Effect::PauseReply { turn } => {
                    tracing::info!(turn, "reply held while the user speaks");
                    if let Some(active) = session.as_ref() {
                        active.sink.pause();
                    }
                }
                Effect::ResumeReply { turn } => {
                    tracing::info!(turn, "reply hold released");
                    if let Some(active) = session.as_ref() {
                        active.sink.resume();
                    }
                }
                Effect::Stop { reason } => {
                    stop_reason = Some(reason);
                    if let Some(cancel) = turn_cancel.take() {
                        cancel.cancel();
                    }
                    if let Some(closing) = session.take() {
                        closing.stop.cancel();
                        closing.user_floor.store(false, Ordering::SeqCst);
                        closing.awaiting_answer.store(false, Ordering::SeqCst);
                        closing.gate.close_any();
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
                    lock_dialog().reset();
                }
            }
        }

        if let Some(active) = session.as_ref() {
            if next.phase == Phase::Listening && state.phase != Phase::Listening {
                active.listen_epoch.fetch_add(1, Ordering::SeqCst);
            }
            active
                .user_floor
                .store(next.phase.user_has_floor(), Ordering::SeqCst);
            // The answer time limit runs only while the user has the floor.
            active.awaiting_answer.store(
                next.phase.user_has_floor() && lock_dialog().awaiting_answer(),
                Ordering::SeqCst,
            );
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
        turn_model,
        model,
        language,
    } = resources;
    let session = Session {
        conversation,
        stop: CancelFlag::default(),
        user_floor: Arc::new(AtomicBool::new(false)),
        listen_epoch: Arc::new(AtomicU64::new(0)),
        keep_model_warm: Arc::new(AtomicBool::new(false)),
        assistant_speaking: Arc::new(AtomicBool::new(false)),
        speaking_text: Arc::new(Mutex::new(String::new())),
        utterances: Arc::new(Mutex::new(Utterances::default())),
        source,
        gate: SpeakerGate::new(sink.clone()),
        sink,
        asr,
        chat,
        voice,
        page,
        agents: host.agents(),
        agent_job: Arc::new(Mutex::new(None)),
        awaiting_answer: Arc::new(AtomicBool::new(false)),
        model,
        language,
    };

    let ear = Ear {
        conversation,
        source: session.source.clone(),
        detector,
        stop: session.stop.clone(),
        user_floor: session.user_floor.clone(),
        listen_epoch: session.listen_epoch.clone(),
        utterances: session.utterances.clone(),
        asr: session.asr.clone(),
        language: session.language.clone(),
        readiness,
        turn_model,
        echo,
        sink: session.sink.clone(),
        assistant_speaking: session.assistant_speaking.clone(),
        speaking_text: session.speaking_text.clone(),
        agent_job: session.agent_job.clone(),
        awaiting_answer: session.awaiting_answer.clone(),
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
    // Find the coding agents now, so the first command does not wait for it.
    let agents = session.agents.clone();
    thread::spawn(move || agents.prepare());

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
    user_floor: Arc<AtomicBool>,
    listen_epoch: Arc<AtomicU64>,
    utterances: Arc<Mutex<Utterances>>,
    asr: Arc<dyn SpeechToTextProvider>,
    language: String,
    readiness: ReadinessJudge,
    /// Taken by the turn model's own thread when listening starts.
    turn_model: Option<Box<dyn TurnCompletionModel>>,
    echo: Box<dyn EchoCanceller>,
    sink: Arc<dyn AudioSink>,
    assistant_speaking: Arc<AtomicBool>,
    speaking_text: Arc<Mutex<String>>,
    agent_job: Arc<Mutex<Option<AgentJob>>>,
    awaiting_answer: Arc<AtomicBool>,
    host: Arc<dyn ConversationHost>,
    tx: mpsc::Sender<Message>,
    limits: Limits,
}

/// Where the ear sends audio to be worked on, each on its own thread so listening never
/// stalls.
struct Workers {
    transcribe: mpsc::Sender<TranscribeJob>,
    /// The turn model, when there is one.
    judge: Option<mpsc::Sender<JudgeJob>>,
}

/// The audio of the turn in progress and the bookkeeping that goes with it.
#[derive(Default)]
struct TurnAudio {
    /// 16 kHz audio not yet released. `samples[0]` is position `base` on the audio clock.
    samples: Vec<f32>,
    base: u64,
    /// Where the turn's audio began, for measuring how long the user spoke.
    turn_start: u64,
    /// Where the turn's speech itself began, lead-in aside.
    speech_start: u64,
    /// Recogniser time spent on this turn so far.
    transcribe_ms: u64,
    /// How long the turn model took on its latest answer in this turn.
    turn_model_ms: Option<u64>,
}

impl TurnAudio {
    fn clear(&mut self) {
        self.samples.clear();
        self.base = 0;
        self.turn_start = 0;
        self.speech_start = 0;
        self.transcribe_ms = 0;
        self.turn_model_ms = None;
    }

    /// A copy of the audio in `from..to`, clamped to what is still held.
    fn slice(&self, from: u64, to: u64) -> Vec<f32> {
        let start = from
            .saturating_sub(self.base)
            .min(self.samples.len() as u64) as usize;
        let end = to.saturating_sub(self.base).min(self.samples.len() as u64) as usize;
        self.samples[start..end.max(start)].to_vec()
    }

    /// Drop audio before `position`, except the last few seconds: the turn model hears the
    /// turn's last 8 seconds at every pause, transcribed or not.
    fn release(&mut self, position: u64) {
        let drop = position
            .saturating_sub(self.base)
            .min(self.samples.len().saturating_sub(TURN_MODEL_WINDOW) as u64)
            as usize;
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

/// Where the floor is, as far as the ear can tell. The ear moves on by itself as it hears
/// things happen, and tells the loop. The loop's view (the shared flags) is consulted only
/// for what the ear cannot hear: the floor being handed back, and the reply being audible.
enum Stage {
    /// Not acting on the microphone until the floor is first given to the user.
    Closed,
    /// The user has the floor: finding the start and end of their turn.
    User {
        /// What the user said before, when this turn carries on from it before its reply
        /// was heard (see [`Utterance::continues`]).
        continues: Option<String>,
        /// The turn began while the reply was being heard, so it could be the assistant's
        /// own voice.
        over_reply: bool,
        /// The turn began while the user had the floor, not while the assistant did: only
        /// such a turn can answer a coding-agent read-back.
        own_floor: bool,
    },
    /// A turn has been handed over and the assistant has the floor, preparing or speaking
    /// the reply. Listening only for the user starting to speak.
    Assistant {
        /// Everything the handed-over turn said, in case the user carries on with it.
        said: String,
    },
    /// The user started speaking while the assistant had the floor. The reply is held while
    /// their words are judged.
    Holding {
        held: HeldReply,
        said: String,
        /// The coding-agent question the reply is waiting on, if it is.
        job: Option<AgentJob>,
    },
}

impl Stage {
    fn user() -> Self {
        Self::User {
            continues: None,
            over_reply: false,
            own_floor: true,
        }
    }

    /// Whether a turn of the user's is being heard out.
    fn hears_user(&self) -> bool {
        matches!(self, Self::User { .. } | Self::Holding { .. })
    }
}

/// What the ear keeps from one read of the microphone to the next.
struct Hearing {
    /// Finds the start and end of the user's speech.
    segmenter: TurnSegmenter,
    /// The stricter rule for speech over the assistant's voice. Sees every frame, like the
    /// segmenter, so neither starts cold when the assistant begins or stops speaking.
    guard: TurnSegmenter,
    assembler: TurnAssembler,
    turn: TurnAudio,
    /// Position on the audio clock: 16 kHz samples scored since the last fresh start.
    clock: u64,
    /// Audio skipped at the last fresh start, before the clock began.
    floor_skip: u64,
    stage: Stage,
}

impl Hearing {
    /// Forget the audio so far: the floor has just been handed to the user afresh.
    fn reset(&mut self) {
        self.segmenter.reset();
        self.guard.reset();
        self.assembler.reset();
        self.turn.clear();
        self.clock = 0;
        self.floor_skip = 0;
        self.stage = Stage::user();
    }

    /// End the hold, if there is one, returning it, what the handed-over turn said and the
    /// coding-agent question it was over.
    fn take_hold(&mut self) -> Option<(HeldReply, String, Option<AgentJob>)> {
        match std::mem::replace(&mut self.stage, Stage::Closed) {
            Stage::Holding { held, said, job } => Some((held, said, job)),
            other => {
                self.stage = other;
                None
            }
        }
    }
}

/// One phrase for the recogniser: the request id and its audio.
type TranscribeJob = (u64, Vec<f32>);
/// The recogniser's answer: the id, the text (or failure) and how long it took.
type TranscribeDone = (u64, Result<String, ()>, u64);
/// One pause for the turn model: its number and the turn's audio up to it.
type JudgeJob = (u64, Vec<f32>);
/// The turn model's answer: the pause, the probability that the turn is finished (none when
/// it failed), and how long it took.
type JudgeDone = (u64, Option<f32>, u64);

/// Runs the turn model on its own thread, one pause at a time, beside the recogniser. Only the
/// latest pause matters, so pauses queued behind a slow one are skipped.
fn start_turn_judge(
    mut model: Box<dyn TurnCompletionModel>,
) -> (mpsc::Sender<JudgeJob>, mpsc::Receiver<JudgeDone>) {
    let (job_tx, job_rx) = mpsc::channel::<JudgeJob>();
    let (done_tx, done_rx) = mpsc::channel::<JudgeDone>();
    thread::Builder::new()
        .name("localflow-turn-judge".to_string())
        .spawn(move || {
            while let Ok(mut job) = job_rx.recv() {
                while let Ok(newer) = job_rx.try_recv() {
                    job = newer;
                }
                let (pause, audio) = job;
                let started = Instant::now();
                let probability = match model.probability_complete(&audio) {
                    Ok(probability) => Some(probability),
                    Err(problem) => {
                        // A fixed description: never the audio or a path.
                        tracing::warn!(problem = %problem, "turn model could not judge a pause");
                        None
                    }
                };
                let took = started.elapsed().as_millis() as u64;
                if done_tx.send((pause, probability, took)).is_err() {
                    return;
                }
            }
        })
        .expect("the turn model thread should start");
    (job_tx, done_rx)
}

impl Ear {
    fn send(&self, event: Event) {
        let _ = self.tx.send(Message::Event(event));
    }

    /// The reply being spoken, as text: to tell its echo from the user.
    fn speaking_text(&self) -> String {
        self.speaking_text
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
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
        // Speaking over the assistant's voice has to be surer and last longer than starting a
        // turn. Once begun it may dip, as speech does between syllables, without starting over.
        let guard_config = |frame: Duration, limits: &Limits| SegmenterConfig {
            start_threshold: limits.barge_in_threshold,
            confirm_threshold: Some(limits.barge_in_keep_threshold),
            confirm_gap: limits.barge_in_gap,
            min_speech: limits.barge_in_speech,
            ..SegmenterConfig::conversation(frame)
        };
        let hold_config = HoldConfig {
            sample_rate: DETECTOR_SAMPLE_RATE,
            false_interruption_timeout: self.limits.false_interruption_timeout,
            max_hold: self.limits.max_hold,
        };
        let mut config = segmenter_config(self.detector.frame(), self.limits.pause);
        let mut frame_samples = samples_16k(config.frame).max(1);
        // The turn model, loaded once for the conversation, runs on its own thread.
        let (judge_jobs, judged) = match self.turn_model.take().map(start_turn_judge) {
            Some((jobs, judged)) => (Some(jobs), Some(judged)),
            None => (None, None),
        };
        let mut hearing = Hearing {
            segmenter: TurnSegmenter::new(config),
            guard: TurnSegmenter::new(guard_config(self.detector.frame(), &self.limits)),
            assembler: TurnAssembler::new(
                AssemblerConfig {
                    sample_rate: DETECTOR_SAMPLE_RATE,
                    pre_roll: self.limits.pre_roll,
                    tail: self.limits.tail,
                    ready_wait: self.limits.ready_wait,
                    unsure_wait: self.limits.unsure_wait,
                    not_ready_wait: self.limits.not_ready_wait,
                    max_turn: self.limits.max_utterance,
                    turn_model: judge_jobs.is_some(),
                },
                self.readiness.clone(),
            ),
            turn: TurnAudio::default(),
            clock: 0,
            floor_skip: 0,
            stage: Stage::Closed,
        };
        let (transcribe, done) = self.start_transcriber();
        let jobs = Workers {
            transcribe,
            judge: judge_jobs,
        };
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
        // The last "floor is yours" signal acted on. Zero means none yet.
        let mut seen_epoch = 0u64;
        // The microphone is ignored while the assistant's voice cannot be removed from it.
        let mut deaf = false;
        let mut guard_remaining = 0usize;
        // Audio-clock counters: they advance with samples received, not with wall time.
        let mut idle_samples = 0usize;
        // Silence while a coding-agent read-back waits for its answer.
        let mut answer_samples = 0usize;
        let mut total_native = 0u64;
        let mut picture_samples = 0usize;
        let mut max_reported = false;
        let mut last_audio_at = Instant::now();

        loop {
            if self.stop.is_cancelled() {
                return;
            }
            thread::sleep(self.limits.poll);

            // Transcripts first: they can end a turn or decide a hold even when no new audio
            // has arrived.
            while let Ok((id, result, took_ms)) = done.try_recv() {
                if !hearing.stage.hears_user() {
                    continue;
                }
                let actions = match result {
                    Ok(text) => {
                        hearing.turn.transcribe_ms += took_ms;
                        hearing.assembler.transcribed(id, &text, hearing.clock)
                    }
                    Err(()) => {
                        let actions = hearing.assembler.transcription_failed(id);
                        // Over a held reply a failure only lets the reply carry on.
                        if !actions.is_empty() && matches!(hearing.stage, Stage::User { .. }) {
                            self.host.notify(Notice::Problem(
                                "I couldn't make out that audio.".to_string(),
                            ));
                        }
                        actions
                    }
                };
                self.judge_words(&mut hearing, &actions);
                self.carry_out(actions, &mut hearing, &jobs);
            }
            // The turn model's answers, each for the pause it was asked about. One for an
            // earlier pause, or a turn already over, is ignored by the assembler.
            while let Some(Ok((pause, probability, took_ms))) =
                judged.as_ref().map(mpsc::Receiver::try_recv)
            {
                if !hearing.stage.hears_user() {
                    continue;
                }
                hearing.turn.turn_model_ms = Some(took_ms);
                let actions = hearing
                    .assembler
                    .turn_judged(pause, probability, hearing.clock);
                self.carry_out(actions, &mut hearing, &jobs);
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

            // The floor handed back to the user.
            let epoch = self.listen_epoch.load(Ordering::SeqCst);
            if self.user_floor.load(Ordering::SeqCst) && epoch != seen_epoch {
                seen_epoch = epoch;
                if hearing.stage.hears_user() && hearing.assembler.in_turn() {
                    // ...while they were speaking: the reply ended, failed or was stopped as
                    // they talked. What they are saying is the start of their turn.
                    let (over_reply, own_floor) = match &hearing.stage {
                        Stage::Holding { held, .. } => (held.over_reply(), false),
                        Stage::User {
                            over_reply,
                            own_floor,
                            ..
                        } => (*over_reply, *own_floor),
                        _ => (false, false),
                    };
                    hearing.stage = Stage::User {
                        continues: None,
                        over_reply,
                        own_floor,
                    };
                    idle_samples = 0;
                    answer_samples = 0;
                    self.send(Event::SpeechStarted {
                        conversation: self.conversation,
                    });
                } else {
                    // A fresh start. A transcription still running belongs to the old turn.
                    hearing.reset();
                    self.detector.reset();
                    analyzer.reset();
                    staging.clear();
                    idle_samples = 0;
                    answer_samples = 0;
                    deaf = false;
                    // Without echo cancellation, skip the tail of the assistant's own voice.
                    guard_remaining = if can_barge_in {
                        0
                    } else {
                        samples_16k(self.limits.reopen_guard)
                    };
                    hearing.floor_skip = guard_remaining as u64;
                }
            }
            if matches!(hearing.stage, Stage::Closed) {
                continue;
            }

            // While the assistant has the floor, whether its voice is coming out of the
            // speaker. Audio queued counts at once, before the loop has caught up, so the
            // first of it is never taken for the user.
            let audible =
                || self.assistant_speaking.load(Ordering::SeqCst) || self.sink.queued() > 0;
            if matches!(hearing.stage, Stage::Assistant { .. }) && !can_barge_in && audible() {
                // Its voice cannot be told from the user's: ignore the microphone until the
                // floor comes back.
                deaf = true;
                continue;
            }
            if deaf {
                // The audio skipped while deaf does not join up with what follows.
                deaf = false;
                hearing.segmenter.reset();
                hearing.guard.reset();
                self.detector.reset();
                staging.clear();
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
            if matches!(hearing.stage, Stage::User { .. }) {
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
                hearing.segmenter = TurnSegmenter::new(config);
                hearing.guard =
                    TurnSegmenter::new(guard_config(self.detector.frame(), &self.limits));
                frame_samples = samples_16k(config.frame).max(1);
                staging.clear();
                match hearing.stage {
                    // Nothing more can be judged of it: let the reply carry on.
                    Stage::Holding { .. } => {
                        self.dismiss(&mut hearing);
                    }
                    Stage::User { .. } if hearing.assembler.in_turn() => {
                        hearing.assembler.reset();
                        hearing.stage = Stage::user();
                        self.send(Event::TurnAbandoned {
                            conversation: self.conversation,
                        });
                    }
                    _ => {}
                }
                hearing.turn.clear();
                hearing.clock = 0;
                continue;
            }

            // Each probability scores the next frame of staged audio, in order. Moving the
            // audio across one frame at a time keeps the captured turn exactly in step with
            // the detector, however much audio arrived at once.
            let mut consumed = 0usize;
            for probability in probabilities.iter().copied() {
                let frame_end = (consumed + frame_samples).min(staging.len());
                hearing
                    .turn
                    .samples
                    .extend_from_slice(&staging[consumed..frame_end]);
                hearing.clock += (frame_end - consumed) as u64;
                consumed = frame_end;
                let frame = frame_samples as u64;
                let heard = hearing.segmenter.observe(probability);
                let guarded = hearing.guard.observe(probability);

                if matches!(hearing.stage, Stage::Assistant { .. }) {
                    // Only one thing matters here: does the user start to speak? Over the
                    // assistant's voice the stricter rule decides.
                    let over_reply = audible();
                    if over_reply && !can_barge_in {
                        deaf = true;
                        break;
                    }
                    hearing.turn.keep_last(
                        samples_16k(self.limits.pre_roll)
                            + samples_16k(self.limits.barge_in_speech),
                    );
                    let started = if over_reply { guarded } else { heard };
                    if let Some(SegmentEvent::SpeechStarted { frames_ago }) = started {
                        let start = hearing
                            .clock
                            .saturating_sub((frames_ago as u64 + 1) * frame);
                        self.begin_hold(&mut hearing, start, over_reply, hold_config, &jobs);
                    }
                    continue;
                }

                let actions = match heard {
                    Some(SegmentEvent::SpeechStarted { frames_ago }) => {
                        if let Stage::Holding { held, .. } = &mut hearing.stage {
                            held.user_resumed();
                        }
                        let start = hearing
                            .clock
                            .saturating_sub((frames_ago as u64 + 1) * frame);
                        if !hearing.assembler.in_turn() {
                            hearing.turn.speech_start = start;
                        }
                        hearing.assembler.speech_started(start)
                    }
                    Some(SegmentEvent::SpeechEnded { frames_ago }) => {
                        let speech_end = hearing.clock.saturating_sub(frames_ago as u64 * frame);
                        if let Stage::Holding { held, .. } = &mut hearing.stage {
                            held.user_paused(speech_end);
                        }
                        hearing.assembler.pause_started(speech_end, hearing.clock)
                    }
                    None => hearing.assembler.tick(hearing.clock),
                };
                self.carry_out(actions, &mut hearing, &jobs);
                // A hold that has lasted too long, or a silence after which nothing was decided.
                if let Stage::Holding { held, .. } = &mut hearing.stage {
                    let decision = held.tick(hearing.clock);
                    self.decide(&mut hearing, decision);
                }

                if matches!(hearing.stage, Stage::User { .. }) {
                    if !hearing.assembler.in_turn() {
                        // Waiting: keep only a short lead-in, and count the silence.
                        hearing.turn.keep_last(samples_16k(self.limits.pre_roll));
                        idle_samples += frame_samples;
                        if idle_samples >= samples_16k(self.limits.idle) {
                            idle_samples = 0;
                            self.send(Event::IdleTimeout {
                                conversation: self.conversation,
                            });
                        }
                        // A coding-agent read-back nobody answers is declined, aloud.
                        if self.awaiting_answer.load(Ordering::SeqCst) {
                            answer_samples += frame_samples;
                            if answer_samples >= samples_16k(self.limits.answer_timeout) {
                                answer_samples = 0;
                                idle_samples = 0;
                                self.answer_timed_out(&mut hearing);
                            }
                        } else {
                            answer_samples = 0;
                        }
                    } else {
                        idle_samples = 0;
                        answer_samples = 0;
                    }
                }
            }
            staging.drain(..consumed);
        }
    }

    /// The user has started to speak while the assistant had the floor: hold the reply and
    /// hear them out. `over_reply` when the reply was audible as they started.
    fn begin_hold(
        &self,
        hearing: &mut Hearing,
        start: u64,
        over_reply: bool,
        config: HoldConfig,
        jobs: &Workers,
    ) {
        let said = match std::mem::replace(&mut hearing.stage, Stage::Closed) {
            Stage::Assistant { said } => said,
            other => {
                hearing.stage = other;
                return;
            }
        };
        self.send(Event::BargeIn {
            conversation: self.conversation,
        });
        // While the reply waits for a coding agent, only "stop that" does anything.
        let job = self
            .agent_job
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let held = match job {
            Some(_) => HeldReply::while_agent_works(config, hearing.clock, over_reply),
            None => HeldReply::new(config, hearing.clock, over_reply),
        };
        hearing.stage = Stage::Holding { held, said, job };
        // The stricter rule may have confirmed it; the ordinary one must report the pause.
        hearing.segmenter.assume_speaking();
        hearing.turn.speech_start = start;
        let actions = hearing.assembler.speech_started(start);
        self.carry_out(actions, hearing, jobs);
    }

    /// Judge the words heard over a held reply as soon as each part is transcribed, so a
    /// real interruption stops the reply at once and an "mm-hmm" lets it carry on.
    fn judge_words(&self, hearing: &mut Hearing, actions: &[Action]) {
        // Nothing new (a transcript for a turn already dropped), or the speech is over and
        // is judged as a whole when that is carried out.
        if actions.is_empty()
            || !matches!(hearing.stage, Stage::Holding { .. })
            || actions
                .iter()
                .any(|action| matches!(action, Action::Commit { .. } | Action::Abandon))
        {
            return;
        }
        let more_coming = actions
            .iter()
            .any(|action| matches!(action, Action::Transcribe { .. }));
        let speaking = self.speaking_text();
        let Stage::Holding { held, .. } = &mut hearing.stage else {
            return;
        };
        let decision = held.heard(hearing.assembler.text(), &speaking, more_coming);
        self.decide(hearing, decision);
    }

    fn decide(&self, hearing: &mut Hearing, decision: HoldDecision) {
        match decision {
            HoldDecision::Wait => {}
            HoldDecision::Interrupt => self.confirm(hearing),
            HoldDecision::Resume => {
                self.dismiss(hearing);
            }
            HoldDecision::StopAgent => {
                if let Some(job) = self.dismiss(hearing) {
                    tracing::info!(turn = job.turn, "coding agent question stopped by voice");
                    job.stop();
                }
            }
            HoldDecision::RemindAgentBusy => {
                if let Some(job) = self.dismiss(hearing) {
                    job.ask_for_reminder();
                }
            }
        }
    }

    /// A real interruption: the reply is cancelled and the user's speech goes on as their
    /// turn.
    fn confirm(&self, hearing: &mut Hearing) {
        let Some((held, said, _)) = hearing.take_hold() else {
            return;
        };
        tracing::info!(
            verdict = ?held.verdict(),
            over_reply = held.over_reply(),
            "interruption confirmed; reply cancelled"
        );
        self.send(Event::InterruptionConfirmed {
            conversation: self.conversation,
        });
        hearing.stage = Stage::User {
            // Before any of the reply was heard, this is the rest of what was said before it.
            continues: (!held.over_reply() && !said.is_empty()).then_some(said),
            over_reply: held.over_reply(),
            own_floor: false,
        };
    }

    /// A false interruption: the reply carries on, and what was heard over it is dropped.
    /// Returns the coding-agent question the reply waits on, if it does.
    fn dismiss(&self, hearing: &mut Hearing) -> Option<AgentJob> {
        let (held, said, job) = hearing.take_hold()?;
        tracing::info!(
            verdict = ?held.verdict(),
            agent_verdict = ?held.agent_verdict(),
            over_reply = held.over_reply(),
            "false interruption; the reply carries on"
        );
        self.send(Event::FalseInterruption {
            conversation: self.conversation,
        });
        // A transcription still running belongs to what was dropped.
        hearing.assembler.reset();
        hearing.segmenter.reset();
        hearing.guard.reset();
        hearing.stage = Stage::Assistant { said };
        job
    }

    /// Do what the assembler asked, as fits the stage the ear is in.
    fn carry_out(&self, actions: Vec<Action>, hearing: &mut Hearing, jobs: &Workers) {
        for action in actions {
            if !hearing.stage.hears_user() {
                // A hold was just dismissed: the rest belongs to what was dropped.
                if let Action::Release { before } = action {
                    hearing.turn.release(before);
                }
                continue;
            }
            match action {
                Action::TurnStarted => {
                    hearing.turn.turn_start = hearing.turn.base;
                    hearing.turn.transcribe_ms = 0;
                    hearing.turn.turn_model_ms = None;
                    // Speech over the assistant was announced as a barge-in instead.
                    if matches!(hearing.stage, Stage::User { .. }) {
                        self.send(Event::SpeechStarted {
                            conversation: self.conversation,
                        });
                    }
                }
                Action::Transcribe { id, from, to } => {
                    let _ = jobs.transcribe.send((id, hearing.turn.slice(from, to)));
                }
                Action::JudgeTurn { pause, from, to } => {
                    if let Some(judge) = &jobs.judge {
                        let from = from.max(to.saturating_sub(TURN_MODEL_WINDOW as u64));
                        let _ = judge.send((pause, hearing.turn.slice(from, to)));
                    }
                }
                Action::Release { before } => hearing.turn.release(before),
                Action::Commit {
                    text,
                    speech_end,
                    waited,
                    sounded_finished,
                } => {
                    if let Stage::Holding { held, .. } = &mut hearing.stage {
                        // The user has finished: this is all there is to judge.
                        let decision = held.finished(&text, &self.speaking_text());
                        if decision != HoldDecision::Interrupt {
                            self.decide(hearing, decision);
                            continue;
                        }
                        self.confirm(hearing);
                    }
                    self.hand_over(hearing, text, speech_end, waited, sounded_finished);
                }
                Action::Abandon => {
                    if matches!(hearing.stage, Stage::Holding { .. }) {
                        self.dismiss(hearing);
                    } else {
                        hearing.stage = Stage::user();
                        self.send(Event::TurnAbandoned {
                            conversation: self.conversation,
                        });
                    }
                }
            }
        }
    }

    /// Give a finished turn to the loop, under its own number.
    fn send_turn(&self, utterance: Utterance) {
        let id = self
            .utterances
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .put(utterance);
        let _ = self.tx.send(Message::TurnEnded {
            conversation: self.conversation,
            utterance: id,
        });
    }

    /// The user's turn is over: give it to the loop. The ear goes on listening, in case they
    /// carry on before the reply is heard.
    fn hand_over(
        &self,
        hearing: &mut Hearing,
        text: String,
        speech_end: u64,
        waited: Duration,
        sounded_finished: Option<bool>,
    ) {
        let (continues, over_reply, own_floor) =
            match std::mem::replace(&mut hearing.stage, Stage::Closed) {
                Stage::User {
                    continues,
                    over_reply,
                    own_floor,
                } => (continues, over_reply, own_floor),
                other => {
                    hearing.stage = other;
                    return;
                }
            };
        // A turn begun over the reply that is only the assistant's own words, heard back
        // through the microphone, is not a turn.
        if over_reply && is_echo_of(&text, &self.speaking_text()) {
            tracing::info!("an interruption was the assistant's own voice; ignored");
            hearing.stage = Stage::user();
            self.send(Event::TurnAbandoned {
                conversation: self.conversation,
            });
            return;
        }
        let said = match &continues {
            Some(earlier) => format!("{earlier} {text}"),
            None => text.clone(),
        };
        let began = hearing.turn.speech_start + hearing.floor_skip;
        let utterance = Utterance {
            kind: UtteranceKind::Speech,
            heard: HeardTurn {
                began_after_floor: own_floor
                    .then(|| Duration::from_secs_f64(began as f64 / DETECTOR_SAMPLE_RATE as f64)),
            },
            text,
            continues,
            speech_ms: speech_end.saturating_sub(hearing.turn.turn_start) * 1_000
                / DETECTOR_SAMPLE_RATE as u64,
            transcribe_ms: hearing.turn.transcribe_ms,
            speech_ended_at: Instant::now()
                .checked_sub(waited)
                .unwrap_or_else(Instant::now),
            endpoint_wait: waited,
            turn_model_finished: sounded_finished,
            turn_model_ms: hearing.turn.turn_model_ms,
        };
        self.send_turn(utterance);
        // Whatever the stricter rule made of the user's own turn must not delay noticing
        // them speak over the reply.
        hearing.guard.reset();
        hearing.stage = Stage::Assistant { said };
    }

    /// Nobody answered a coding-agent read-back in time. This is handed over as a turn with
    /// no words, so the assistant can say so as it would answer anything else.
    fn answer_timed_out(&self, hearing: &mut Hearing) {
        if !matches!(hearing.stage, Stage::User { .. }) || hearing.assembler.in_turn() {
            return;
        }
        self.send(Event::SpeechStarted {
            conversation: self.conversation,
        });
        self.send_turn(Utterance {
            kind: UtteranceKind::AnswerTimedOut,
            heard: HeardTurn::default(),
            text: String::new(),
            continues: None,
            speech_ms: 0,
            transcribe_ms: 0,
            speech_ended_at: Instant::now(),
            endpoint_wait: Duration::ZERO,
            turn_model_finished: None,
            turn_model_ms: None,
        });
        hearing.guard.reset();
        hearing.stage = Stage::Assistant {
            said: String::new(),
        };
    }
}

struct TurnContext {
    turn: u64,
    utterance: Utterance,
    cancel: CancelFlag,
    /// The speaker, for as long as this turn's reply may use it.
    sink: Arc<dyn AudioSink>,
    chat: Arc<dyn ChatProvider>,
    voice: Arc<dyn StreamingTextToSpeech>,
    page: Arc<dyn PageSource>,
    agents: Arc<dyn AgentRuntime>,
    agent_job: Arc<Mutex<Option<AgentJob>>>,
    dialog: Arc<Mutex<AgentDialog>>,
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

    fn agent_job(&self) -> std::sync::MutexGuard<'_, Option<AgentJob>> {
        self.agent_job
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// What the coding-agent dialog makes of this turn, before anything else sees it.
    ///
    /// Only the user's own words, as the microphone heard them, are given to it. Checked under
    /// its lock: a turn already cancelled must not move the dialog on, because the loop
    /// resets a question whose asking turn was cut off.
    fn plan(&self, transcript: &str) -> Option<TurnPlan> {
        let blank = crate::asr::whisper_text::is_blank_transcript(transcript);
        if blank && self.utterance.kind == UtteranceKind::Speech {
            return Some(TurnPlan::PassOn);
        }
        // Read before the lock: the first time, it may wait for the agents to be found.
        let commands = self.agents.context();
        let mut dialog = self
            .dialog
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if self.cancel.is_cancelled() {
            return None;
        }
        Some(match self.utterance.kind {
            UtteranceKind::AnswerTimedOut => dialog.timed_out(self.turn, &commands),
            UtteranceKind::Speech => dialog.user_said(
                self.turn,
                transcript,
                self.utterance.heard,
                self.limits.answer_guard,
                &commands,
            ),
        })
    }
}

/// How a reply ended, as far as the loop is concerned.
enum ReplyEnd {
    Finished,
    /// It opened a coding agent: the conversation steps aside.
    HandedToAgent,
}

fn run_turn(context: TurnContext) {
    let turn = context.turn;
    let mut timings = TurnTimings {
        turn,
        speech_ms: context.utterance.speech_ms,
        endpoint_wait_ms: context.utterance.endpoint_wait.as_millis() as u64,
        transcribe_ms: context.utterance.transcribe_ms,
        turn_model_finished: context.utterance.turn_model_finished,
        turn_model_ms: context.utterance.turn_model_ms,
        ..Default::default()
    };
    // Transcribed phrase by phrase while the user was speaking. When they carried on with
    // what they had said before, while its reply was being prepared, the two are one turn
    // and are understood together: "read this page" ... "from the top".
    let transcript = match &context.utterance.continues {
        Some(earlier) => format!("{earlier} {}", context.utterance.text),
        None => context.utterance.text.clone(),
    };
    timings.transcript_chars = transcript.chars().count();

    let Some(plan) = context.plan(&transcript) else {
        return;
    };
    let mut end = ReplyEnd::Finished;
    let outcome = match plan {
        TurnPlan::Ignore => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::NoReply,
            });
            return;
        }
        TurnPlan::Say(line) => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            context.keep_model_warm.store(false, Ordering::SeqCst);
            // About a coding-agent command: spoken, and not part of what the model is told.
            speak_reply(&context, &mut timings, Reply::Fixed(line))
        }
        TurnPlan::Execute(command) => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            context.keep_model_warm.store(false, Ordering::SeqCst);
            let Some((outcome, hand_over)) = carry_out_command(&context, &mut timings, &command)
            else {
                return;
            };
            if hand_over {
                end = ReplyEnd::HandedToAgent;
            }
            outcome
        }
        TurnPlan::PassOn => match answer(&context, &mut timings, &transcript) {
            Some(outcome) => outcome,
            None => return,
        },
    };

    if context.cancel.is_cancelled() {
        return;
    }
    timings.reply_chars = outcome.heard.chars().count();
    context.host.notify(Notice::Turn(timings));
    match (outcome.failure, end) {
        (Some(problem), _) => {
            context.host.notify(Notice::Problem(problem));
            context.send(Event::ReplyFailed { turn });
        }
        (None, ReplyEnd::Finished) => context.send(Event::ReplyFinished { turn }),
        (None, ReplyEnd::HandedToAgent) => context.send(Event::HandedToAgent { turn }),
    }
}

/// An ordinary turn: a control phrase, a reading, or a question for the model. `None` when
/// the turn has nothing more to report.
fn answer(
    context: &TurnContext,
    timings: &mut TurnTimings,
    transcript: &str,
) -> Option<SpokenOutcome> {
    let turn = context.turn;
    let intent = if crate::asr::whisper_text::is_blank_transcript(transcript) {
        TurnIntent::Nothing
    } else {
        route_turn(transcript)
    };

    Some(match intent {
        TurnIntent::Nothing | TurnIntent::Quiet => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::NoReply,
            });
            return None;
        }
        TurnIntent::End => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::End,
            });
            return None;
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
            speak_reply(context, timings, Reply::Fixed(line))
        }
        TurnIntent::Read(what) => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            // The model is not needed for reading; stop nudging it.
            context.keep_model_warm.store(false, Ordering::SeqCst);
            match context.page.read(what) {
                Ok(pieces) if pieces.is_empty() => {
                    speak_reply(context, timings, Reply::Fixed(END_OF_PAGE_LINE.to_string()))
                }
                Ok(pieces) => {
                    let outcome = speak_reply(context, timings, Reply::Verbatim(pieces));
                    // Move the reading position on by what was heard to the end.
                    context.page.heard(outcome.finished_pieces);
                    SpokenOutcome {
                        // Page text is not kept: it must not reach a notice or the memory.
                        heard: String::new(),
                        ..outcome
                    }
                }
                // Why the window cannot be read, said aloud.
                Err(sentence) => speak_reply(context, timings, Reply::Fixed(sentence)),
            }
        }
        TurnIntent::Ask(question) => {
            context.send(Event::TurnRouted {
                turn,
                outcome: TurnOutcome::Reply,
            });
            let messages = {
                let mut memory = context.memory();
                // Checked under the lock: a turn cancelled because the user carried on must
                // never add its words after the turn that continues it has added its own.
                if context.cancel.is_cancelled() {
                    return None;
                }
                match &context.utterance.continues {
                    // The model is asked the whole of it, as one message.
                    Some(earlier) => {
                        memory.push_user_continuing(turn, earlier, &context.utterance.text)
                    }
                    None => memory.push_user(turn, &question),
                }
                memory.messages()
            };
            let outcome = speak_reply(context, timings, Reply::FromModel(messages));
            // Remember what was actually heard, even when the reply was cut short. Recorded
            // under this turn, so it lands before the user's next words however late it is.
            context.memory().push_assistant(turn, &outcome.heard);
            outcome
        }
    })
}

/// Carry out a coding-agent command the user confirmed, saying what happens. Returns what was
/// spoken and whether the conversation should now step aside for the agent; `None` when the
/// turn was cancelled.
fn carry_out_command(
    context: &TurnContext,
    timings: &mut TurnTimings,
    command: &AgentCommand,
) -> Option<(SpokenOutcome, bool)> {
    if let Some(line) = agent_speech::starting(command) {
        speak_reply(context, timings, Reply::Fixed(line));
    }
    if context.cancel.is_cancelled() {
        return None;
    }
    let outcome = match command {
        // Opening returns at once.
        AgentCommand::Open { .. } => context.agents.execute(command, &context.cancel),
        // A question takes a while, and the user can stop it.
        AgentCommand::Ask { .. } => wait_for_agent(context, timings, command)?,
    };
    if context.cancel.is_cancelled() {
        return None;
    }
    // The agent's answer is only ever spoken, after "Claude says:", and never acted on.
    let line = agent_speech::outcome(command, &outcome);
    let spoken = speak_reply(context, timings, Reply::Fixed(line));
    Some((spoken, outcome.ends_conversation(command)))
}

/// Run a background question, keeping the floor but listening, until it answers, the user
/// says "stop that", or the turn is cancelled. Reminds the user how to stop it if they talk
/// meanwhile. `None` when the turn was cancelled.
fn wait_for_agent(
    context: &TurnContext,
    timings: &mut TurnTimings,
    command: &AgentCommand,
) -> Option<ExecutionOutcome> {
    let job = AgentJob::new(context.turn);
    *context.agent_job() = Some(job.clone());
    context.send(Event::ReplyWaiting { turn: context.turn });

    let (done_tx, done_rx) = mpsc::channel();
    let agents = context.agents.clone();
    let asked = command.clone();
    let stop = job.cancel_flag().clone();
    thread::Builder::new()
        .name("localflow-coding-agent".to_string())
        .spawn(move || {
            let _ = done_tx.send(agents.execute(&asked, &stop));
        })
        .expect("the coding agent thread should start");

    let outcome = loop {
        match done_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(outcome) => break Some(outcome),
            Err(mpsc::RecvTimeoutError::Disconnected) => break None,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if context.cancel.is_cancelled() {
                    // The hotkey, or the end of the conversation: stop the agent and its
                    // children now, then wait for it to go.
                    job.stop();
                } else if job.take_reminder() {
                    speak_reply(context, timings, Reply::Fixed(STILL_WORKING.to_string()));
                    if !context.cancel.is_cancelled() {
                        context.send(Event::ReplyWaiting { turn: context.turn });
                    }
                }
            }
        }
    };
    let mut registered = context.agent_job();
    if registered
        .as_ref()
        .is_some_and(|job| job.turn == context.turn)
    {
        *registered = None;
    }
    drop(registered);
    if context.cancel.is_cancelled() {
        return None;
    }
    outcome
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
    use std::{collections::VecDeque, sync::atomic::AtomicUsize};

    use super::*;
    use crate::{
        asr::provider::{AsrError, Transcription},
        audio::devices::mock::{RecordingSpeaker, ScriptedMic},
        coding_agents::{
            agent::AskOutcome,
            projects::{CodingProject, ProjectIndex},
            AgentAvailability, AgentKind, CommandContext,
        },
        llm::chat::{mock::MockChat, ChatRole},
        tts::stream::mock::MockVoice,
    };

    /// What a scripted turn model answers, in order, and what it was asked.
    #[derive(Default)]
    struct TurnScript {
        /// How long each call takes and what it returns. Calls past the end fail.
        answers: Mutex<VecDeque<(Duration, Result<f32, String>)>>,
        calls: AtomicUsize,
        answered: AtomicUsize,
        audio_lengths: Mutex<Vec<usize>>,
    }

    impl TurnScript {
        fn answering(answers: &[(u64, Result<f32, &str>)]) -> Arc<Self> {
            Arc::new(Self {
                answers: Mutex::new(
                    answers
                        .iter()
                        .map(|(delay_ms, answer)| {
                            (
                                Duration::from_millis(*delay_ms),
                                answer.map_err(str::to_string),
                            )
                        })
                        .collect(),
                ),
                ..Self::default()
            })
        }
    }

    struct FakeTurnModel(Arc<TurnScript>);

    impl TurnCompletionModel for FakeTurnModel {
        fn id(&self) -> &'static str {
            "fake-turn-model"
        }

        fn probability_complete(&mut self, audio: &[f32]) -> Result<f32, String> {
            let script = &self.0;
            script.calls.fetch_add(1, Ordering::SeqCst);
            script.audio_lengths.lock().unwrap().push(audio.len());
            let next = script.answers.lock().unwrap().pop_front();
            let (delay, answer) =
                next.unwrap_or((Duration::ZERO, Err("no more answers".to_string())));
            thread::sleep(delay);
            script.answered.fetch_add(1, Ordering::SeqCst);
            answer
        }
    }

    /// Coding agents that record what they were asked to do and start nothing.
    struct FakeAgents {
        executed: Mutex<Vec<AgentCommand>>,
        /// A question's answer, after a short while; `None` to answer only when stopped.
        answer: Option<String>,
        question_started: AtomicBool,
        stopped: AtomicBool,
        prepared: AtomicUsize,
    }

    impl FakeAgents {
        fn answering(answer: Option<&str>) -> Arc<Self> {
            Arc::new(Self {
                executed: Mutex::new(Vec::new()),
                answer: answer.map(str::to_string),
                question_started: AtomicBool::new(false),
                stopped: AtomicBool::new(false),
                prepared: AtomicUsize::new(0),
            })
        }

        fn executed(&self) -> Vec<AgentCommand> {
            self.executed.lock().unwrap().clone()
        }
    }

    impl AgentRuntime for FakeAgents {
        fn prepare(&self) {
            self.prepared.fetch_add(1, Ordering::SeqCst);
        }

        fn context(&self) -> CommandContext {
            let project = |id: &str, path: &str, name: &str| CodingProject {
                id: id.into(),
                path: path.into(),
                spoken_names: vec![name.into()],
            };
            CommandContext {
                projects: ProjectIndex::new(&[
                    project("lf", r"C:\work\localflow", "localflow"),
                    project("a1", r"C:\work\app", "my app"),
                    project("a2", r"C:\personal\app", "my app"),
                ]),
                availability: AgentAvailability::all(),
            }
        }

        fn execute(&self, command: &AgentCommand, cancel: &CancelFlag) -> ExecutionOutcome {
            self.executed.lock().unwrap().push(command.clone());
            if let AgentCommand::Open { .. } = command {
                return ExecutionOutcome::Opened;
            }
            self.question_started.store(true, Ordering::SeqCst);
            let started = Instant::now();
            loop {
                if cancel.is_cancelled() {
                    self.stopped.store(true, Ordering::SeqCst);
                    return ExecutionOutcome::Answered(AskOutcome::Cancelled);
                }
                match &self.answer {
                    Some(answer) if started.elapsed() >= Duration::from_millis(150) => {
                        return ExecutionOutcome::Answered(AskOutcome::Answer(answer.clone()))
                    }
                    // A safety net so a broken test cannot hang.
                    None if started.elapsed() >= Duration::from_secs(20) => {
                        return ExecutionOutcome::Answered(AskOutcome::TimedOut)
                    }
                    _ => thread::sleep(Duration::from_millis(5)),
                }
            }
        }
    }

    const RATE: u32 = 16_000;

    /// Returns a fixed transcript for every utterance and counts how often it was asked.
    struct ScriptedAsr {
        replies: Mutex<Vec<Result<String, AsrError>>>,
        /// How long each call takes, in order. Calls past the end are instant.
        delays: Mutex<Vec<Duration>>,
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
                delays: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
                samples_seen: Mutex::new(Vec::new()),
            }
        }

        fn taking(self, delays: &[Duration]) -> Self {
            *self.delays.lock().unwrap() = delays.iter().rev().copied().collect();
            self
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
            let delay = self.delays.lock().unwrap().pop();
            if let Some(delay) = delay {
                thread::sleep(delay);
            }
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
        agents: Mutex<Option<Arc<FakeAgents>>>,
        /// A turn model to hear pauses with, if any.
        turn_script: Mutex<Option<Arc<TurnScript>>>,
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
                agents: Mutex::new(None),
                turn_script: Mutex::new(None),
            })
        }

        /// Give the conversation a turn model that answers as `script` says.
        fn with_turn_model(self: Arc<Self>, script: &Arc<TurnScript>) -> Arc<Self> {
            *self.turn_script.lock().unwrap() = Some(script.clone());
            self
        }

        /// Give the conversation coding agents to drive.
        fn with_agents(self: Arc<Self>, agents: &Arc<FakeAgents>) -> Arc<Self> {
            *self.agents.lock().unwrap() = Some(agents.clone());
            self
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
                turn_model: self
                    .turn_script
                    .lock()
                    .unwrap()
                    .clone()
                    .map(|script| Box::new(FakeTurnModel(script)) as Box<dyn TurnCompletionModel>),
                model: "test-model".to_string(),
                language: "en".to_string(),
            })
        }

        fn notify(&self, notice: Notice) {
            if !matches!(notice, Notice::Microphone { .. }) {
                self.notices.lock().unwrap().push(notice);
            }
        }

        fn agents(&self) -> Arc<dyn AgentRuntime> {
            match self.agents.lock().unwrap().clone() {
                Some(agents) => agents,
                None => Arc::new(NoAgents),
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

    const LONG_STORY: &[&str] = &[
        "Once upon a time. ",
        "There was a very long story. ",
        "It went on and on.",
    ];

    #[test]
    fn speaking_over_the_reply_holds_it_and_a_real_interruption_is_answered() {
        // Changed by design: the reply used to be cut off as soon as speech was heard.
        let host = interruptible(
            &["Tell me a long story.", "Actually, what time is it?"],
            LONG_STORY,
        );
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        let flushes_before = host.speaker.flushes.load(Ordering::SeqCst);

        // The user talks over the reply: it is paused, not thrown away, until their words
        // are known.
        host.user_speaks(Duration::from_millis(900));
        wait_for("the reply to be held", || host.speaker.is_paused());
        thread::sleep(Duration::from_millis(50));
        assert_eq!(core.phase(), Phase::Speaking);
        assert_eq!(host.speaker.flushes.load(Ordering::SeqCst), flushes_before);

        // They stop; what they said is a real interruption.
        host.silence(Duration::from_millis(900));
        wait_for("the interruption to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        assert!(host.speaker.flushes.load(Ordering::SeqCst) > flushes_before);
        assert!(!host.speaker.is_paused(), "the next reply can be heard");
        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(
            requests[1].last().unwrap().content,
            "Tell me a long story. Actually, what time is it?",
            "nothing of the first reply was heard, so the user's two messages run together"
        );
        // The recogniser heard the interruption from its first word.
        let samples = host.asr.samples_seen.lock().unwrap();
        assert!(samples[1] >= RATE as usize * 9 / 10, "{samples:?}");
        drop(samples);
        drop(requests);
        host.speaker.release();
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_backchannel_over_the_reply_lets_it_carry_on_where_it_stopped() {
        let host = interruptible(
            &["Tell me a long story.", "Mm-hmm.", "And then?"],
            LONG_STORY,
        );
        let core = started(&host);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        let flushes_before = host.speaker.flushes.load(Ordering::SeqCst);

        host.user_speaks(Duration::from_millis(500));
        host.silence(Duration::from_millis(900));
        wait_for("the backchannel to be heard", || {
            host.asr.calls.load(Ordering::SeqCst) == 2
        });
        wait_for("the reply to carry on", || {
            host.speaker.resumes.load(Ordering::SeqCst) >= 1
        });

        assert_eq!(host.speaker.pauses.load(Ordering::SeqCst), 1);
        assert_eq!(
            host.speaker.flushes.load(Ordering::SeqCst),
            flushes_before,
            "nothing was cut off"
        );
        assert_eq!(core.phase(), Phase::Speaking);
        host.speaker.release();
        wait_for("the reply to finish", || core.phase() == Phase::Listening);
        // Every sentence was spoken, and "mm-hmm" never became a question.
        assert_eq!(host.voice.spoken.lock().unwrap().len(), 3);
        assert_eq!(host.chat.requests.lock().unwrap().len(), 1);
        // The story was heard in full, so it is remembered in full.
        host.user_turn();
        wait_for("the next question", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(
            requests[1][2].content,
            "Once upon a time. There was a very long story. It went on and on."
        );
        core.send(Event::EndRequested);
    }

    #[test]
    fn talking_over_the_reply_for_too_long_cancels_it() {
        let host = interruptible(
            &[
                "Tell me a long story.",
                "Let me tell you about my day instead.",
            ],
            LONG_STORY,
        );
        let core = ConversationCore::start(
            host.clone(),
            Limits {
                max_hold: Duration::from_millis(1_000),
                ..limits()
            },
        );
        core.send(Event::Toggle);
        wait_for("listening", || core.phase() == Phase::Listening);

        host.user_turn();
        wait_for("speaking", || core.phase() == Phase::Speaking);
        let flushes_before = host.speaker.flushes.load(Ordering::SeqCst);

        // On and on without the pause that would let the words be judged.
        host.user_speaks(Duration::from_millis(2_000));
        wait_for("the reply to be given up", || {
            core.phase() == Phase::UserSpeaking
        });
        assert!(host.speaker.flushes.load(Ordering::SeqCst) > flushes_before);
        assert_eq!(
            host.asr.calls.load(Ordering::SeqCst),
            1,
            "decided before any of it was transcribed"
        );

        host.speaker.release();
        host.silence(Duration::from_millis(900));
        wait_for("what they said to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_single_word_after_the_reply_has_ended_is_an_answer() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Tell me something.", "Yeah."]),
            MockChat::replying(&["Do you want to hear a joke?"]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        host.cancel_echo.store(true, Ordering::Relaxed);
        let core = started(&host);

        host.user_turn();
        wait_for("the reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        host.user_turn();
        wait_for("the answer to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });

        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(requests[1].last().unwrap().content, "Yeah.");
        core.send(Event::EndRequested);
    }

    #[test]
    fn carrying_on_while_the_reply_is_prepared_makes_one_turn_of_both_parts() {
        let mut chat = MockChat::replying(&["Paris."]);
        // A slow model, so there is time to carry on before any of the reply is ready.
        chat.piece_delay = Duration::from_millis(400);
        let host = TestHost::new(
            ScriptedAsr::saying(&["What is", "the capital of France?"]),
            chat,
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        // A pause long enough to end the turn, though the question is not finished.
        host.user_turn();
        wait_for("the first part to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 1
        });
        assert_eq!(core.phase(), Phase::Thinking);
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(900));

        wait_for("the whole question to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 2
        });
        wait_for("the reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        let requests = host.chat.requests.lock().unwrap();
        // One question, as one message: the reply to its first half was never heard.
        assert_eq!(requests[1].len(), 2);
        assert_eq!(requests[1][1].content, "What is the capital of France?");
        // The first reply was cancelled before it said anything.
        assert_eq!(*host.voice.spoken.lock().unwrap(), vec!["Paris."]);
        assert!(!host.phases()[..host.phases().len() - 2].contains(&Phase::Speaking));
        core.send(Event::EndRequested);
    }

    #[test]
    fn carrying_on_with_a_command_while_the_reply_is_prepared_is_understood_whole() {
        let host = TestHost::new(
            ScriptedAsr::saying(&["Read this page", "from the top."]),
            MockChat::replying(&["Unused."]),
            // Slow to start speaking, so there is time to carry on first.
            MockVoice {
                sample_rate: RATE,
                chunk_delay: Duration::from_millis(400),
                chunk_samples: 4_000,
                ..MockVoice::default()
            },
        );
        host.page
            .showing(&["First paragraph.", "Second paragraph."]);
        let core = started(&host);

        host.user_turn();
        wait_for("thinking", || core.phase() == Phase::Thinking);
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(900));
        wait_for("the page to be read", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });

        // "Read this page from the top" asks for the start; the model is never involved.
        assert!(host
            .page
            .requests
            .lock()
            .unwrap()
            .contains(&ReadWhat::FromStart));
        assert!(host.chat.requests.lock().unwrap().is_empty());
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_noise_while_the_reply_is_prepared_holds_its_audio_then_lets_it_play() {
        let mut chat = MockChat::replying(&["A mutex is a lock."]);
        chat.piece_delay = Duration::from_millis(150);
        // The noise takes a while to transcribe, so the reply is ready while it is judged.
        let asr = ScriptedAsr::saying(&["What is a mutex?", ""])
            .taking(&[Duration::ZERO, Duration::from_millis(800)]);
        let host = TestHost::new(
            asr,
            chat,
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        let core = started(&host);

        host.user_turn();
        wait_for("thinking", || core.phase() == Phase::Thinking);
        // A chair scraping: loud enough to sound like the start of speech.
        host.user_speaks(Duration::from_millis(300));
        host.silence(Duration::from_millis(700));

        // The reply is produced meanwhile but held: queued, and none of it played.
        wait_for("the reply's audio to be held", || host.speaker.queued() > 0);
        assert!(host.speaker.is_paused());
        assert!(host.speaker.heard.lock().unwrap().is_empty());

        // The noise held no words: the reply plays after all.
        wait_for("the reply to finish", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        assert_eq!(host.chat.requests.lock().unwrap().len(), 1);
        assert_eq!(
            *host.voice.spoken.lock().unwrap(),
            vec!["A mutex is a lock."]
        );
        assert!(!host.speaker.heard.lock().unwrap().is_empty());
        assert_eq!(host.speaker.pauses.load(Ordering::SeqCst), 1);
        assert_eq!(host.speaker.flushes.load(Ordering::SeqCst), 0);
        // The noise was heard from its start, lead-in included.
        let samples = host.asr.samples_seen.lock().unwrap();
        assert!(samples[1] >= RATE as usize * 3 / 10, "{samples:?}");
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
        let flushes_before = host.speaker.flushes.load(Ordering::SeqCst);
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(900));

        // Changed by design: the reply is held, recognised as echo, and carries on, where it
        // used to be cut off first and then found to be echo.
        wait_for("it to be recognised as echo", || {
            host.speaker.resumes.load(Ordering::SeqCst) >= 1
        });
        assert_eq!(host.speaker.pauses.load(Ordering::SeqCst), 1);
        assert_eq!(host.speaker.flushes.load(Ordering::SeqCst), flushes_before);
        host.speaker.release();
        wait_for("the reply to finish", || core.phase() == Phase::Listening);
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

    fn utterance(text: &str) -> Utterance {
        Utterance {
            kind: UtteranceKind::Speech,
            heard: HeardTurn::default(),
            text: text.to_string(),
            continues: None,
            speech_ms: 0,
            transcribe_ms: 0,
            speech_ended_at: Instant::now(),
            endpoint_wait: Duration::ZERO,
            turn_model_finished: None,
            turn_model_ms: None,
        }
    }

    #[test]
    fn finished_turns_are_handed_over_by_number_and_never_mixed_up() {
        let mut store = Utterances::default();
        // A second turn finishes before the loop has started the first.
        let first = store.put(utterance("What is"));
        let second = store.put(utterance("the capital of France?"));

        assert_eq!(store.take(first).unwrap().text, "What is");
        assert_eq!(store.take(second).unwrap().text, "the capital of France?");
        assert!(store.take(first).is_none());

        // A turn the loop never started is dropped once a later one is taken.
        let skipped = store.put(utterance("lost"));
        let third = store.put(utterance("kept"));
        assert_eq!(store.take(third).unwrap().text, "kept");
        assert!(store.take(skipped).is_none());
        assert!(store.waiting.is_empty());
    }

    const READ_BACK: &str =
        "Open Claude Code in localflow? Then dictate your task in the new tab. Shall I?";
    const OPENED: &str =
        "Opened Claude Code in localflow. Dictate your task in the new tab and press Enter.";

    /// A conversation that can drive the fake coding agents.
    fn with_agents(asr: &[&str], reply: &[&str], agents: &Arc<FakeAgents>) -> Arc<TestHost> {
        TestHost::new(
            ScriptedAsr::saying(asr),
            MockChat::replying(reply),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        )
        .with_agents(agents)
    }

    fn spoken(host: &TestHost) -> Vec<String> {
        host.voice.spoken.lock().unwrap().clone()
    }

    fn reply_done(host: &TestHost) -> bool {
        host.phases()
            .ends_with(&[Phase::Speaking, Phase::Listening])
    }

    #[test]
    fn a_command_runs_only_on_yes_and_opening_claude_code_ends_the_conversation() {
        let agents = FakeAgents::answering(None);
        let host = with_agents(&["Open Claude Code in localflow", "Yes."], &[], &agents);
        let core = started(&host);
        wait_for("the agents to be looked for", || {
            agents.prepared.load(Ordering::SeqCst) == 1
        });

        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        assert_eq!(spoken(&host), [READ_BACK]);
        assert!(agents.executed().is_empty(), "nothing runs before yes");

        host.user_turn();
        wait_for("the conversation to step aside", || {
            core.phase() == Phase::Off
        });

        let executed = agents.executed();
        assert_eq!(executed.len(), 1);
        assert_eq!(executed[0].agent(), AgentKind::ClaudeCode);
        assert_eq!(executed[0].project().id, "lf");
        assert_eq!(spoken(&host), [READ_BACK, OPENED]);
        assert_eq!(host.stop_reason(), Some(StopReason::AgentOpened));
        assert!(
            host.chat.requests.lock().unwrap().is_empty(),
            "the model was asked"
        );
    }

    #[test]
    fn a_declined_command_does_nothing_and_never_reaches_the_model() {
        let agents = FakeAgents::answering(None);
        let host = with_agents(
            &["Open Claude Code in localflow", "No.", "What is a mutex?"],
            &["A lock."],
            &agents,
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("the refusal", || reply_done(&host));
        assert_eq!(spoken(&host), [READ_BACK, "OK, I won't."]);
        assert!(agents.executed().is_empty());

        // The conversation goes on, and the model sees none of the command exchange.
        host.user_turn();
        wait_for("the question to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 1
        });
        let requests = host.chat.requests.lock().unwrap();
        assert_eq!(requests[0].len(), 2);
        assert_eq!(requests[0][1].content, "What is a mutex?");
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_read_back_nobody_answers_is_declined_aloud() {
        let agents = FakeAgents::answering(None);
        let host = with_agents(&["Open Claude Code in localflow"], &[], &agents);
        let core = ConversationCore::start(
            host.clone(),
            Limits {
                answer_timeout: Duration::from_secs(1),
                ..limits()
            },
        );
        core.send(Event::Toggle);
        wait_for("listening", || core.phase() == Phase::Listening);

        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        host.notices.lock().unwrap().clear();
        // Silence, longer than the answer may take.
        host.silence(Duration::from_millis(1_500));
        wait_for("the time limit", || reply_done(&host));

        assert_eq!(
            spoken(&host),
            [READ_BACK, "I didn't hear a yes, so I've left it."]
        );
        assert!(agents.executed().is_empty());
        assert_eq!(core.phase(), Phase::Listening, "the conversation goes on");
        core.send(Event::EndRequested);
    }

    #[test]
    fn an_unclear_project_is_asked_about_before_the_read_back() {
        let agents = FakeAgents::answering(None);
        let host = with_agents(
            &["Open Claude Code in my app", "The one in personal.", "Yes."],
            &[],
            &agents,
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the question back", || reply_done(&host));
        assert!(
            spoken(&host)[0].ends_with("Which one?"),
            "{:?}",
            spoken(&host)
        );
        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        assert!(spoken(&host)[1].starts_with("Open Claude Code in my app?"));
        assert!(agents.executed().is_empty());

        host.user_turn();
        wait_for("the conversation to step aside", || {
            core.phase() == Phase::Off
        });
        assert_eq!(agents.executed()[0].project().id, "a2");
    }

    const ASK: &str = "Ask Claude what the overlay code in localflow does";
    const STARTING: &str = "Asking Claude. Say stop that to cancel.";

    #[test]
    fn a_question_for_claude_runs_in_the_background_and_its_answer_is_spoken() {
        let agents = FakeAgents::answering(Some("It **draws** the waveform."));
        let host = with_agents(&[ASK, "Yes.", "What is a mutex?"], &["A lock."], &agents);
        let core = started(&host);

        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        assert!(spoken(&host)[0].ends_with("Shall I send it?"));
        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("the answer", || reply_done(&host));

        assert_eq!(
            spoken(&host)[1..],
            [STARTING, "Claude says: It draws the waveform."]
        );
        // While Claude worked the assistant kept the floor but listened.
        let phases = host.phases();
        assert!(
            phases
                .windows(2)
                .any(|pair| pair == [Phase::Speaking, Phase::Thinking]),
            "{phases:?}"
        );
        match &agents.executed()[0] {
            AgentCommand::Ask { question, .. } => {
                assert_eq!(question, "what the overlay code in localflow does")
            }
            other => panic!("{other:?}"),
        }
        // Neither the question nor Claude's answer is in what the model is told.
        host.user_turn();
        wait_for("the next question", || {
            host.chat.requests.lock().unwrap().len() == 1
        });
        assert_eq!(host.chat.requests.lock().unwrap()[0].len(), 2);
        core.send(Event::EndRequested);
    }

    /// Ask the fake agents a question that only ends when it is stopped, and wait until it
    /// is running.
    fn asking(asr: &[&str], agents: &Arc<FakeAgents>) -> (Arc<TestHost>, ConversationCore) {
        let host = with_agents(asr, &[], agents);
        let core = started(&host);
        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        host.user_turn();
        wait_for("Claude to be working", || {
            agents.question_started.load(Ordering::SeqCst) && core.phase() == Phase::Thinking
        });
        (host, core)
    }

    #[test]
    fn while_claude_works_only_stop_that_stops_it_and_other_speech_gets_a_reminder() {
        let agents = FakeAgents::answering(None);
        let (host, core) = asking(&[ASK, "Yes.", "What time is it?", "Stop that."], &agents);

        // Something else: Claude carries on, and the user is told how to stop it.
        host.user_turn();
        wait_for("the reminder", || {
            spoken(&host).contains(&STILL_WORKING.to_string()) && core.phase() == Phase::Thinking
        });
        assert!(!agents.stopped.load(Ordering::SeqCst));

        host.notices.lock().unwrap().clear();
        host.user_turn();
        wait_for("Claude to stop", || agents.stopped.load(Ordering::SeqCst));
        wait_for("the conversation to carry on", || reply_done(&host));
        assert_eq!(spoken(&host).last().unwrap(), "Stopped.");
        assert!(
            host.chat.requests.lock().unwrap().is_empty(),
            "neither remark went to the model"
        );
        assert_eq!(core.phase(), Phase::Listening);
        core.send(Event::EndRequested);
    }

    #[test]
    fn the_hotkey_stops_claude_at_once() {
        let agents = FakeAgents::answering(None);
        let (host, core) = asking(&[ASK, "Yes."], &agents);
        let said_before = spoken(&host).len();

        core.send(Event::Toggle);
        wait_for("Claude to stop", || agents.stopped.load(Ordering::SeqCst));
        wait_for("the floor to come back", || {
            core.phase() == Phase::Listening
        });
        thread::sleep(Duration::from_millis(100));
        assert_eq!(spoken(&host).len(), said_before, "nothing more is said");
        core.send(Event::EndRequested);
    }

    #[test]
    fn ending_the_conversation_stops_claude_and_forgets_the_question() {
        let agents = FakeAgents::answering(None);
        let (host, core) = asking(&[ASK, "Yes."], &agents);
        core.send(Event::EndRequested);
        wait_for("Claude to stop", || agents.stopped.load(Ordering::SeqCst));
        wait_for("off", || core.phase() == Phase::Off);
        drop(host);
    }

    #[test]
    fn page_text_and_model_replies_never_reach_the_command_rules() {
        let agents = FakeAgents::answering(None);
        let host = with_agents(
            &["Read this page.", "What should I do?", "Yes."],
            &["Open Claude Code in localflow."],
            &agents,
        );
        host.page.showing(&["Open Claude Code in localflow."]);
        let core = started(&host);

        for _ in 0..3 {
            host.notices.lock().unwrap().clear();
            host.user_turn();
            wait_for("the reply", || reply_done(&host));
        }

        // The page was read, the model answered twice, and nothing was read back or run.
        assert_eq!(
            spoken(&host),
            [
                "Open Claude Code in localflow.",
                "Open Claude Code in localflow.",
                "Open Claude Code in localflow."
            ]
        );
        assert!(agents.executed().is_empty());
        // "Yes." was an ordinary answer to the model.
        assert_eq!(
            host.chat.requests.lock().unwrap()[1]
                .last()
                .unwrap()
                .content,
            "Yes."
        );
        core.send(Event::EndRequested);
    }

    #[test]
    fn nothing_said_over_the_read_back_can_confirm_it() {
        let agents = FakeAgents::answering(None);
        // Over the read-back: the user's "yes" (a backchannel), then the read-back's own
        // words coming back through the microphone. Then a real answer afterwards.
        let host = interruptible(
            &["Open Claude Code in localflow", "Yes.", "Shall I?", "Yes."],
            &[],
        )
        .with_agents(&agents);
        let core = started(&host);

        host.user_turn();
        wait_for("the read-back", || core.phase() == Phase::Speaking);
        for heard in 1..=2 {
            host.user_speaks(Duration::from_millis(600));
            host.silence(Duration::from_millis(900));
            wait_for("the read-back to carry on", || {
                host.speaker.resumes.load(Ordering::SeqCst) >= heard
            });
        }
        assert_eq!(core.phase(), Phase::Speaking);
        host.speaker.release();
        wait_for("the read-back to finish", || {
            core.phase() == Phase::Listening
        });
        thread::sleep(Duration::from_millis(100));
        assert!(agents.executed().is_empty(), "confirmed over the read-back");

        // A yes said afterwards, in the user's own turn, does.
        host.user_turn();
        wait_for("the command", || !agents.executed().is_empty());
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_question_pending_when_the_conversation_ends_is_forgotten() {
        let agents = FakeAgents::answering(None);
        let host = with_agents(
            &["Open Claude Code in localflow", "Yes."],
            &["Hello."],
            &agents,
        );
        let core = started(&host);

        host.user_turn();
        wait_for("the read-back", || reply_done(&host));
        core.send(Event::EndRequested);
        wait_for("off", || core.phase() == Phase::Off);
        core.send(Event::Toggle);
        wait_for("listening again", || core.phase() == Phase::Listening);

        host.user_turn();
        wait_for("the yes to reach the model", || {
            host.chat.requests.lock().unwrap().len() == 1
        });
        assert!(agents.executed().is_empty());
        core.send(Event::EndRequested);
    }

    /// A conversation whose pauses are also heard by a scripted turn model.
    fn hearing_turns(asr: &[&str], script: Option<&Arc<TurnScript>>) -> Arc<TestHost> {
        let host = TestHost::new(
            ScriptedAsr::saying(asr),
            MockChat::replying(&["Reply."]),
            MockVoice {
                sample_rate: RATE,
                ..MockVoice::default()
            },
        );
        match script {
            Some(script) => host.with_turn_model(script),
            None => host,
        }
    }

    /// Some speech, then `silence_ms` of quiet: a pause that may or may not end the turn.
    fn speak_then_pause(host: &TestHost, silence_ms: u64) {
        host.silence(Duration::from_millis(400));
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(silence_ms));
    }

    fn asked(host: &TestHost) -> usize {
        host.chat.requests.lock().unwrap().len()
    }

    /// Wait long enough for every answer to be in, then report whether the turn is still open.
    fn still_open_after_a_while(host: &TestHost, core: &ConversationCore) -> bool {
        thread::sleep(Duration::from_millis(300));
        asked(host) == 0 && core.phase() == Phase::UserSpeaking
    }

    fn last_timings(host: &TestHost) -> TurnTimings {
        host.notices
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(|notice| match notice {
                Notice::Turn(timings) => Some(*timings),
                _ => None,
            })
            .expect("turn timings")
    }

    #[test]
    fn a_pause_that_sounds_finished_ends_an_unclear_turn_at_the_short_wait() {
        let script = TurnScript::answering(&[(0, Ok(0.95))]);
        let host = hearing_turns(&["maybe the blue one"], Some(&script));
        let core = started(&host);

        // 420 ms: past the short wait (300 ms), short of the unsure one (500 ms).
        speak_then_pause(&host, 420);
        wait_for("the turn to end", || asked(&host) == 1);

        // The model heard the whole turn so far: lead-in, speech and the phrase's tail.
        let heard = script.audio_lengths.lock().unwrap()[0];
        assert!(
            heard >= RATE as usize * 12 / 10 && heard <= TURN_MODEL_WINDOW,
            "{heard}"
        );
        wait_for("the reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        assert_eq!(last_timings(&host).turn_model_finished, Some(true));
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_pause_that_sounds_unfinished_holds_a_finished_reading_longer() {
        let script = TurnScript::answering(&[(0, Ok(0.05))]);
        // The recogniser's full stop, where the speaker only took a breath.
        let host = hearing_turns(&["I think we should."], Some(&script));
        let core = started(&host);

        speak_then_pause(&host, 420);
        wait_for("the model's answer", || {
            script.answered.load(Ordering::SeqCst) == 1
        });
        assert!(
            still_open_after_a_while(&host, &core),
            "ended at the short wait"
        );

        // At the unsure wait it ends after all.
        host.silence(Duration::from_millis(200));
        wait_for("the turn to end", || asked(&host) == 1);
        core.send(Event::EndRequested);
    }

    #[test]
    fn an_answer_about_an_earlier_pause_is_ignored() {
        // The first pause's answer is slow, and arrives after the user has carried on.
        let script = TurnScript::answering(&[(400, Ok(0.99)), (0, Ok(0.02))]);
        let host = hearing_turns(&["What time", "is it?"], Some(&script));
        let core = started(&host);

        speak_then_pause(&host, 300);
        wait_for("the first pause to be heard", || {
            script.calls.load(Ordering::SeqCst) == 1
        });
        host.user_speaks(Duration::from_millis(900));
        host.silence(Duration::from_millis(420));
        wait_for("both answers", || {
            script.answered.load(Ordering::SeqCst) == 2
        });

        // "Finished" belonged to the first pause. Applied to the second it would end the turn
        // at the short wait.
        assert!(
            still_open_after_a_while(&host, &core),
            "a stale answer was used"
        );
        host.silence(Duration::from_millis(200));
        wait_for("the turn to end", || asked(&host) == 1);
        assert_eq!(
            host.chat.requests.lock().unwrap()[0][1].content,
            "What time is it?"
        );
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_late_turn_model_never_holds_the_turn_up() {
        let script = TurnScript::answering(&[(3_000, Ok(0.02))]);
        let host = hearing_turns(&["What time is it?"], Some(&script));
        let core = started(&host);

        speak_then_pause(&host, 900);
        wait_for("the turn to end", || asked(&host) == 1);
        assert_eq!(
            script.answered.load(Ordering::SeqCst),
            0,
            "the turn waited for the model"
        );
        wait_for("the reply", || {
            host.phases()
                .ends_with(&[Phase::Speaking, Phase::Listening])
        });
        assert_eq!(last_timings(&host).turn_model_finished, None);
        core.send(Event::EndRequested);
    }

    #[test]
    fn a_turn_model_that_fails_leaves_the_words_to_decide() {
        let script = TurnScript::answering(&[(0, Err("broken"))]);
        let host = hearing_turns(&["What time is it?"], Some(&script));
        let core = started(&host);

        // A finished reading gets the short wait, as with no model at all.
        speak_then_pause(&host, 420);
        wait_for("the turn to end", || asked(&host) == 1);
        assert_eq!(script.calls.load(Ordering::SeqCst), 1);
        core.send(Event::EndRequested);
    }

    #[test]
    fn without_a_turn_model_the_words_decide_as_before() {
        // A finished reading ends at the short wait...
        let host = hearing_turns(&["What time is it?"], None);
        let core = started(&host);
        speak_then_pause(&host, 420);
        wait_for("the turn to end", || asked(&host) == 1);
        core.send(Event::EndRequested);

        // ...and an unclear one waits the unsure time.
        let host = hearing_turns(&["maybe the blue one"], None);
        let core = started(&host);
        speak_then_pause(&host, 420);
        wait_for("the transcript", || {
            host.asr.calls.load(Ordering::SeqCst) == 1
        });
        assert!(still_open_after_a_while(&host, &core));
        host.silence(Duration::from_millis(200));
        wait_for("the turn to end", || asked(&host) == 1);
        core.send(Event::EndRequested);
    }
}
