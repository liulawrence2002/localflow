//! In-process speech-to-text through transcribe.cpp (ggml).
//!
//! This is the engine for NVIDIA Parakeet GGUF models. The model is loaded once and kept in
//! memory, so each dictation pays only for inference. Compared with the small Whisper models
//! it is far more accurate and writes its own punctuation and capitalisation.
//!
//! The model and session live inside the LocalFlow process. A run that passes its deadline is
//! cancelled cooperatively through the library's abort callback rather than by killing a
//! process. The library does no networking and its native logging is disabled, so neither
//! audio nor text leaves this module except through the returned transcript.
//!
//! **Compute device.** A default build runs on the CPU. A build with the `gpu-vulkan` Cargo
//! feature runs on a discrete GPU through Vulkan, chosen explicitly by [`choose_gpu`]:
//! integrated GPUs are never used. The CPU is the fallback whenever no discrete GPU exists,
//! loading on it fails, or a run on it fails (that run is retried once on the CPU); each
//! fallback writes one log line with the reason and never any transcript. The voice-activity model (`audio/speech_detector.rs`) stays on the
//! CPU in either build, because transcribe.cpp resolves an automatic backend request for a
//! Silero model to the CPU.

use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex, Once, TryLockError,
    },
    thread,
    time::{Duration, Instant},
};

use transcribe_cpp::{
    Backend, CancelToken, DeviceType, ErrorKind, Feature, Model, ModelOptions, RunOptions, Session,
    SessionOptions, Transcript,
};

use super::provider::{
    transcription_deadline, AsrError, SpeechToTextProvider, Transcription, TranscriptionRequest,
};

pub const TRANSCRIBE_CPP_PROVIDER_ID: &str = "transcribe-cpp";

/// Whether this build includes the Vulkan GPU backend (the `gpu-vulkan` Cargo feature).
pub const GPU_BUILD: bool = cfg!(feature = "gpu-vulkan");

/// Environment variable that overrides the compute device: `cpu` or `gpu`.
pub const DEVICE_ENV: &str = "LOCALFLOW_ASR_DEVICE";
/// Environment variable that overrides the CPU thread count: a number, or `0` / `auto` for
/// the library default.
pub const THREADS_ENV: &str = "LOCALFLOW_ASR_THREADS";

/// CPU threads used unless overridden. `None` is the library default, which is the number of
/// usable logical CPUs capped at 8 (`default_n_threads` in transcribe.cpp): 8 on a 22-thread
/// Core Ultra 7 165H.
pub const DEFAULT_THREADS: Option<u32> = None;
/// Thread counts above this are treated as a typo and clamped.
const MAX_THREADS: u32 = 256;

/// The GPU vendor preferred when several discrete GPUs exist, matched against the device
/// description without regard to case.
pub const DEFAULT_PREFERRED_GPU: &str = "NVIDIA";

/// Warm-up clip lengths, in seconds. On the CPU one short clip is enough. On a GPU, Vulkan
/// prepares some kernels only when an input first needs a larger matrix shape, so a 2 s
/// warm-up left a fresh build's first 3 s phrase stalled for 9.6 s and its first 8 s phrase
/// for 5 s (`docs/benchmarks/ASR_SPEED_BENCHMARK.md`); clips of three lengths cover the
/// shapes phrases use.
const WARM_UP_SECONDS_CPU: &[usize] = &[2];
const WARM_UP_SECONDS_GPU: &[usize] = &[2, 4, 10];

/// Whether `model_path` is a GGUF model for this engine. The whisper.cpp sidecars use the
/// older `ggml-*.bin` container, so the extension is enough to tell them apart.
pub fn is_transcribe_cpp_model(model_path: &std::path::Path) -> bool {
    model_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("gguf"))
}

#[derive(Debug, Clone)]
pub struct TranscribeCppConfig {
    pub model_path: PathBuf,
}

/// Where the speech model should run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrDevice {
    /// The CPU only.
    Cpu,
    /// A discrete GPU through Vulkan when this build has the `gpu-vulkan` feature. The CPU
    /// otherwise, or whenever no discrete GPU can be used.
    DiscreteGpu,
}

impl AsrDevice {
    /// The default for this build: the GPU when the build includes a GPU backend.
    pub const fn build_default() -> Self {
        if GPU_BUILD {
            Self::DiscreteGpu
        } else {
            Self::Cpu
        }
    }

    /// Parse `cpu` or `gpu` (also `vulkan`), ignoring case and surrounding space.
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cpu" => Some(Self::Cpu),
            "gpu" | "vulkan" | "discrete-gpu" => Some(Self::DiscreteGpu),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::DiscreteGpu => "gpu",
        }
    }
}

/// Engine tuning, each field with a default. Kept apart from [`TranscribeCppConfig`] so code
/// that builds that struct literally keeps compiling; [`TranscribeCppProvider::new`] uses
/// [`TranscribeCppOptions::default`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscribeCppOptions {
    /// CPU threads for the session. `None` is the library default (usable logical CPUs,
    /// capped at 8). On a GPU this only affects the few operations that stay on the CPU.
    pub threads: Option<u32>,
    /// Where to run the model.
    pub device: AsrDevice,
    /// Preferred discrete GPU, matched against the device description without regard to case.
    /// Any other discrete GPU is still used when none matches.
    pub preferred_gpu: Option<String>,
    /// Let `warm_up` (called when a conversation opens) transcribe a few clips of quiet
    /// noise, so the first real phrase does not pay one-time costs (GPU kernel preparation,
    /// first touch of the weights). `prepare` only ever loads the model.
    pub warm_up: bool,
}

