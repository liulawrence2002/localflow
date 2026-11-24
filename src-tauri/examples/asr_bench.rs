//! Reproducible local ASR benchmark for LocalFlow.
//!
//! Runs the same audio through the same provider code the app uses and reports latency,
//! real-time factor, word error rate, memory, and failure counts. Nothing leaves the machine.
//!
//! ```powershell
//! cd src-tauri
//! cargo run --release --example asr_bench -- `
//!   --runtime ..\.localflow-runtime `
//!   --corpus ..\.localflow-runtime\bench-corpus `
//!   --config cli:ggml-tiny.en-q5_1.bin:5:8 `
//!   --config server:ggml-tiny.en-q5_1.bin:1:6 `
//!   --json ..\docs\benchmarks\results.json
//! ```
//!
//! A config is `engine:model-file:beam:threads[:context]`. Engines: `inprocess` (a GGUF
//! model such as Parakeet kept loaded through transcribe.cpp), `server` (persistent
//! whisper-server), and `cli` (one whisper process per utterance). For Whisper the last field
//! is the context: `full` (default), `dyn`, `vad`, `dyn+vad`. For `inprocess` beam is
//! ignored, threads `0` is the library default (logical CPUs capped at 8), and the last field is the
//! device: `cpu` (default) or `gpu` (a discrete GPU through Vulkan; needs a build with
//! `--features gpu-vulkan`, otherwise it runs on the CPU). The corpus directory holds
//! `manifest.tsv` (`id<TAB>reference transcript`) and `wav/<id>.wav`.
//!
//! Other flags:
//!
//! - `--sweep-threads 4,6,8,12,22` replaces each `inprocess` config with one per thread count.
//! - `--no-warm-up` skips the warm-up transcription after load, so the first request is cold.
//! - `--min-seconds` / `--max-seconds` keep only clips in that length range.
//! - `--list-devices` prints the compute devices the speech library registered.
//!
//! When several configs run, each is compared with the first one clip by clip (identical
//! transcripts and word edits between the two), without printing any text.
//!
//! Hypothesis text is never printed unless `--show-text` is passed.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use localflow_lib::{
    asr::{
        metrics::word_errors,
        provider::{SpeechToTextProvider, TranscriptionRequest},
        transcribe_cpp::{
            AsrDevice, TranscribeCppConfig, TranscribeCppOptions, TranscribeCppProvider,
        },
        whisper_cli::{WhisperCliConfig, WhisperCliProvider},
        whisper_server::{WhisperServerConfig, WhisperServerProvider},
        whisper_text::is_blank_transcript,
        AudioContext, WhisperDecodeOptions,
    },
    audio::{resample::resample_to_16khz, wav::read_wav_mono},
};

