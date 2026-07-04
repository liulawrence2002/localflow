use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use arboard::Clipboard;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    SampleFormat, Stream, StreamConfig,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VIRTUAL_KEY, VK_CONTROL, VK_V,
};

pub struct NativeDictationRuntime {
    command_tx: Mutex<mpsc::Sender<RecorderCommand>>,
    overlay_epoch: AtomicU64,
}

struct RecordingSession {
    stream: Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
    device_name: String,
    sample_format: SampleFormat,
    started_at: Instant,
    level_stop_tx: mpsc::Sender<()>,
}

#[derive(Debug, Clone, Copy)]
struct AudioStats {
    duration_secs: f32,
    peak: f32,
    rms: f32,
    nonzero_ratio: f32,
}

struct RecorderCommand {
    app: AppHandle,
    state: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeDictationEvent {
    phase: String,
    message: String,
    level: Option<f32>,
}

#[derive(Debug, Serialize)]
struct OllamaGenerateRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    stream: bool,
    format: &'a str,
    keep_alive: &'a str,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct OllamaGenerateResponse {
    response: String,
}

#[derive(Debug, Deserialize)]
struct CleanupModelResponse {
    text: String,
}

const OLLAMA_MODEL: &str = "gemma4:12b-it-qat";
const OLLAMA_GENERATE_URL: &str = "http://127.0.0.1:11434/api/generate";
const SILENCE_PEAK_THRESHOLD: f32 = 0.003;
const SILENCE_RMS_THRESHOLD: f32 = 0.0005;
const VAD_POLL_MS: u64 = 55;
const VAD_START_RMS_THRESHOLD: f32 = 0.008;
const VAD_CONTINUE_RMS_THRESHOLD: f32 = 0.0045;
const VAD_CONFIRM_SPEECH_MS: u64 = 120;
const END_OF_SPEECH_TIMEOUT_MS: u64 = 760;
const MIN_AUTO_STOP_RECORDING_MS: u64 = 420;
const QUICK_TAP_RELEASE_MS: u64 = 700;
const NO_SPEECH_TIMEOUT_MS: u64 = 6_000;
const MAX_RECORDING_SECS: u64 = 120;
const OLLAMA_KEEP_ALIVE: &str = "30m";
const OVERLAY_WIDTH: i32 = 320;

impl Default for NativeDictationRuntime {
    fn default() -> Self {
        let (command_tx, command_rx) = mpsc::channel();
        thread::spawn(move || recorder_loop(command_rx));

        Self {
            command_tx: Mutex::new(command_tx),
            overlay_epoch: AtomicU64::new(0),
        }
    }
}

pub fn handle_hotkey(app: AppHandle, state: &str) -> Result<(), String> {
    let runtime = app.state::<NativeDictationRuntime>();
    let command_tx = runtime
        .command_tx
        .lock()
        .map_err(|_| "Native dictation command lock was poisoned.".to_string())?
        .clone();

    command_tx
        .send(RecorderCommand {
            app: app.clone(),
            state: state.to_string(),
        })
        .map_err(|error| format!("Could not send native dictation command: {error}"))
}