impl Default for TranscribeCppOptions {
    fn default() -> Self {
        Self {
            threads: DEFAULT_THREADS,
            device: AsrDevice::build_default(),
            preferred_gpu: Some(DEFAULT_PREFERRED_GPU.to_string()),
            warm_up: true,
        }
    }
}

impl TranscribeCppOptions {
    /// The defaults with any [`DEVICE_ENV`] and [`THREADS_ENV`] overrides applied.
    pub fn from_env() -> Self {
        Self::default().with_overrides(
            std::env::var(DEVICE_ENV).ok().as_deref(),
            std::env::var(THREADS_ENV).ok().as_deref(),
        )
    }

    /// Apply textual overrides. Values that do not parse are ignored, so a typo keeps the
    /// default rather than disabling the engine.
    pub fn with_overrides(mut self, device: Option<&str>, threads: Option<&str>) -> Self {
        if let Some(device) = device.and_then(AsrDevice::parse) {
            self.device = device;
        }
        if let Some(threads) = threads.and_then(parse_threads) {
            self.threads = threads;
        }
        self
    }

    /// The thread count passed to the library, where `0` means its default.
    fn session_threads(&self) -> i32 {
        self.threads
            .map_or(0, |threads| threads.clamp(1, MAX_THREADS) as i32)
    }
}

/// `Some(None)` for the library default, `Some(Some(n))` for a count, `None` when unparsable.
fn parse_threads(value: &str) -> Option<Option<u32>> {
    match value.trim().to_ascii_lowercase().as_str() {
        "0" | "auto" | "default" => Some(None),
        other => other
            .parse::<u32>()
            .ok()
            .map(|threads| Some(threads.clamp(1, MAX_THREADS))),
    }
}

/// What GPU selection needs to know about one compute device, so the rules can be tested
/// without a GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceSummary {
    /// Backend kind reported by the library, for example `vulkan` or `cpu`.
    pub kind: String,
    pub device_type: DeviceType,
    /// For example `NVIDIA GeForce RTX 4070 Laptop GPU`.
    pub description: String,
    /// Device memory in bytes, or 0 when unreported.
    pub memory_total: u64,
}

impl From<&transcribe_cpp::Device> for DeviceSummary {
    fn from(device: &transcribe_cpp::Device) -> Self {
        Self {
            kind: device.kind.clone(),
            device_type: device.device_type,
            description: device.description.clone(),
            memory_total: device.memory_total,
        }
    }
}

/// Why no GPU was chosen. The text is safe for logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoGpu {
    /// No Vulkan device is registered: no driver, no Vulkan loader, or the build has no GPU
    /// backend.
    NoVulkanDevice,
    /// Vulkan devices exist but every one is an integrated GPU, which is never used.
    OnlyIntegrated,
}

impl std::fmt::Display for NoGpu {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NoVulkanDevice => "no Vulkan GPU is available",
            Self::OnlyIntegrated => "only an integrated GPU is available",
        })
    }
}

/// Index into `devices` of the GPU to run on. Only discrete Vulkan GPUs qualify: an
/// integrated GPU shares the CPU's memory and power budget and gains little. Among those, one
/// whose description contains `preferred` (without regard to case) wins, then the one with
/// the most memory, then the first listed.
pub fn choose_gpu(devices: &[DeviceSummary], preferred: Option<&str>) -> Result<usize, NoGpu> {
    let preferred = preferred
        .map(|vendor| vendor.trim().to_ascii_lowercase())
        .filter(|vendor| !vendor.is_empty());
    let vulkan = || {
        devices
            .iter()
            .enumerate()
            .filter(|(_, device)| device.kind.eq_ignore_ascii_case("vulkan"))
    };
    if vulkan().next().is_none() {
        return Err(NoGpu::NoVulkanDevice);
    }

    vulkan()
        .filter(|(_, device)| device.device_type == DeviceType::Gpu)
        .max_by_key(|(index, device)| {
            let is_preferred = preferred.as_ref().is_some_and(|vendor| {
                device
                    .description
                    .to_ascii_lowercase()
                    .contains(vendor.as_str())
            });
            // `Reverse(index)` makes the first listed device win a tie under `max_by_key`.
            (is_preferred, device.memory_total, std::cmp::Reverse(*index))
        })
        .map(|(index, _)| index)
        .ok_or(NoGpu::OnlyIntegrated)
}

struct Loaded {
    session: Session,
    /// Compute backend the library chose, for diagnostics (for example `cpu` or `vulkan`).
    backend: String,
    /// Description of the GPU in use, when on a GPU.
    gpu: Option<String>,
    supports_vocabulary: bool,
    warmed_up: bool,
}

pub struct TranscribeCppProvider {
    config: TranscribeCppConfig,
    options: TranscribeCppOptions,
    loaded: Mutex<Option<Loaded>>,
    /// Set after the GPU failed once, so every later load in this process uses the CPU.
    gpu_failed: AtomicBool,
    /// Set by a shutdown that found the model in use (a load, warm-up or transcription can
    /// hold it for seconds); whoever holds it frees it when done, so the shutdown never waits.
    unload_when_done: AtomicBool,
}

static SILENCE_NATIVE_LOGGING: Once = Once::new();
static NO_GPU_BACKEND_NOTICE: Once = Once::new();