struct Args {
    runtime: PathBuf,
    corpus: PathBuf,
    configs: Vec<String>,
    limit: Option<usize>,
    json: Option<PathBuf>,
    show_text: bool,
    concurrency: usize,
    passes: usize,
    idle_secs: u64,
    sweep_threads: Vec<u32>,
    warm_up: bool,
    min_seconds: f64,
    max_seconds: f64,
    list_devices: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        runtime: PathBuf::from("../.localflow-runtime"),
        corpus: PathBuf::from("../.localflow-runtime/bench-corpus"),
        configs: Vec::new(),
        limit: None,
        json: None,
        show_text: false,
        concurrency: 4,
        passes: 1,
        idle_secs: 0,
        sweep_threads: Vec::new(),
        warm_up: true,
        min_seconds: 0.0,
        max_seconds: f64::INFINITY,
        list_devices: false,
    };

    let mut iter = std::env::args().skip(1);
    while let Some(flag) = iter.next() {
        let mut value = |name: &str| {
            iter.next()
                .ok_or_else(|| format!("{name} requires a value"))
        };
        match flag.as_str() {
            "--runtime" => args.runtime = PathBuf::from(value("--runtime")?),
            "--corpus" => args.corpus = PathBuf::from(value("--corpus")?),
            "--config" => args.configs.push(value("--config")?),
            "--limit" => {
                args.limit = Some(
                    value("--limit")?
                        .parse()
                        .map_err(|_| "--limit must be a number".to_string())?,
                )
            }
            "--passes" => {
                args.passes = value("--passes")?
                    .parse()
                    .map_err(|_| "--passes must be a number".to_string())?
            }
            "--idle-secs" => {
                args.idle_secs = value("--idle-secs")?
                    .parse()
                    .map_err(|_| "--idle-secs must be a number".to_string())?
            }
            "--concurrency" => {
                args.concurrency = value("--concurrency")?
                    .parse()
                    .map_err(|_| "--concurrency must be a number".to_string())?
            }
            "--json" => args.json = Some(PathBuf::from(value("--json")?)),
            "--show-text" => args.show_text = true,
            "--sweep-threads" => {
                args.sweep_threads = value("--sweep-threads")?
                    .split(',')
                    .map(|threads| threads.trim().parse::<u32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| "--sweep-threads must be numbers like 4,6,8".to_string())?
            }
            "--no-warm-up" => args.warm_up = false,
            "--min-seconds" => {
                args.min_seconds = value("--min-seconds")?
                    .parse()
                    .map_err(|_| "--min-seconds must be a number".to_string())?
            }
            "--max-seconds" => {
                args.max_seconds = value("--max-seconds")?
                    .parse()
                    .map_err(|_| "--max-seconds must be a number".to_string())?
            }
            "--list-devices" => args.list_devices = true,
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    if !args.sweep_threads.is_empty() {
        args.configs = expand_thread_sweep(&args.configs, &args.sweep_threads);
    }

    // Sidecars run with their own working directory, so every path must be absolute.
    args.runtime = std::path::absolute(&args.runtime)
        .map_err(|error| format!("bad --runtime path: {error}"))?;
    args.corpus =
        std::path::absolute(&args.corpus).map_err(|error| format!("bad --corpus path: {error}"))?;

    if args.configs.is_empty() && !args.list_devices {
        return Err("at least one --config engine:model:beam:threads is required".to_string());
    }
    Ok(args)
}

/// Replace each `inprocess` config with one copy per thread count; other engines are kept.
fn expand_thread_sweep(configs: &[String], thread_counts: &[u32]) -> Vec<String> {
    configs
        .iter()
        .flat_map(|spec| {
            let parts: Vec<&str> = spec.split(':').collect();
            if parts.first() != Some(&"inprocess") || parts.len() < 4 {
                return vec![spec.clone()];
            }
            thread_counts
                .iter()
                .map(|threads| {
                    let mut parts: Vec<String> =
                        parts.iter().map(|part| part.to_string()).collect();
                    parts[3] = threads.to_string();
                    parts.join(":")
                })
                .collect()
        })
        .collect()
}

fn print_devices() {
    println!(
        "speech library compute devices (this build: {}):",
        if localflow_lib::asr::transcribe_cpp::GPU_BUILD {
            "gpu-vulkan"
        } else {
            "cpu only"
        }
    );
    for device in transcribe_cpp::devices() {
        println!(
            "  {} [{} {:?}] {} — {} MiB total, {} MiB free",
            device.name,
            device.kind,
            device.device_type,
            device.description,
            device.memory_total >> 20,
            device.memory_free >> 20
        );
    }
}

struct Utterance {
    id: String,
    reference: String,
    samples: Vec<f32>,
    seconds: f64,
}

fn load_corpus(args: &Args) -> Result<Vec<Utterance>, String> {
    let corpus = args.corpus.as_path();
    let limit = args.limit;
    let manifest = std::fs::read_to_string(corpus.join("manifest.tsv"))
        .map_err(|error| format!("Could not read manifest.tsv: {error}"))?;

    let mut utterances = Vec::new();
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        if limit.is_some_and(|limit| utterances.len() >= limit) {
            break;
        }
        let (id, reference) = line
            .split_once('\t')
            .ok_or_else(|| format!("manifest line is not `id<TAB>text`: {line}"))?;
        let decoded = read_wav_mono(&corpus.join("wav").join(format!("{id}.wav")))?;
        // Same resampler as the live path, so non-16 kHz corpora exercise it too.
        let samples = resample_to_16khz(&decoded.samples, decoded.sample_rate);
        let seconds = samples.len() as f64 / 16_000.0;
        if seconds < args.min_seconds || seconds > args.max_seconds {
            continue;
        }
        utterances.push(Utterance {
            id: id.to_string(),
            reference: reference.to_string(),
            samples,
            seconds,
        });
    }
    Ok(utterances)
}