fn recorder_loop(command_rx: mpsc::Receiver<RecorderCommand>) {
    let mut session: Option<RecordingSession> = None;

    for command in command_rx {
        let result = match command.state.as_str() {
            "pressed" => {
                if let Some(active_session) = session.as_ref() {
                    if should_toggle_stop(active_session.started_at.elapsed()) {
                        let active_session = session
                            .take()
                            .expect("active recording session should exist");
                        finish_recording(command.app.clone(), active_session)
                    } else {
                        Ok(())
                    }
                } else {
                    start_recording(&command.app, &mut session)
                }
            }
            "released" => {
                if let Some(active_session) = session.as_ref() {
                    if should_ignore_quick_release(active_session.started_at.elapsed()) {
                        tracing::info!("quick hotkey release ignored; continuing tap-to-dictate");
                        Ok(())
                    } else {
                        let active_session = session
                            .take()
                            .expect("active recording session should exist");
                        finish_recording(command.app.clone(), active_session)
                    }
                } else {
                    Ok(())
                }
            }
            "auto_stop" => {
                if let Some(session) = session.take() {
                    finish_recording(command.app.clone(), session)
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };

        if let Err(error) = result {
            tracing::warn!(error = %error, "native dictation command failed");
            emit_status(&command.app, "error", &error);
        }
    }
}

fn should_ignore_quick_release(elapsed: Duration) -> bool {
    elapsed < Duration::from_millis(QUICK_TAP_RELEASE_MS)
}

fn should_toggle_stop(elapsed: Duration) -> bool {
    elapsed >= Duration::from_millis(QUICK_TAP_RELEASE_MS)
}

fn start_recording(app: &AppHandle, session: &mut Option<RecordingSession>) -> Result<(), String> {
    if session.is_some() {
        return Ok(());
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default microphone is available.".to_string())?;
    let device_name = device
        .name()
        .unwrap_or_else(|_| "Unknown input device".to_string());
    let supported_config = device
        .default_input_config()
        .map_err(|error| format!("Could not read default microphone config: {error}"))?;
    let sample_rate = supported_config.sample_rate().0;
    let channels = supported_config.channels();
    let sample_format = supported_config.sample_format();
    let stream_config: StreamConfig = supported_config.into();
    let samples = Arc::new(Mutex::new(Vec::with_capacity(sample_rate as usize * 20)));

    let stream = build_stream(&device, &stream_config, sample_format, samples.clone())?;
    stream
        .play()
        .map_err(|error| format!("Could not start microphone capture: {error}"))?;
    let command_tx = app
        .state::<NativeDictationRuntime>()
        .command_tx
        .lock()
        .map_err(|_| "Native dictation command lock was poisoned.".to_string())?
        .clone();
    let level_stop_tx = start_level_meter(app.clone(), samples.clone(), command_tx);

    *session = Some(RecordingSession {
        stream,
        samples,
        sample_rate,
        channels,
        device_name: device_name.clone(),
        sample_format,
        started_at: Instant::now(),
        level_stop_tx,
    });

    emit_status(app, "listening", "Listening");
    warm_pinned_ollama_in_background();
    tracing::info!(
        device = %device_name,
        sample_rate,
        channels,
        sample_format = ?sample_format,
        "native microphone recording started"
    );
    Ok(())
}

fn finish_recording(app: AppHandle, session: RecordingSession) -> Result<(), String> {
    let RecordingSession {
        stream,
        samples,
        sample_rate,
        channels,
        device_name,
        sample_format,
        started_at,
        level_stop_tx,
    } = session;

    let _ = level_stop_tx.send(());
    drop(stream);
    emit_status(&app, "processing", "Transcribing with local whisper.cpp");

    let captured = samples
        .lock()
        .map_err(|_| "Captured audio lock was poisoned.".to_string())?
        .clone();

    if captured.len() < sample_rate as usize / 4 {
        return Err("Recording was too short to transcribe.".to_string());
    }

    let stats = analyze_audio(&captured, sample_rate);
    tracing::info!(
        device = %device_name,
        sample_rate,
        channels,
        sample_format = ?sample_format,
        duration_secs = stats.duration_secs,
        peak = stats.peak,
        rms = stats.rms,
        nonzero_ratio = stats.nonzero_ratio,
        "native microphone recording finished"
    );

    if is_near_silent(stats) {
        return Err(format!(
            "Captured audio from \"{device_name}\" was silent or near-silent. peak={:.5}, rms={:.5}. Check the Windows default input device, microphone privacy access, and input gain.",
            stats.peak, stats.rms
        ));
    }

    let output_dir = env::temp_dir().join("localflow");
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Could not create temporary audio directory: {error}"))?;
    let stamp = chrono::Utc::now().timestamp_millis();
    let wav_path = output_dir.join(format!("dictation-{stamp}.wav"));
    let output_base = output_dir.join(format!("dictation-{stamp}"));

    let mono_16k = resample_to_16khz(&captured, sample_rate);
    write_wav(&wav_path, &mono_16k)?;

    let transcript = run_whisper(&app, &wav_path, &output_base)?;
    if transcript.trim().is_empty() || is_blank_transcript(&transcript) {
        return Err("Local whisper.cpp did not return any transcript text.".to_string());
    }

    emit_status(&app, "refining", "Cleaning with local gemma4:12b-it-qat");
    let final_text = match refine_with_pinned_ollama(&transcript) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(error = %error, "gemma4:12b-it-qat cleanup failed; using raw transcript");
            emit_status(
                &app,
                "error",
                "gemma4:12b-it-qat cleanup failed; using raw transcript",
            );
            transcript.clone()
        }
    };

    paste_text(&final_text)?;
    emit_status(&app, "inserted", "Inserted transcript");
    tracing::info!(
        elapsed_ms = started_at.elapsed().as_millis(),
        chars = final_text.chars().count(),
        sample_rate,
        channels,
        "native dictation inserted transcript"
    );

    let _ = fs::remove_file(&wav_path);
    let _ = fs::remove_file(output_base.with_extension("json"));

    Ok(())
}

fn start_level_meter(
    app: AppHandle,
    samples: Arc<Mutex<Vec<f32>>>,
    command_tx: mpsc::Sender<RecorderCommand>,
) -> mpsc::Sender<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut cursor = 0usize;
        let mut detector = EndOfSpeechDetector::new(Instant::now());

        loop {
            thread::sleep(Duration::from_millis(VAD_POLL_MS));
            if stop_rx.try_recv().is_ok() {
                break;
            }

            let chunk = match samples.lock() {
                Ok(output) => {
                    let start = cursor.min(output.len());
                    let next = output[start..].to_vec();
                    cursor = output.len();
                    next
                }
                Err(_) => Vec::new(),
            };

            if chunk.is_empty() {
                continue;
            }

            let rms = rms_level(&chunk);
            let level = display_level_from_rms(rms);
            emit_level(&app, level);

            if detector.observe(Instant::now(), rms) {
                let _ = command_tx.send(RecorderCommand {
                    app: app.clone(),
                    state: "auto_stop".to_string(),
                });
                break;
            }
        }
    });

    stop_tx
}