impl TranscribeCppProvider {
    pub fn new(config: TranscribeCppConfig) -> Self {
        Self::with_options(config, TranscribeCppOptions::default())
    }

    pub fn with_options(config: TranscribeCppConfig, options: TranscribeCppOptions) -> Self {
        Self {
            config,
            options,
            loaded: Mutex::new(None),
            gpu_failed: AtomicBool::new(false),
            unload_when_done: AtomicBool::new(false),
        }
    }

    pub fn options(&self) -> &TranscribeCppOptions {
        &self.options
    }

    /// The compute backend in use, once the model is loaded.
    pub fn backend(&self) -> Option<String> {
        self.lock().as_ref().map(|loaded| loaded.backend.clone())
    }

    /// The GPU in use, once the model is loaded on one.
    pub fn gpu(&self) -> Option<String> {
        self.lock().as_ref().and_then(|loaded| loaded.gpu.clone())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Loaded>> {
        self.loaded
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// End of an operation on the model: free it if a shutdown asked for that meanwhile.
    fn finish(&self, guard: &mut Option<Loaded>) {
        if self.unload_when_done.swap(false, Ordering::SeqCst) {
            *guard = None;
        }
    }

    fn ensure_loaded<'a>(&self, guard: &'a mut Option<Loaded>) -> Result<&'a mut Loaded, AsrError> {
        if guard.is_none() {
            *guard = Some(self.load()?);
        }
        Ok(guard.as_mut().expect("model was just loaded"))
    }

    fn warm_up_locked(&self, guard: &mut Option<Loaded>) -> Result<(), AsrError> {
        let loaded = self.ensure_loaded(guard)?;
        if loaded.warmed_up {
            return Ok(());
        }
        let lengths = if is_cpu_backend(&loaded.backend) {
            WARM_UP_SECONDS_CPU
        } else {
            WARM_UP_SECONDS_GPU
        };
        let started_at = Instant::now();
        for &seconds in lengths {
            let samples = warm_up_samples(seconds);
            let request = TranscriptionRequest {
                samples: &samples,
                language: "en",
                initial_prompt: None,
            };
            // The text is discarded: it is noise, and it is never logged.
            if let Err(error) = self.run_with_cpu_retry(guard, &request) {
                if guard.is_none() {
                    // The model could not even be reloaded: speech recognition is unusable.
                    return Err(error);
                }
                // The model is loaded and usable; a slow or failed warm-up only means the
                // first phrase may pay the one-time costs. It must not stop a conversation.
                tracing::warn!(error = %error, "transcribe.cpp warm-up did not finish");
                return Ok(());
            }
        }
        if let Some(loaded) = guard.as_mut() {
            loaded.warmed_up = true;
            tracing::info!(
                backend = %loaded.backend,
                warm_up_ms = started_at.elapsed().as_millis() as u64,
                "transcribe.cpp warm-up finished"
            );
        }
        Ok(())
    }

    /// Run `request`; if the run fails on the GPU, run it once more on the CPU. A failed GPU
    /// run marks the GPU unusable and drops the model, so the reload is on the CPU. A timeout
    /// is not retried: the caller has already waited out the deadline.
    fn run_with_cpu_retry(
        &self,
        guard: &mut Option<Loaded>,
        request: &TranscriptionRequest<'_>,
    ) -> Result<Transcription, AsrError> {
        let was_on_gpu = self.ensure_loaded(guard)?.gpu.is_some();
        match self.run_loaded(guard, request) {
            Err(AsrError::Failed(_)) if was_on_gpu => self.run_loaded(guard, request),
            result => result,
        }
    }

    /// Whether this load should try the GPU.
    fn wants_gpu(&self) -> bool {
        if self.options.device != AsrDevice::DiscreteGpu {
            return false;
        }
        if !GPU_BUILD {
            NO_GPU_BACKEND_NOTICE.call_once(|| {
                tracing::info!(
                    "speech model GPU requested, but this build has no GPU backend; using the CPU"
                );
            });
            return false;
        }
        !self.gpu_failed.load(Ordering::Relaxed)
    }

    fn load(&self) -> Result<Loaded, AsrError> {
        if !self.config.model_path.is_file() {
            return Err(AsrError::Unavailable(format!(
                "Missing speech model ({})",
                super::model_label(&self.config.model_path)
            )));
        }

        // The native library prints diagnostics to stderr by default. Nothing it prints is
        // needed, and silence is the safe default for a dictation app.
        SILENCE_NATIVE_LOGGING.call_once(transcribe_cpp::disable_logging);

        let started_at = Instant::now();
        let mut gpu = None;
        let mut model = None;
        if self.wants_gpu() {
            match load_on_gpu(
                &self.config.model_path,
                self.options.preferred_gpu.as_deref(),
            ) {
                Ok((on_gpu, description)) => {
                    model = Some(on_gpu);
                    gpu = Some(description);
                }
                Err(reason) => {
                    tracing::warn!(reason = %reason, "speech model GPU unavailable; using the CPU")
                }
            }
        }
        let model = match model {
            Some(model) => model,
            None => Model::load_with(
                &self.config.model_path,
                &ModelOptions {
                    backend: Backend::Cpu,
                    ..Default::default()
                },
            )
            .map_err(|error| {
                AsrError::Unavailable(format!(
                    "Could not load speech model ({}): {:?} error",
                    super::model_label(&self.config.model_path),
                    error.kind()
                ))
            })?,
        };

        let session = model
            .session_with(&SessionOptions {
                n_threads: self.options.session_threads(),
                ..Default::default()
            })
            .map_err(|error| {
                AsrError::Unavailable(format!(
                    "Could not start a speech session: {:?} error",
                    error.kind()
                ))
            })?;
        let backend = model.backend();

        tracing::info!(
            backend = %backend,
            gpu = gpu.as_deref().unwrap_or("none"),
            threads = self.options.session_threads(),
            load_ms = started_at.elapsed().as_millis() as u64,
            "transcribe.cpp model loaded"
        );

        Ok(Loaded {
            session,
            backend,
            gpu,
            supports_vocabulary: model.supports(Feature::Vocabulary),
            warmed_up: false,
        })
    }