enum Engine {
    InProcess(TranscribeCppProvider),
    Cli(WhisperCliProvider),
    Server(WhisperServerProvider),
}

impl Engine {
    fn provider(&self) -> &dyn SpeechToTextProvider {
        match self {
            Self::InProcess(provider) => provider,
            Self::Cli(provider) => provider,
            Self::Server(provider) => provider,
        }
    }

    fn peak_memory_bytes(&self) -> Option<u64> {
        match self {
            // The model lives in this process, so its memory is the harness's own.
            Self::InProcess(_) => own_peak_memory_bytes(),
            Self::Cli(provider) => provider.last_peak_memory_bytes(),
            Self::Server(provider) => provider.peak_memory_bytes(),
        }
    }
}

fn build_engine(spec: &str, args: &Args) -> Result<Engine, String> {
    let runtime = args.runtime.as_path();
    let parts: Vec<&str> = spec.split(':').collect();
    let (engine, model, beam, threads, context) = match parts.as_slice() {
        [engine, model, beam, threads] => (engine, model, beam, threads, None),
        [engine, model, beam, threads, context] => (engine, model, beam, threads, Some(*context)),
        _ => {
            return Err(format!(
                "config must be engine:model:beam:threads[:context|device], got {spec}"
            ))
        }
    };
    let model_path = runtime.join("models").join(model);

    if *engine == "inprocess" {
        let threads: u32 = threads
            .parse()
            .map_err(|_| format!("bad threads in {spec}"))?;
        let device = match context {
            None => AsrDevice::Cpu,
            Some(device) => AsrDevice::parse(device)
                .ok_or_else(|| format!("bad device {device} in {spec} (cpu or gpu)"))?,
        };
        return Ok(Engine::InProcess(TranscribeCppProvider::with_options(
            TranscribeCppConfig { model_path },
            TranscribeCppOptions {
                threads: (threads > 0).then_some(threads),
                device,
                warm_up: args.warm_up,
                ..Default::default()
            },
        )));
    }

    let context = context.unwrap_or("full");
    let decode = WhisperDecodeOptions {
        beam_size: beam.parse().map_err(|_| format!("bad beam in {spec}"))?,
        threads: threads
            .parse()
            .map_err(|_| format!("bad threads in {spec}"))?,
        audio_context: match context {
            "full" | "vad" => AudioContext::Full,
            "dyn" | "dyn+vad" => AudioContext::Dynamic,
            other => return Err(format!("bad audio context {other} in {spec}")),
        },
    };
    let vad_model = context
        .ends_with("vad")
        .then(|| runtime.join("models").join("ggml-silero-v6.2.0.bin"));
    let release = runtime.join("whisper").join("Release");

    match *engine {
        "cli" => Ok(Engine::Cli(WhisperCliProvider::new(WhisperCliConfig {
            cli_path: release.join("whisper-cli.exe"),
            model_path,
            decode,
            temp_dir: std::env::temp_dir().join("localflow-bench"),
        }))),
        "server" => WhisperServerProvider::new(
            WhisperServerConfig::new(release.join("whisper-server.exe"), model_path, decode)
                .with_vad_model(vad_model),
        )
        .map(Engine::Server)
        .map_err(|error| error.to_string()),
        other => Err(format!("unknown engine {other} in {spec}")),
    }
}

/// Peak resident memory of this process, for engines that run inside it.
fn own_peak_memory_bytes() -> Option<u64> {
    localflow_lib::asr::sidecar::current_process_peak_memory_bytes()
}

fn percentile(sorted_ms: &[f64], fraction: f64) -> f64 {
    if sorted_ms.is_empty() {
        return f64::NAN;
    }
    let rank = (fraction * (sorted_ms.len() - 1) as f64).round() as usize;
    sorted_ms[rank.min(sorted_ms.len() - 1)]
}