#[derive(Debug)]
struct EndOfSpeechDetector {
    started_at: Instant,
    speech_started_at: Option<Instant>,
    last_voice_at: Option<Instant>,
    speech_seen: bool,
}

impl EndOfSpeechDetector {
    fn new(started_at: Instant) -> Self {
        Self {
            started_at,
            speech_started_at: None,
            last_voice_at: None,
            speech_seen: false,
        }
    }

    fn observe(&mut self, now: Instant, rms: f32) -> bool {
        let elapsed = now.duration_since(self.started_at);

        if elapsed >= Duration::from_secs(MAX_RECORDING_SECS) {
            return true;
        }

        if !self.speech_seen && elapsed >= Duration::from_millis(NO_SPEECH_TIMEOUT_MS) {
            return true;
        }

        let voice_threshold = if self.speech_seen {
            VAD_CONTINUE_RMS_THRESHOLD
        } else {
            VAD_START_RMS_THRESHOLD
        };

        if rms >= voice_threshold {
            let speech_started_at = self.speech_started_at.get_or_insert(now);
            self.last_voice_at = Some(now);

            if now.duration_since(*speech_started_at)
                >= Duration::from_millis(VAD_CONFIRM_SPEECH_MS)
                || rms >= VAD_START_RMS_THRESHOLD * 1.6
            {
                self.speech_seen = true;
            }
        } else if !self.speech_seen {
            self.speech_started_at = None;
        }

        self.speech_seen
            && now.duration_since(self.started_at)
                >= Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS)
            && self.last_voice_at.is_some_and(|last_voice_at| {
                now.duration_since(last_voice_at) >= Duration::from_millis(END_OF_SPEECH_TIMEOUT_MS)
            })
    }
}

fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    (samples.iter().map(|sample| sample * sample).sum::<f32>() / samples.len() as f32).sqrt()
}

fn display_level_from_rms(rms: f32) -> f32 {
    (rms * 15.0).clamp(0.05, 1.0)
}

fn build_stream(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Result<Stream, String> {
    let channels = config.channels as usize;
    let error_callback = |error| tracing::warn!(error = %error, "microphone stream error");

    match sample_format {
        SampleFormat::F32 => device
            .build_input_stream(
                config,
                move |data: &[f32], _| push_mono_samples(data, channels, &samples),
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build f32 microphone stream: {error}")),
        SampleFormat::I16 => device
            .build_input_stream(
                config,
                move |data: &[i16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| *sample as f32 / i16::MAX as f32)
                        .collect();
                    push_mono_samples(&converted, channels, &samples);
                },
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build i16 microphone stream: {error}")),
        SampleFormat::U16 => device
            .build_input_stream(
                config,
                move |data: &[u16], _| {
                    let converted: Vec<f32> = data
                        .iter()
                        .map(|sample| (*sample as f32 - 32768.0) / 32768.0)
                        .collect();
                    push_mono_samples(&converted, channels, &samples);
                },
                error_callback,
                None,
            )
            .map_err(|error| format!("Could not build u16 microphone stream: {error}")),
        other => Err(format!("Unsupported microphone sample format: {other:?}")),
    }
}

fn push_mono_samples(data: &[f32], channels: usize, samples: &Arc<Mutex<Vec<f32>>>) {
    if channels == 0 {
        return;
    }

    if let Ok(mut output) = samples.lock() {
        output.extend(downmix_input_callback(data, channels));
    }
}

fn downmix_input_callback(data: &[f32], channels: usize) -> Vec<f32> {
    if channels == 0 {
        return Vec::new();
    }

    if channels == 1 {
        return data
            .iter()
            .copied()
            .map(|sample| sample.clamp(-1.0, 1.0))
            .collect();
    }

    let selected_channel = loudest_channel(data, channels);
    data.chunks(channels)
        .filter_map(|frame| frame.get(selected_channel).copied())
        .map(|sample| sample.clamp(-1.0, 1.0))
        .collect()
}

fn loudest_channel(data: &[f32], channels: usize) -> usize {
    let mut energy = vec![0.0f64; channels];
    let mut counts = vec![0usize; channels];

    for frame in data.chunks(channels) {
        for (channel, sample) in frame.iter().enumerate() {
            energy[channel] += (*sample as f64) * (*sample as f64);
            counts[channel] += 1;
        }
    }

    let mut selected = 0usize;
    let mut selected_energy = 0.0f64;

    for channel in 0..channels {
        let channel_energy = if counts[channel] == 0 {
            0.0
        } else {
            energy[channel] / counts[channel] as f64
        };

        if channel_energy > selected_energy {
            selected = channel;
            selected_energy = channel_energy;
        }
    }

    selected
}

fn analyze_audio(samples: &[f32], sample_rate: u32) -> AudioStats {
    if samples.is_empty() || sample_rate == 0 {
        return AudioStats {
            duration_secs: 0.0,
            peak: 0.0,
            rms: 0.0,
            nonzero_ratio: 0.0,
        };
    }

    let mut peak = 0.0f32;
    let mut sum_squares = 0.0f64;
    let mut nonzero = 0usize;

    for sample in samples {
        let abs = sample.abs();
        peak = peak.max(abs);
        sum_squares += (*sample as f64) * (*sample as f64);
        if abs > 0.000_01 {
            nonzero += 1;
        }
    }

    AudioStats {
        duration_secs: samples.len() as f32 / sample_rate as f32,
        peak,
        rms: (sum_squares / samples.len() as f64).sqrt() as f32,
        nonzero_ratio: nonzero as f32 / samples.len() as f32,
    }
}

fn is_near_silent(stats: AudioStats) -> bool {
    stats.peak < SILENCE_PEAK_THRESHOLD && stats.rms < SILENCE_RMS_THRESHOLD
}

fn resample_to_16khz(input: &[f32], input_rate: u32) -> Vec<f32> {
    const TARGET_RATE: u32 = 16_000;
    if input_rate == TARGET_RATE {
        return input.to_vec();
    }

    let output_len = ((input.len() as f64 / input_rate as f64) * TARGET_RATE as f64)
        .round()
        .max(1.0) as usize;
    let ratio = input_rate as f64 / TARGET_RATE as f64;

    (0..output_len)
        .map(|index| {
            let source = index as f64 * ratio;
            let lower = source.floor() as usize;
            let upper = (lower + 1).min(input.len().saturating_sub(1));
            let fraction = (source - lower as f64) as f32;
            let a = input.get(lower).copied().unwrap_or(0.0);
            let b = input.get(upper).copied().unwrap_or(a);
            a + (b - a) * fraction
        })
        .collect()
}

fn write_wav(path: &Path, samples: &[f32]) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|error| format!("WAV write failed: {error}"))?;

    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        writer
            .write_sample(pcm)
            .map_err(|error| format!("WAV sample write failed: {error}"))?;
    }

    writer
        .finalize()
        .map_err(|error| format!("WAV finalize failed: {error}"))
}