    /// Run `request` on the loaded model (loading it first if needed) under the deadline.
    fn run_loaded(
        &self,
        guard: &mut Option<Loaded>,
        request: &TranscriptionRequest<'_>,
    ) -> Result<Transcription, AsrError> {
        let loaded = self.ensure_loaded(guard)?;

        let options = RunOptions {
            language: Some(request.language.to_string()),
            vocabulary: if loaded.supports_vocabulary {
                vocabulary_terms(request.initial_prompt)
            } else {
                Vec::new()
            },
            ..Default::default()
        };

        let started_at = Instant::now();
        let result = run_with_deadline(
            &mut loaded.session,
            request.samples,
            &options,
            transcription_deadline(request.audio_duration()),
        );

        match result {
            Ok(transcript) => Ok(Transcription {
                text: transcript
                    .text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                provider: TRANSCRIBE_CPP_PROVIDER_ID,
                fallback_reason: None,
            }),
            Err(error) if error.kind() == ErrorKind::Aborted => Err(AsrError::Timeout {
                provider: TRANSCRIBE_CPP_PROVIDER_ID,
                waited: started_at.elapsed(),
            }),
            // Audio too short to decode is not a failure; it simply contains no words.
            Err(error) if error.kind() == ErrorKind::InputTooShort => Ok(Transcription {
                text: String::new(),
                provider: TRANSCRIBE_CPP_PROVIDER_ID,
                fallback_reason: None,
            }),
            Err(error) => {
                // A failed run can leave the session in an unknown state; reload next time.
                // Only the error kind is reported: the full message can carry partial text.
                let kind = error.kind();
                if loaded.gpu.is_some() {
                    self.gpu_failed.store(true, Ordering::Relaxed);
                    tracing::warn!(
                        error = ?kind,
                        "speech model run failed on the GPU; retrying and staying on the CPU"
                    );
                }
                *guard = None;
                Err(AsrError::Failed(format!(
                    "transcribe.cpp run failed: {kind:?} error"
                )))
            }
        }
    }
}

/// Run one transcription, cancelling it through the library's abort callback if it passes
/// `deadline`. A watchdog thread waits for the run to signal completion.
fn run_with_deadline(
    session: &mut Session,
    samples: &[f32],
    options: &RunOptions,
    deadline: Duration,
) -> transcribe_cpp::Result<Transcript> {
    let cancel = CancelToken::new();
    session.set_cancel_token(&cancel);
    let (finished_tx, finished_rx) = mpsc::channel::<()>();

    let result = thread::scope(|scope| {
        let watchdog_cancel = cancel.clone();
        scope.spawn(move || {
            if finished_rx.recv_timeout(deadline).is_err() {
                watchdog_cancel.cancel();
            }
        });
        let result = session.run(samples, options);
        let _ = finished_tx.send(());
        result
    });
    session.clear_cancel_token();
    result
}

/// Load the model on the chosen discrete GPU. The error is a log-safe reason.
#[cfg(feature = "gpu-vulkan")]
fn load_on_gpu(model_path: &Path, preferred: Option<&str>) -> Result<(Model, String), String> {
    let devices = transcribe_cpp::devices();
    let summaries: Vec<DeviceSummary> = devices.iter().map(DeviceSummary::from).collect();
    let index = choose_gpu(&summaries, preferred).map_err(|reason| reason.to_string())?;
    let device = devices[index].clone();
    let description = device.description.clone();
    let model = Model::load_with(
        model_path,
        &ModelOptions {
            backend: Backend::Vulkan,
            device: Some(device),
            ..Default::default()
        },
    )
    .map_err(|error| format!("loading on {description} failed: {:?} error", error.kind()))?;
    Ok((model, description))
}

#[cfg(not(feature = "gpu-vulkan"))]
fn load_on_gpu(_model_path: &Path, _preferred: Option<&str>) -> Result<(Model, String), String> {
    Err(NoGpu::NoVulkanDevice.to_string())
}

/// Whether a backend name (as transcribe.cpp reports it) is the CPU.
fn is_cpu_backend(backend: &str) -> bool {
    backend.to_ascii_lowercase().contains("cpu")
}

/// `seconds` of quiet noise at 16 kHz from a fixed-seed generator (about -46 dBFS RMS).
fn warm_up_samples(seconds: usize) -> Vec<f32> {
    let mut state: u32 = 0x2545_f491;
    (0..seconds * 16_000)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.017
        })
        .collect()
}

/// Dictionary terms recovered from the vocabulary prompt (`Glossary: a, b, c.`), so engines
/// with a native vocabulary feature receive a term list instead of a sentence.
fn vocabulary_terms(initial_prompt: Option<&str>) -> Vec<String> {
    let Some(prompt) = initial_prompt else {
        return Vec::new();
    };
    prompt
        .trim()
        .trim_start_matches("Glossary:")
        .trim_end_matches('.')
        .split(',')
        .map(|term| term.trim().to_string())
        .filter(|term| !term.is_empty())
        .collect()
}