#[derive(Debug)]
struct ConfigResult {
    spec: String,
    description: String,
    utterances: usize,
    audio_seconds: f64,
    prepare_ms: f64,
    first_request_ms: f64,
    p50_ms: f64,
    p95_ms: f64,
    mean_ms: f64,
    max_ms: f64,
    short_p50_ms: f64,
    real_time_factor: f64,
    wer_percent: f64,
    errors: usize,
    empty_outputs: usize,
    peak_memory_mb: Option<f64>,
    recovery_ms: Option<f64>,
    concurrent_wall_ms: Option<f64>,
    concurrent_failures: Option<usize>,
    /// Non-speech probes (silence, low noise) that produced text other than a blank marker.
    non_speech_hallucinations: usize,
    non_speech_probes: usize,
    /// Latency of one request sent after the engine sat idle for `--idle-secs`.
    after_idle_ms: Option<f64>,
    per_utterance: Vec<UtteranceResult>,
}

/// Per-clip measurements, kept so runs can be compared with paired statistics.
#[derive(Debug)]
struct UtteranceResult {
    id: String,
    seconds: f64,
    reference_words: usize,
    edits: usize,
    latencies_ms: Vec<f64>,
    /// First-pass transcript, kept in memory to compare configs. Never written out.
    hypothesis: Option<String>,
}

/// How closely one config's transcripts match another's, clip by clip.
#[derive(Debug, Clone, Copy)]
struct Agreement {
    identical_clips: usize,
    compared_clips: usize,
    /// Word edits turning the base config's transcripts into this one's.
    word_edits: usize,
    base_words: usize,
}

fn agreement(base: &ConfigResult, other: &ConfigResult) -> Agreement {
    let mut result = Agreement {
        identical_clips: 0,
        compared_clips: 0,
        word_edits: 0,
        base_words: 0,
    };
    for (base, other) in base.per_utterance.iter().zip(&other.per_utterance) {
        if let (Some(base), Some(other)) = (&base.hypothesis, &other.hypothesis) {
            let (edits, words) = word_errors(base, other);
            result.compared_clips += 1;
            result.identical_clips += usize::from(base == other);
            result.word_edits += edits;
            result.base_words += words;
        }
    }
    result
}

/// Three seconds of digital silence and three seconds of low-level white noise (about
/// -46 dBFS RMS, from a fixed-seed generator so every run sees the same samples).
fn non_speech_probes() -> Vec<Vec<f32>> {
    let silence = vec![0.0_f32; 48_000];
    let mut state: u32 = 0x1234_5678;
    let noise = (0..48_000)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((state >> 8) as f32 / (1u32 << 24) as f32 - 0.5) * 0.017
        })
        .collect();
    vec![silence, noise]
}