fn run_whisper(app: &AppHandle, wav_path: &Path, output_base: &Path) -> Result<String, String> {
    let cli = whisper_cli_path(app)?;
    let model = whisper_model_path(app)?;
    let thread_count = whisper_thread_count().to_string();
    let whisper_dir = cli
        .parent()
        .ok_or_else(|| "Could not resolve whisper.cpp runtime directory.".to_string())?;

    let output = Command::new(&cli)
        .current_dir(whisper_dir)
        .arg("-m")
        .arg(&model)
        .arg("-f")
        .arg(wav_path)
        .arg("-oj")
        .arg("-of")
        .arg(output_base)
        .arg("-np")
        .arg("-nt")
        .arg("-l")
        .arg("en")
        .arg("-t")
        .arg(thread_count)
        .output()
        .map_err(|error| format!("Could not start whisper.cpp: {error}"))?;

    if !output.status.success() {
        return Err(format!(
            "whisper.cpp failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    parse_whisper_json(&output_base.with_extension("json"))
}

fn whisper_thread_count() -> usize {
    thread::available_parallelism()
        .map(|available| available.get().saturating_sub(1).max(2))
        .unwrap_or(4)
        .clamp(2, 8)
}

fn parse_whisper_json(path: &Path) -> Result<String, String> {
    let payload = fs::read_to_string(path)
        .map_err(|error| format!("Could not read whisper.cpp JSON output: {error}"))?;
    let value: serde_json::Value = serde_json::from_str(&payload)
        .map_err(|error| format!("Could not parse whisper.cpp JSON output: {error}"))?;

    if let Some(text) = value.get("text").and_then(|item| item.as_str()) {
        return Ok(text.trim().to_string());
    }

    if let Some(segments) = value.get("transcription").and_then(|item| item.as_array()) {
        let text = segments
            .iter()
            .filter_map(|segment| segment.get("text").and_then(|item| item.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        return Ok(text.split_whitespace().collect::<Vec<_>>().join(" "));
    }

    Err("whisper.cpp JSON output did not include transcript text.".to_string())
}

fn is_blank_transcript(transcript: &str) -> bool {
    let normalized = transcript
        .trim()
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '[' | ']' | '(' | ')' | '{' | '}' | '.' | '!' | '?'
                )
        })
        .to_ascii_lowercase()
        .replace(['_', '-'], " ");

    matches!(
        normalized.as_str(),
        "" | "blank audio" | "silence" | "no speech" | "inaudible"
    )
}

fn refine_with_pinned_ollama(raw_transcript: &str) -> Result<String, String> {
    let first_prompt = build_cleanup_prompt(raw_transcript);
    let first_payload = request_ollama_cleanup(&first_prompt)?;

    match parse_cleanup_text(&first_payload) {
        Ok(text) => Ok(text),
        Err(first_error) => {
            let repair_prompt = build_repair_prompt(&first_payload, &first_error);
            let repaired_payload = request_ollama_cleanup(&repair_prompt)?;
            parse_cleanup_text(&repaired_payload).map_err(|repair_error| {
                format!(
                    "gemma4:12b-it-qat returned invalid cleanup JSON twice: {first_error}; {repair_error}"
                )
            })
        }
    }
}

fn request_ollama_cleanup(prompt: &str) -> Result<String, String> {
    request_ollama_generate(prompt, Duration::from_secs(60), 0.1)
}

fn request_ollama_generate(
    prompt: &str,
    timeout: Duration,
    temperature: f32,
) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| format!("Could not create local Ollama client: {error}"))?;

    let response = client
        .post(OLLAMA_GENERATE_URL)
        .json(&OllamaGenerateRequest {
            model: OLLAMA_MODEL,
            prompt,
            stream: false,
            format: "json",
            keep_alive: OLLAMA_KEEP_ALIVE,
            options: OllamaOptions { temperature },
        })
        .send()
        .map_err(|error| {
            format!("Could not reach local Ollama at {OLLAMA_GENERATE_URL}: {error}")
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().unwrap_or_default();
        if status.as_u16() == 404 {
            return Err(format!(
                "Ollama model \"{OLLAMA_MODEL}\" was not found locally. {body}"
            ));
        }
        return Err(format!("Ollama returned HTTP {status}: {body}"));
    }

    let payload: OllamaGenerateResponse = response
        .json()
        .map_err(|error| format!("Ollama returned invalid JSON: {error}"))?;

    Ok(payload.response)
}

fn warm_pinned_ollama_in_background() {
    thread::spawn(|| {
        let prompt = serde_json::json!({
            "task": "localflow.warm_dictation_cleanup",
            "instruction": "Return only JSON.",
            "requiredShape": {
                "text": "ready",
                "confidence": 1.0,
                "resolved_corrections": [],
                "warnings": []
            }
        })
        .to_string();

        if let Err(error) = request_ollama_generate(&prompt, Duration::from_secs(12), 0.0) {
            tracing::debug!(error = %error, "gemma4:12b-it-qat warmup skipped");
        }
    });
}

fn parse_cleanup_text(payload: &str) -> Result<String, String> {
    let value = parse_json_value(payload)?;
    let cleanup: CleanupModelResponse = serde_json::from_value(value)
        .map_err(|error| format!("Cleanup payload did not match the strict contract: {error}"))?;
    let text = cleanup.text.trim();

    if text.is_empty() {
        Err("Cleanup payload returned empty text.".to_string())
    } else {
        Ok(text.to_string())
    }
}

fn parse_json_value(payload: &str) -> Result<serde_json::Value, String> {
    let trimmed = payload.trim();
    serde_json::from_str(trimmed).or_else(|first_error| {
        let start = trimmed.find('{');
        let end = trimmed.rfind('}');

        match (start, end) {
            (Some(start), Some(end)) if start < end => serde_json::from_str(&trimmed[start..=end])
                .map_err(|second_error| {
                    format!("Invalid JSON: {first_error}; extraction failed: {second_error}")
                }),
            _ => Err(format!("Invalid JSON: {first_error}")),
        }
    })
}

fn build_cleanup_prompt(raw_transcript: &str) -> String {
    serde_json::json!({
        "task": "localflow.dictation_cleanup",
        "contract": "Return only strict JSON with text, confidence, resolved_corrections, and warnings.",
        "rules": [
            "Preserve meaning, facts, names, numbers, uncertainty, and intent.",
            "Never answer the dictated content.",
            "Never add new claims.",
            "Remove filler words only when meaning is unchanged.",
            "Resolve explicit self-corrections in favor of the latest correction.",
            "Add punctuation and capitalization conservatively.",
            "Return only JSON."
        ],
        "cleanupLevel": "balanced",
        "rawTranscript": raw_transcript
    })
    .to_string()
}

fn build_repair_prompt(invalid_payload: &str, error: &str) -> String {
    serde_json::json!({
        "task": "localflow.repair_cleanup_json",
        "instruction": "Convert the invalid payload to strict JSON only. Do not change the intended cleaned text.",
        "error": error,
        "invalidPayload": invalid_payload,
        "requiredShape": {
            "text": "final text",
            "confidence": 0.0,
            "resolved_corrections": [],
            "warnings": []
        }
    })
    .to_string()
}

fn paste_text(text: &str) -> Result<(), String> {
    let mut clipboard =
        Clipboard::new().map_err(|error| format!("Could not open clipboard: {error}"))?;
    let previous_text = clipboard.get_text().ok();
    clipboard
        .set_text(text.to_string())
        .map_err(|error| format!("Could not set clipboard text: {error}"))?;

    send_ctrl_v()?;
    thread::sleep(Duration::from_millis(700));

    if let Some(previous_text) = previous_text {
        let _ = clipboard.set_text(previous_text);
    }

    Ok(())
}

fn send_ctrl_v() -> Result<(), String> {
    let inputs = [
        key_input(VK_CONTROL, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYBD_EVENT_FLAGS(0)),
        key_input(VK_V, KEYEVENTF_KEYUP),
        key_input(VK_CONTROL, KEYEVENTF_KEYUP),
    ];

    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent == inputs.len() as u32 {
        Ok(())
    } else {
        Err(format!(
            "Could only send {sent} of {} key events.",
            inputs.len()
        ))
    }
}

fn key_input(key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn whisper_cli_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("LOCALFLOW_WHISPER_CLI") {
        return require_file(PathBuf::from(path), "whisper.cpp executable");
    }

    find_file(
        "whisper.cpp executable",
        runtime_candidates(app).into_iter().map(|runtime| {
            runtime
                .join("whisper")
                .join("Release")
                .join("whisper-cli.exe")
        }),
    )
}

fn whisper_model_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("LOCALFLOW_WHISPER_MODEL") {
        return require_file(PathBuf::from(path), "Whisper model");
    }

    find_file(
        "Whisper model",
        runtime_candidates(app)
            .into_iter()
            .map(|runtime| runtime.join("models").join("ggml-tiny.en-q5_1.bin")),
    )
}