impl SpeechToTextProvider for TranscribeCppProvider {
    fn id(&self) -> &'static str {
        TRANSCRIBE_CPP_PROVIDER_ID
    }

    fn describe(&self) -> String {
        let threads = self.options.threads.map_or_else(
            || "library default".to_string(),
            |threads| threads.to_string(),
        );
        let mut description = format!(
            "transcribe.cpp in-process, model kept loaded ({}), threads {threads}, device {}",
            super::model_label(&self.config.model_path),
            self.options.device.label()
        );
        // Not worth waiting for a transcription in progress to say where it runs.
        if let Ok(loaded) = self.loaded.try_lock() {
            if let Some(loaded) = loaded.as_ref() {
                description.push_str(&format!(", running on {}", loaded.backend));
                if let Some(gpu) = &loaded.gpu {
                    description.push_str(&format!(" ({gpu})"));
                }
            }
        }
        description
    }

    /// Load the model only. Dictation calls this as recording starts, and a warm-up here
    /// would hold the model lock while the user's own first phrase waits.
    fn prepare(&self) -> Result<(), AsrError> {
        let mut guard = self.lock();
        let result = self.ensure_loaded(&mut guard).map(|_| ());
        self.finish(&mut guard);
        result
    }

    /// Load the model if needed and, when [`TranscribeCppOptions::warm_up`] is set (the
    /// default), transcribe a few clips of quiet noise (2 s on the CPU; 2, 4 and 10 s on a
    /// GPU), so the next real phrase does not pay one-time costs: GPU kernel preparation
    /// (seconds on first use) and the first touch of the memory-mapped weights. Only the
    /// first call after a load does any work. Called when a conversation opens. A warm-up
    /// that fails or times out is logged and ignored, as long as the model is loaded.
    fn warm_up(&self) -> Result<(), AsrError> {
        let mut guard = self.lock();
        let result = if self.options.warm_up {
            self.warm_up_locked(&mut guard)
        } else {
            self.ensure_loaded(&mut guard).map(|_| ())
        };
        self.finish(&mut guard);
        result
    }

    fn transcribe(&self, request: &TranscriptionRequest<'_>) -> Result<Transcription, AsrError> {
        let mut guard = self.lock();
        let result = self.run_with_cpu_retry(&mut guard, request);
        self.finish(&mut guard);
        result
    }

    /// Free the model. Never waits: if a load, warm-up or transcription holds it, that
    /// operation frees it when it ends. A later request loads it again.
    fn shutdown(&self) {
        match self.loaded.try_lock() {
            // Dropping the session and model frees the weights.
            Ok(mut guard) => *guard = None,
            Err(TryLockError::Poisoned(poisoned)) => *poisoned.into_inner() = None,
            Err(TryLockError::WouldBlock) => {
                self.unload_when_done.store(true, Ordering::SeqCst);
                // The holder may have finished between the attempt and the flag.
                if let Ok(mut guard) = self.loaded.try_lock() {
                    self.finish(&mut guard);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gguf_files_select_this_engine_and_ggml_bins_do_not() {
        assert!(is_transcribe_cpp_model(Path::new(
            "models/parakeet-tdt-0.6b-v3-Q5_K_M.gguf"
        )));
        assert!(is_transcribe_cpp_model(Path::new("MODEL.GGUF")));
        assert!(!is_transcribe_cpp_model(Path::new(
            "models/ggml-tiny.en-q5_1.bin"
        )));
        assert!(!is_transcribe_cpp_model(Path::new("models/gguf")));
    }

    #[test]
    fn shutting_down_while_the_model_is_in_use_does_not_wait_for_it() {
        let provider = std::sync::Arc::new(TranscribeCppProvider::new(TranscribeCppConfig {
            model_path: PathBuf::from("Z:/someone/models/parakeet.gguf"),
        }));
        // A transcription in progress holds the model.
        let (holding_tx, holding_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = {
            let provider = provider.clone();
            thread::spawn(move || {
                let mut guard = provider.lock();
                holding_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                provider.finish(&mut guard);
            })
        };
        holding_rx.recv().unwrap();

        let started = Instant::now();
        provider.shutdown();
        assert!(started.elapsed() < Duration::from_millis(200));
        assert!(provider.unload_when_done.load(Ordering::SeqCst));

        // The holder frees it when it is done, and the request is cleared.
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert!(!provider.unload_when_done.load(Ordering::SeqCst));
        assert!(provider.lock().is_none());
        // An idle model is freed at once.
        provider.shutdown();
        assert!(!provider.unload_when_done.load(Ordering::SeqCst));
    }

    #[test]
    fn vocabulary_terms_are_recovered_from_the_prompt() {
        assert_eq!(
            vocabulary_terms(Some("Glossary: Tauri, cpal, LocalFlow.")),
            vec!["Tauri", "cpal", "LocalFlow"]
        );
        assert!(vocabulary_terms(None).is_empty());
        assert!(vocabulary_terms(Some("Glossary: .")).is_empty());
    }

    #[test]
    fn a_missing_model_is_unavailable_and_names_no_path() {
        let provider = TranscribeCppProvider::new(TranscribeCppConfig {
            model_path: PathBuf::from("Z:/someone/models/parakeet.gguf"),
        });

        let error = provider.prepare().unwrap_err();

        assert_eq!(error.to_string(), "Missing speech model (parakeet.gguf)");
        assert_eq!(provider.backend(), None);
        assert_eq!(provider.gpu(), None);
        assert_eq!(
            provider.warm_up().unwrap_err().to_string(),
            "Missing speech model (parakeet.gguf)"
        );
    }

    fn device(kind: &str, device_type: DeviceType, description: &str, gib: u64) -> DeviceSummary {
        DeviceSummary {
            kind: kind.to_string(),
            device_type,
            description: description.to_string(),
            memory_total: gib << 30,
        }
    }

    /// The devices this laptop registers in a Vulkan build, in a plausible registry order.
    fn laptop_devices() -> Vec<DeviceSummary> {
        vec![
            device("vulkan", DeviceType::Igpu, "Intel(R) Arc(TM) Graphics", 16),
            device(
                "vulkan",
                DeviceType::Gpu,
                "NVIDIA GeForce RTX 4070 Laptop GPU",
                8,
            ),
            device("cpu", DeviceType::Cpu, "Intel(R) Core(TM) Ultra 7 165H", 32),
        ]
    }

    #[test]
    fn the_discrete_nvidia_gpu_is_chosen_and_the_intel_igpu_never_is() {
        let devices = laptop_devices();

        assert_eq!(choose_gpu(&devices, Some("NVIDIA")), Ok(1));
        // Even with Intel preferred, or no preference, an integrated GPU is not a candidate,
        // although it reports more (shared) memory.
        assert_eq!(choose_gpu(&devices, Some("intel")), Ok(1));
        assert_eq!(choose_gpu(&devices, None), Ok(1));
    }

    #[test]
    fn no_gpu_reasons_distinguish_missing_vulkan_from_an_igpu_only_machine() {
        let cpu_only = vec![device("cpu", DeviceType::Cpu, "CPU", 32)];
        assert_eq!(
            choose_gpu(&cpu_only, Some("NVIDIA")),
            Err(NoGpu::NoVulkanDevice)
        );
        assert_eq!(choose_gpu(&[], None), Err(NoGpu::NoVulkanDevice));

        let igpu_only = vec![
            device("vulkan", DeviceType::Igpu, "Intel(R) Arc(TM) Graphics", 16),
            device("cpu", DeviceType::Cpu, "CPU", 32),
        ];
        assert_eq!(
            choose_gpu(&igpu_only, Some("NVIDIA")),
            Err(NoGpu::OnlyIntegrated)
        );

        // A non-Vulkan GPU (another backend) is not usable by this feature either.
        let cuda_only = vec![device("cuda", DeviceType::Gpu, "NVIDIA GeForce", 8)];
        assert_eq!(choose_gpu(&cuda_only, None), Err(NoGpu::NoVulkanDevice));

        // Reasons are short, log-safe sentences.
        assert_eq!(
            NoGpu::NoVulkanDevice.to_string(),
            "no Vulkan GPU is available"
        );
        assert_eq!(
            NoGpu::OnlyIntegrated.to_string(),
            "only an integrated GPU is available"
        );
    }

    #[test]
    fn among_discrete_gpus_the_preferred_vendor_wins_then_memory_then_order() {
        let devices = vec![
            device("vulkan", DeviceType::Gpu, "AMD Radeon RX 7900", 24),
            device("Vulkan", DeviceType::Gpu, "NVIDIA GeForce RTX 4070", 8),
            device("vulkan", DeviceType::Gpu, "NVIDIA GeForce RTX 4090", 24),
        ];
        // Preferred vendor first, then the larger of the two.
        assert_eq!(choose_gpu(&devices, Some(" nvidia ")), Ok(2));
        // Without a usable preference the most memory wins; the first listed breaks the tie.
        assert_eq!(choose_gpu(&devices, None), Ok(0));
        assert_eq!(choose_gpu(&devices, Some("")), Ok(0));
        // A preference nothing matches still uses a discrete GPU.
        assert_eq!(choose_gpu(&devices, Some("Moore Threads")), Ok(0));
    }

    #[test]
    fn options_default_to_the_build_device_and_the_library_thread_default() {
        let options = TranscribeCppOptions::default();

        assert_eq!(options.threads, DEFAULT_THREADS);
        assert_eq!(options.device, AsrDevice::build_default());
        assert_eq!(
            options.device == AsrDevice::DiscreteGpu,
            cfg!(feature = "gpu-vulkan")
        );
        assert_eq!(options.preferred_gpu.as_deref(), Some("NVIDIA"));
        assert!(options.warm_up);
    }

    #[test]
    fn thread_option_maps_to_the_library_value() {
        let with = |threads| TranscribeCppOptions {
            threads,
            ..Default::default()
        };

        assert_eq!(with(None).session_threads(), 0, "0 is the library default");
        assert_eq!(with(Some(8)).session_threads(), 8);
        assert_eq!(
            with(Some(0)).session_threads(),
            1,
            "never zero when explicit"
        );
        assert_eq!(with(Some(100_000)).session_threads(), MAX_THREADS as i32);
    }

    #[test]
    fn environment_overrides_parse_or_are_ignored() {
        let base = TranscribeCppOptions {
            threads: Some(6),
            device: AsrDevice::DiscreteGpu,
            ..Default::default()
        };

        let cpu8 = base.clone().with_overrides(Some(" CPU "), Some("8"));
        assert_eq!(cpu8.device, AsrDevice::Cpu);
        assert_eq!(cpu8.threads, Some(8));

        let auto = base.clone().with_overrides(Some("vulkan"), Some("auto"));
        assert_eq!(auto.device, AsrDevice::DiscreteGpu);
        assert_eq!(auto.threads, None);
        assert_eq!(base.clone().with_overrides(None, Some("0")).threads, None);

        // Typos keep the defaults rather than disabling anything.
        let typo = base.clone().with_overrides(Some("gpuu"), Some("eight"));
        assert_eq!(typo, base);
        assert_eq!(base.clone().with_overrides(None, None), base);
    }

    #[test]
    fn device_labels_round_trip() {
        for device in [AsrDevice::Cpu, AsrDevice::DiscreteGpu] {
            assert_eq!(AsrDevice::parse(device.label()), Some(device));
        }
        assert_eq!(AsrDevice::parse("tpu"), None);
    }

    #[test]
    fn describe_names_the_model_threads_and_device_but_no_path() {
        let provider = TranscribeCppProvider::with_options(
            TranscribeCppConfig {
                model_path: PathBuf::from("Z:/someone/models/parakeet.gguf"),
            },
            TranscribeCppOptions {
                threads: Some(8),
                device: AsrDevice::Cpu,
                ..Default::default()
            },
        );

        let description = provider.describe();

        assert!(description.contains("(parakeet.gguf), threads 8, device cpu"));
        assert!(!description.contains("someone"));
    }

    #[test]
    fn the_warm_up_clips_are_quiet_noise_of_the_given_length() {
        let samples = warm_up_samples(2);

        assert_eq!(samples.len(), 32_000);
        let peak = samples
            .iter()
            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
        assert!(peak > 0.0 && peak < 0.01, "peak {peak}");
        assert_eq!(samples, warm_up_samples(2), "deterministic");
        assert_eq!(warm_up_samples(10).len(), 160_000);
    }

    #[test]
    fn a_gpu_is_warmed_up_with_longer_clips_than_the_cpu() {
        assert!(is_cpu_backend("CPU"));
        assert!(is_cpu_backend("cpu"));
        assert!(!is_cpu_backend("Vulkan0"));
        assert_eq!(WARM_UP_SECONDS_CPU, &[2]);
        assert!(WARM_UP_SECONDS_GPU.iter().any(|&seconds| seconds >= 8));
    }

    fn runtime() -> PathBuf {
        std::path::absolute(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join(".localflow-runtime"),
        )
        .unwrap()
    }

    fn jfk_samples() -> Vec<f32> {
        let audio = crate::audio::wav::read_wav_mono(
            &runtime().join("bench-corpus").join("wav").join("jfk.wav"),
        )
        .expect("bench corpus jfk.wav");
        crate::audio::resample::resample_to_16khz(&audio.samples, audio.sample_rate)
    }

    fn parakeet() -> TranscribeCppConfig {
        TranscribeCppConfig {
            model_path: runtime()
                .join("models")
                .join("parakeet-tdt-0.6b-v3-Q5_K_M.gguf"),
        }
    }

    /// End-to-end check with the real model. Ignored by default; run with
    /// `cargo test --release --lib real_parakeet -- --ignored`.
    #[test]
    #[ignore = "needs the Parakeet GGUF model and the bench corpus in .localflow-runtime"]
    fn real_parakeet_transcribes_and_reloads_after_shutdown() {
        let samples = jfk_samples();
        let provider = TranscribeCppProvider::new(parakeet());
        let request = TranscriptionRequest {
            samples: &samples,
            language: "en",
            initial_prompt: None,
        };

        let first = provider.transcribe(&request).unwrap();
        assert!(first.text.to_lowercase().contains("fellow americans"));
        assert!(provider.backend().is_some());

        provider.shutdown();
        assert_eq!(provider.backend(), None);
        assert_eq!(provider.transcribe(&request).unwrap().text, first.text);
    }

    /// The CPU and the build's default device (the GPU in a `gpu-vulkan` build) agree, an
    /// explicit thread count works, and the warm-up leaves the model loaded. Ignored by
    /// default; run with `cargo test --release --lib real_parakeet -- --ignored`.
    #[test]
    #[ignore = "needs the Parakeet GGUF model and the bench corpus in .localflow-runtime"]
    fn real_parakeet_default_device_matches_the_cpu() {
        let samples = jfk_samples();
        let request = TranscriptionRequest {
            samples: &samples,
            language: "en",
            initial_prompt: None,
        };
        let cpu = TranscribeCppProvider::with_options(
            parakeet(),
            TranscribeCppOptions {
                threads: Some(8),
                device: AsrDevice::Cpu,
                ..Default::default()
            },
        );
        let default = TranscribeCppProvider::new(parakeet());

        default.prepare().unwrap();
        assert!(default.backend().is_some());
        if GPU_BUILD {
            // On this laptop the discrete GPU must be chosen. A machine without one falls
            // back to the CPU, which this assertion would flag.
            let gpu = default.gpu().expect("the discrete GPU should be in use");
            assert!(gpu.contains("NVIDIA"), "unexpected GPU {gpu}");
        } else {
            assert_eq!(default.gpu(), None);
        }

        let cpu_text = cpu.transcribe(&request).unwrap().text;
        assert_eq!(cpu.gpu(), None);
        assert_eq!(default.transcribe(&request).unwrap().text, cpu_text);
    }

    /// GPU against CPU on the whole bench corpus. Transcripts must agree closely; rough
    /// timings and GPU memory are printed as numbers only, never text. Run alone so the two
    /// engines do not compete:
    /// `cargo test --release --lib --features gpu-vulkan real_parakeet_gpu -- --ignored
    /// --nocapture --test-threads=1`.
    #[cfg(feature = "gpu-vulkan")]
    #[test]
    #[ignore = "needs a discrete GPU, the Parakeet GGUF model and the bench corpus"]
    fn real_parakeet_gpu_agrees_with_the_cpu_on_the_bench_corpus() {
        use crate::asr::metrics::word_errors;

        let corpus_dir = runtime().join("bench-corpus");
        let manifest = std::fs::read_to_string(corpus_dir.join("manifest.tsv")).unwrap();
        let clips: Vec<(String, Vec<f32>)> = manifest
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(id, reference)| {
                let audio = crate::audio::wav::read_wav_mono(
                    &corpus_dir.join("wav").join(format!("{id}.wav")),
                )
                .unwrap();
                let samples =
                    crate::audio::resample::resample_to_16khz(&audio.samples, audio.sample_rate);
                (reference.to_string(), samples)
            })
            .collect();
        let seconds = |samples: &[f32]| samples.len() as f64 / 16_000.0;
        let short: Vec<usize> = (0..clips.len())
            .filter(|&index| (1.0..=3.0).contains(&seconds(&clips[index].1)))
            .collect();
        assert!(!short.is_empty(), "the corpus should have 1-3 s clips");

        let gpu_free_mib = || {
            transcribe_cpp::devices()
                .iter()
                .find(|device| device.kind == "vulkan" && device.device_type == DeviceType::Gpu)
                .map(|device| device.memory_free >> 20)
        };
        let engine = |device| {
            TranscribeCppProvider::with_options(
                parakeet(),
                TranscribeCppOptions {
                    device,
                    warm_up: false,
                    ..Default::default()
                },
            )
        };
        let run = |provider: &TranscribeCppProvider, samples: &[f32]| {
            let started = Instant::now();
            let text = provider
                .transcribe(&TranscriptionRequest {
                    samples,
                    language: "en",
                    initial_prompt: None,
                })
                .unwrap()
                .text;
            (started.elapsed().as_secs_f64() * 1000.0, text)
        };
        let median = |mut values: Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values[values.len() / 2]
        };

        // GPU first, so its first request is cold: nothing in this process has compiled
        // shaders yet.
        let free_before = gpu_free_mib();
        let gpu = engine(AsrDevice::DiscreteGpu);
        let started = Instant::now();
        gpu.prepare().unwrap();
        let gpu_load_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert!(gpu.gpu().is_some(), "the discrete GPU should be in use");
        let (gpu_cold_ms, _) = run(&gpu, &clips[short[0]].1);
        let free_loaded = gpu_free_mib();

        let cpu = engine(AsrDevice::Cpu);
        let started = Instant::now();
        cpu.prepare().unwrap();
        let cpu_load_ms = started.elapsed().as_secs_f64() * 1000.0;
        let (cpu_cold_ms, _) = run(&cpu, &clips[short[0]].1);

        let mut gpu_ms = Vec::new();
        let mut cpu_ms = Vec::new();
        let (mut gpu_edits, mut cpu_edits, mut words) = (0, 0, 0);
        let (mut identical, mut cross_edits) = (0, 0);
        for (reference, samples) in &clips {
            let (gpu_time, gpu_text) = run(&gpu, samples);
            let (cpu_time, cpu_text) = run(&cpu, samples);
            gpu_ms.push(gpu_time);
            cpu_ms.push(cpu_time);
            let (edits, reference_words) = word_errors(reference, &gpu_text);
            gpu_edits += edits;
            words += reference_words;
            cpu_edits += word_errors(reference, &cpu_text).0;
            cross_edits += word_errors(&cpu_text, &gpu_text).0;
            identical += usize::from(gpu_text == cpu_text);
        }
        let short_ms = |all: &[f64]| median(short.iter().map(|&index| all[index]).collect());
        let free_after = gpu_free_mib();

        let percent = |edits: usize| 100.0 * edits as f64 / words.max(1) as f64;
        eprintln!(
            "corpus {} clips ({} of 1-3 s); WER cpu {:.2} %, gpu {:.2} %; identical {}/{}; \
             cpu-to-gpu word edits {}",
            clips.len(),
            short.len(),
            percent(cpu_edits),
            percent(gpu_edits),
            identical,
            clips.len(),
            cross_edits
        );
        eprintln!(
            "load ms cpu {cpu_load_ms:.0}, gpu {gpu_load_ms:.0}; cold first 1-3 s clip ms \
             cpu {cpu_cold_ms:.0}, gpu {gpu_cold_ms:.0}"
        );
        eprintln!(
            "warm median ms, 1-3 s clips: cpu {:.0}, gpu {:.0}; all clips: cpu {:.0}, gpu {:.0}",
            short_ms(&cpu_ms),
            short_ms(&gpu_ms),
            median(cpu_ms.clone()),
            median(gpu_ms.clone())
        );
        eprintln!(
            "GPU free MiB: before load {free_before:?}, after load {free_loaded:?}, after \
             corpus {free_after:?}"
        );

        assert!(
            (percent(gpu_edits) - percent(cpu_edits)).abs() <= 0.5,
            "GPU and CPU word error rates should be near-identical"
        );
        assert!(
            cross_edits * 100 <= words,
            "GPU transcripts should differ from the CPU's in at most 1 % of words"
        );
    }
}