fn run_config(spec: &str, args: &Args, corpus: &[Utterance]) -> Result<ConfigResult, String> {
    let engine = Arc::new(build_engine(spec, args)?);
    let provider = engine.provider();

    let prepare_started = Instant::now();
    provider.prepare().map_err(|error| error.to_string())?;
    let prepare_ms = prepare_started.elapsed().as_secs_f64() * 1000.0;

    let mut latencies_ms = Vec::new();
    let mut short_latencies_ms = Vec::new();
    let mut first_request_ms = f64::NAN;
    let mut edits = 0usize;
    let mut reference_words = 0usize;
    let mut errors = 0usize;
    let mut empty_outputs = 0usize;
    let mut processing = Duration::ZERO;
    let mut audio_seconds = 0.0;
    let mut per_utterance: Vec<UtteranceResult> = corpus
        .iter()
        .map(|utterance| UtteranceResult {
            id: utterance.id.clone(),
            seconds: utterance.seconds,
            reference_words: 0,
            edits: 0,
            latencies_ms: Vec::new(),
            hypothesis: None,
        })
        .collect();

    for pass in 0..args.passes.max(1) {
        for (index, utterance) in corpus.iter().enumerate() {
            let request = TranscriptionRequest {
                samples: &utterance.samples,
                language: "en",
                initial_prompt: None,
            };
            let started = Instant::now();
            let result = provider.transcribe(&request);
            let elapsed = started.elapsed();
            let elapsed_ms = elapsed.as_secs_f64() * 1000.0;

            if first_request_ms.is_nan() {
                first_request_ms = elapsed_ms;
            }
            latencies_ms.push(elapsed_ms);
            per_utterance[index].latencies_ms.push(elapsed_ms);
            if utterance.seconds <= 5.0 {
                short_latencies_ms.push(elapsed_ms);
            }
            processing += elapsed;
            audio_seconds += utterance.seconds;

            match result {
                Ok(transcription) => {
                    // Accuracy is deterministic at temperature 0, so score the first pass only.
                    if pass == 0 {
                        let (utterance_edits, words) =
                            word_errors(&utterance.reference, &transcription.text);
                        edits += utterance_edits;
                        reference_words += words;
                        per_utterance[index].edits = utterance_edits;
                        per_utterance[index].reference_words = words;
                        per_utterance[index].hypothesis = Some(transcription.text.clone());
                        if transcription.text.trim().is_empty() {
                            empty_outputs += 1;
                        }
                        if args.show_text {
                            println!(
                                "  {} [{elapsed_ms:.0} ms] {}",
                                utterance.id, transcription.text
                            );
                        }
                    }
                }
                Err(error) => {
                    errors += 1;
                    if pass == 0 {
                        let (_, words) = word_errors(&utterance.reference, "");
                        edits += words;
                        reference_words += words;
                        per_utterance[index].edits = words;
                        per_utterance[index].reference_words = words;
                    }
                    eprintln!("  {} failed: {error}", utterance.id);
                }
            }
        }
    }

    let peak_memory_mb = engine
        .peak_memory_bytes()
        .map(|bytes| bytes as f64 / (1024.0 * 1024.0));

    // Idle gap: real dictation has pauses between utterances, unlike the back-to-back loop
    // above. Let the engine sit idle, then time one request.
    let after_idle_ms = match (args.idle_secs, corpus.first()) {
        (0, _) | (_, None) => None,
        (seconds, Some(utterance)) => {
            std::thread::sleep(Duration::from_secs(seconds));
            let started = Instant::now();
            let ok = provider
                .transcribe(&TranscriptionRequest {
                    samples: &utterance.samples,
                    language: "en",
                    initial_prompt: None,
                })
                .is_ok();
            ok.then(|| started.elapsed().as_secs_f64() * 1000.0)
        }
    };

    // Non-speech robustness: Whisper tends to invent text for silence and noise.
    let probes = non_speech_probes();
    let non_speech_hallucinations = probes
        .iter()
        .filter(|samples| {
            provider
                .transcribe(&TranscriptionRequest {
                    samples,
                    language: "en",
                    initial_prompt: None,
                })
                .is_ok_and(|transcription| !is_blank_transcript(&transcription.text))
        })
        .count();

    // Recovery: kill the persistent server as a crash would, then time the next request,
    // which must restart it and still return text.
    let recovery_ms = match (&*engine, corpus.first()) {
        (Engine::Server(server), Some(utterance)) => {
            server.simulate_crash();
            let started = Instant::now();
            let ok = server
                .transcribe(&TranscriptionRequest {
                    samples: &utterance.samples,
                    language: "en",
                    initial_prompt: None,
                })
                .is_ok();
            ok.then(|| started.elapsed().as_secs_f64() * 1000.0)
        }
        _ => None,
    };

    // Concurrency: overlapping requests must all succeed (the server serialises them).
    let (concurrent_wall_ms, concurrent_failures) = if args.concurrency > 1 && !corpus.is_empty() {
        let started = Instant::now();
        let handles: Vec<_> = (0..args.concurrency)
            .map(|index| {
                let engine = engine.clone();
                let samples = corpus[index % corpus.len()].samples.clone();
                std::thread::spawn(move || {
                    engine
                        .provider()
                        .transcribe(&TranscriptionRequest {
                            samples: &samples,
                            language: "en",
                            initial_prompt: None,
                        })
                        .is_ok()
                })
            })
            .collect();
        let failures = handles
            .into_iter()
            .map(|handle| handle.join().unwrap_or(false))
            .filter(|ok| !ok)
            .count();
        (
            Some(started.elapsed().as_secs_f64() * 1000.0),
            Some(failures),
        )
    } else {
        (None, None)
    };

    let description = provider.describe();
    provider.shutdown();

    let mut sorted = latencies_ms.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    short_latencies_ms.sort_by(|a, b| a.total_cmp(b));

    Ok(ConfigResult {
        spec: spec.to_string(),
        description,
        utterances: corpus.len(),
        audio_seconds,
        prepare_ms,
        first_request_ms,
        p50_ms: percentile(&sorted, 0.50),
        p95_ms: percentile(&sorted, 0.95),
        mean_ms: sorted.iter().sum::<f64>() / sorted.len().max(1) as f64,
        max_ms: sorted.last().copied().unwrap_or(f64::NAN),
        short_p50_ms: percentile(&short_latencies_ms, 0.50),
        real_time_factor: processing.as_secs_f64() / audio_seconds.max(f64::EPSILON),
        wer_percent: 100.0 * edits as f64 / reference_words.max(1) as f64,
        errors,
        empty_outputs,
        peak_memory_mb,
        recovery_ms,
        concurrent_wall_ms,
        concurrent_failures,
        non_speech_hallucinations,
        non_speech_probes: probes.len(),
        after_idle_ms,
        per_utterance,
    })
}