fn runtime_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("localflow-runtime"));
    }

    candidates.push(workspace_runtime_dir());
    candidates
}

fn workspace_runtime_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".localflow-runtime")
}

fn require_file(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("Missing {label}: {}", path.display()))
    }
}

fn find_file(
    label: &str,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Result<PathBuf, String> {
    let mut checked = Vec::new();

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
        checked.push(candidate.display().to_string());
    }

    Err(format!("Missing {label}. Checked: {}", checked.join("; ")))
}

fn emit_status(app: &AppHandle, phase: &str, message: &str) {
    emit_native_event(app, phase, message, None);
}

fn emit_level(app: &AppHandle, level: f32) {
    emit_native_event(app, "listening", "Listening", Some(level));
}

fn emit_native_event(app: &AppHandle, phase: &str, message: &str, level: Option<f32>) {
    let epoch = app
        .state::<NativeDictationRuntime>()
        .overlay_epoch
        .fetch_add(1, Ordering::SeqCst)
        + 1;

    match phase {
        "listening" | "processing" | "refining" | "inserted" | "error" => {
            show_overlay(app, level.is_none());
        }
        _ => hide_overlay(app),
    }

    let _ = app.emit(
        "localflow://native-dictation",
        NativeDictationEvent {
            phase: phase.to_string(),
            message: message.to_string(),
            level,
        },
    );

    if matches!(phase, "inserted" | "error") && level.is_none() {
        schedule_overlay_hide(app.clone(), epoch);
    }
}

