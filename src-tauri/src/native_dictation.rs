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
    started_at: Instant,
    level_stop_tx: mpsc::Sender<()>,
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
            "pressed" => start_recording(&command.app, &mut session),
            "released" => {
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

fn start_recording(app: &AppHandle, session: &mut Option<RecordingSession>) -> Result<(), String> {
    if session.is_some() {
        return Ok(());
    }

    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| "No default microphone is available.".to_string())?;
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
    let level_stop_tx = start_level_meter(app.clone(), samples.clone());

    *session = Some(RecordingSession {
        stream,
        samples,
        sample_rate,
        channels,
        started_at: Instant::now(),
        level_stop_tx,
    });

    emit_status(app, "listening", "Listening");
    tracing::info!(sample_rate, channels, "native microphone recording started");
    Ok(())
}

fn finish_recording(app: AppHandle, session: RecordingSession) -> Result<(), String> {
    let RecordingSession {
        stream,
        samples,
        sample_rate,
        channels,
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

    let output_dir = env::temp_dir().join("localflow");
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("Could not create temporary audio directory: {error}"))?;
    let stamp = chrono::Utc::now().timestamp_millis();
    let wav_path = output_dir.join(format!("dictation-{stamp}.wav"));
    let output_base = output_dir.join(format!("dictation-{stamp}"));

    let mono_16k = resample_to_16khz(&captured, sample_rate);
    write_wav(&wav_path, &mono_16k)?;

    let transcript = run_whisper(&app, &wav_path, &output_base)?;
    if transcript.trim().is_empty() {
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

fn start_level_meter(app: AppHandle, samples: Arc<Mutex<Vec<f32>>>) -> mpsc::Sender<()> {
    let (stop_tx, stop_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut cursor = 0usize;

        loop {
            thread::sleep(Duration::from_millis(70));
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

            let rms = (chunk.iter().map(|sample| sample * sample).sum::<f32>()
                / chunk.len() as f32)
                .sqrt();
            let level = (rms * 9.0).clamp(0.04, 1.0);
            emit_level(&app, level);
        }
    });

    stop_tx
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
        for frame in data.chunks(channels) {
            let sum: f32 = frame.iter().copied().sum();
            output.push((sum / frame.len() as f32).clamp(-1.0, 1.0));
        }
    }
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
        .arg("-l")
        .arg("en")
        .arg("-t")
        .arg("4")
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
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .map_err(|error| format!("Could not create local Ollama client: {error}"))?;

    let response = client
        .post(OLLAMA_GENERATE_URL)
        .json(&OllamaGenerateRequest {
            model: OLLAMA_MODEL,
            prompt,
            stream: false,
            format: "json",
            keep_alive: "10m",
            options: OllamaOptions { temperature: 0.1 },
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
        let x = position.x + ((size.width as i32 - 280) / 2).max(16);
        let y = position.y + (size.height as i32 - 172).max(16);
        window.set_position(PhysicalPosition::new(x, y))?;
    }

    Ok(())
}