fn optional(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".to_string(), |value| format!("{value:.0}"))
}

fn print_table(results: &[ConfigResult]) {
    println!(
        "\n| Config | p50 ms | p95 ms | mean ms | p50 ms (clips <= 5 s) | RTF | WER % | Errors | Non-speech hallucinations | Peak RSS MB | Ready ms | First ms | Crash recovery ms |"
    );
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for result in results {
        println!(
            "| `{}` | {:.0} | {:.0} | {:.0} | {:.0} | {:.3} | {:.2} | {} | {}/{} | {} | {:.0} | {:.0} | {} |",
            result.spec,
            result.p50_ms,
            result.p95_ms,
            result.mean_ms,
            result.short_p50_ms,
            result.real_time_factor,
            result.wer_percent,
            result.errors,
            result.non_speech_hallucinations,
            result.non_speech_probes,
            optional(result.peak_memory_mb),
            result.prepare_ms,
            result.first_request_ms,
            optional(result.recovery_ms),
        );
    }

    if let Some((base, others)) = results.split_first() {
        for other in others {
            let agreement = agreement(base, other);
            println!(
                "`{}` vs `{}`: identical transcripts {}/{} clips, {} word edits in {} words ({:.2} %)",
                other.spec,
                base.spec,
                agreement.identical_clips,
                agreement.compared_clips,
                agreement.word_edits,
                agreement.base_words,
                100.0 * agreement.word_edits as f64 / agreement.base_words.max(1) as f64
            );
        }
    }
    for result in results {
        println!("`{}`: {}", result.spec, result.description);
    }
}