fn show_overlay(app: &AppHandle, reposition: bool) {
    if let Some(window) = app.get_webview_window("overlay") {
        if reposition {
            if let Err(error) = position_overlay(&window) {
                tracing::warn!(error = %error, "could not position voice overlay");
            }
        }
        let _ = window.show();
    }
}

fn hide_overlay(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("overlay") {
        let _ = window.hide();
    }
}

fn schedule_overlay_hide(app: AppHandle, epoch: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(1200));

        if app
            .state::<NativeDictationRuntime>()
            .overlay_epoch
            .load(Ordering::SeqCst)
            == epoch
        {
            hide_overlay(&app);
        }
    });
}

fn position_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    let monitor = window
        .current_monitor()?
        .or_else(|| window.primary_monitor().ok().flatten());

    if let Some(monitor) = monitor {
        let size = monitor.size();
        let position = monitor.position();
        let x = position.x + ((size.width as i32 - OVERLAY_WIDTH) / 2).max(16);
        let y = position.y + (size.height as i32 - 178).max(16);
        window.set_position(PhysicalPosition::new(x, y))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_uses_loudest_channel_to_avoid_phase_cancellation() {
        let interleaved = vec![0.8, -0.8, 0.7, -0.7, 0.6, -0.6, 0.5, -0.5];

        let mono = downmix_input_callback(&interleaved, 2);

        assert_eq!(mono, vec![0.8, 0.7, 0.6, 0.5]);
    }

    #[test]
    fn downmix_prefers_nonzero_channel_for_sparse_multichannel_input() {
        let interleaved = vec![0.0, 0.25, 0.0, 0.5, 0.0, 0.75];

        let mono = downmix_input_callback(&interleaved, 2);

        assert_eq!(mono, vec![0.25, 0.5, 0.75]);
    }

    #[test]
    fn audio_stats_detect_near_silence() {
        let silent = vec![0.0; 16_000];
        let speech = vec![0.02; 16_000];

        assert!(is_near_silent(analyze_audio(&silent, 16_000)));
        assert!(!is_near_silent(analyze_audio(&speech, 16_000)));
    }

    #[test]
    fn end_of_speech_detector_waits_for_actual_voice() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(started_at + Duration::from_millis(900), 0.0));
        assert!(!detector.observe(started_at + Duration::from_millis(1_800), 0.001));
    }

    #[test]
    fn quick_hotkey_release_is_treated_as_tap_to_start() {
        assert!(should_ignore_quick_release(Duration::from_millis(
            QUICK_TAP_RELEASE_MS - 1,
        )));
        assert!(!should_ignore_quick_release(Duration::from_millis(
            QUICK_TAP_RELEASE_MS,
        )));
        assert!(should_toggle_stop(Duration::from_millis(
            QUICK_TAP_RELEASE_MS,
        )));
    }

    #[test]
    fn end_of_speech_detector_times_out_when_no_voice_arrives() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS - 100),
            0.0,
        ));
        assert!(detector.observe(
            started_at + Duration::from_millis(NO_SPEECH_TIMEOUT_MS),
            0.0,
        ));
    }

    #[test]
    fn end_of_speech_detector_stops_after_post_speech_silence() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(!detector.observe(started_at + Duration::from_millis(80), 0.02));
        assert!(!detector.observe(started_at + Duration::from_millis(220), 0.012));
        assert!(!detector.observe(
            started_at + Duration::from_millis(MIN_AUTO_STOP_RECORDING_MS + 120),
            0.0,
        ));
        assert!(detector.observe(
            started_at
                + Duration::from_millis(
                    MIN_AUTO_STOP_RECORDING_MS + END_OF_SPEECH_TIMEOUT_MS + 120,
                ),
            0.0,
        ));
    }

    #[test]
    fn end_of_speech_detector_caps_long_recordings() {
        let started_at = Instant::now();
        let mut detector = EndOfSpeechDetector::new(started_at);

        assert!(detector.observe(
            started_at + Duration::from_secs(MAX_RECORDING_SECS + 1),
            0.0
        ));
    }

    #[test]
    fn detects_blank_whisper_markers() {
        assert!(is_blank_transcript("[BLANK_AUDIO]"));
        assert!(is_blank_transcript("(silence)"));
        assert!(!is_blank_transcript("hello local flow"));
    }
}