fn to_json(results: &[ConfigResult], corpus: &[Utterance], args: &Args) -> serde_json::Value {
    let configs: Vec<serde_json::Value> = results
        .iter()
        .map(|result| {
            let mut map = BTreeMap::new();
            if let Some(base) = results.first().filter(|base| !std::ptr::eq(*base, result)) {
                let agreement = agreement(base, result);
                map.insert(
                    "agreementWithFirst",
                    serde_json::json!({
                        "baseConfig": base.spec,
                        "identicalClips": agreement.identical_clips,
                        "comparedClips": agreement.compared_clips,
                        "wordEdits": agreement.word_edits,
                        "baseWords": agreement.base_words,
                    }),
                );
            }
            map.insert("config", serde_json::json!(result.spec));
            map.insert("description", serde_json::json!(result.description));
            map.insert("utterances", serde_json::json!(result.utterances));
            map.insert(
                "audioSecondsProcessed",
                serde_json::json!(result.audio_seconds),
            );
            map.insert("prepareMs", serde_json::json!(result.prepare_ms));
            map.insert("firstRequestMs", serde_json::json!(result.first_request_ms));
            map.insert("p50Ms", serde_json::json!(result.p50_ms));
            map.insert("p95Ms", serde_json::json!(result.p95_ms));
            map.insert("meanMs", serde_json::json!(result.mean_ms));
            map.insert("maxMs", serde_json::json!(result.max_ms));
            map.insert("shortClipP50Ms", serde_json::json!(result.short_p50_ms));
            map.insert("realTimeFactor", serde_json::json!(result.real_time_factor));
            map.insert("werPercent", serde_json::json!(result.wer_percent));
            map.insert("errors", serde_json::json!(result.errors));
            map.insert("emptyOutputs", serde_json::json!(result.empty_outputs));
            map.insert("peakMemoryMb", serde_json::json!(result.peak_memory_mb));
            map.insert("crashRecoveryMs", serde_json::json!(result.recovery_ms));
            map.insert(
                "concurrentWallMs",
                serde_json::json!(result.concurrent_wall_ms),
            );
            map.insert(
                "concurrentFailures",
                serde_json::json!(result.concurrent_failures),
            );
            map.insert(
                "nonSpeechHallucinations",
                serde_json::json!(result.non_speech_hallucinations),
            );
            map.insert(
                "nonSpeechProbes",
                serde_json::json!(result.non_speech_probes),
            );
            map.insert("afterIdleMs", serde_json::json!(result.after_idle_ms));
            map.insert(
                "perUtterance",
                serde_json::json!(result
                    .per_utterance
                    .iter()
                    .map(|utterance| serde_json::json!({
                        "id": utterance.id,
                        "seconds": utterance.seconds,
                        "referenceWords": utterance.reference_words,
                        "edits": utterance.edits,
                        "latenciesMs": utterance.latencies_ms,
                    }))
                    .collect::<Vec<_>>()),
            );
            serde_json::json!(map)
        })
        .collect();

    serde_json::json!({
        "corpus": {
            "utterances": corpus.len(),
            "audioSeconds": corpus.iter().map(|utterance| utterance.seconds).sum::<f64>(),
            "minClipSeconds": args.min_seconds,
            "maxClipSeconds": args.max_seconds.is_finite().then_some(args.max_seconds),
        },
        "passes": args.passes,
        "warmUp": args.warm_up,
        "gpuBuild": localflow_lib::asr::transcribe_cpp::GPU_BUILD,
        "dynamicAudioContext": localflow_lib::asr::describe_dynamic_audio_context(),
        "logicalCpus": std::thread::available_parallelism().map(|count| count.get()).unwrap_or(0),
        "configs": configs,
    })
}

fn main() {
    let args = match parse_args() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };

    if args.list_devices {
        print_devices();
        if args.configs.is_empty() {
            return;
        }
    }

    let corpus = match load_corpus(&args) {
        Ok(corpus) if !corpus.is_empty() => corpus,
        Ok(_) => {
            eprintln!("error: corpus is empty");
            std::process::exit(2);
        }
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };
    let total_seconds: f64 = corpus.iter().map(|utterance| utterance.seconds).sum();
    println!(
        "corpus: {} utterances, {:.1} s of audio, {} pass(es)",
        corpus.len(),
        total_seconds,
        args.passes
    );

    let mut results = Vec::new();
    for spec in &args.configs {
        println!("running {spec} ...");
        match run_config(spec, &args, &corpus) {
            Ok(result) => {
                // Report and persist after every config so a long run can be followed and an
                // interrupted one keeps what it measured.
                println!(
                    "  p50 {:.0} ms, p95 {:.0} ms, WER {:.2} %, errors {}, non-speech hallucinations {}/{}",
                    result.p50_ms,
                    result.p95_ms,
                    result.wer_percent,
                    result.errors,
                    result.non_speech_hallucinations,
                    result.non_speech_probes
                );
                results.push(result);
                write_json(&args, &results, &corpus);
            }
            Err(error) => eprintln!("  {spec} could not run: {error}"),
        }
    }

    print_table(&results);

    if let Some(path) = &args.json {
        println!(
            "
wrote {}",
            path.display()
        );
    }
}

fn write_json(args: &Args, results: &[ConfigResult], corpus: &[Utterance]) {
    let Some(path) = &args.json else {
        return;
    };
    let payload = serde_json::to_string_pretty(&to_json(results, corpus, args))
        .expect("benchmark results serialise");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(path, payload) {
        eprintln!("could not write {}: {error}", path.display());
    }
}
