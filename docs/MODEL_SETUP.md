# Model Setup

LocalFlow does not download large model files during normal builds or tests.

## Whisper.cpp

This workstation has a local dev runtime in `.localflow-runtime/`:

- `whisper\Release\whisper-cli.exe`
- `models\ggml-tiny.en-q5_1.bin`

The native app first checks explicit environment overrides, then bundled Tauri resources, then the dev `.localflow-runtime/` folder.

```powershell
$env:LOCALFLOW_WHISPER_CLI = "C:\path\to\whisper-cli.exe"
$env:LOCALFLOW_WHISPER_MODEL = "C:\path\to\ggml-base.en.bin"
```

The native hotkey path records audio to a temporary mono 16 kHz WAV, runs `whisper-cli.exe` locally (biasing recognition with your dictionary via `--prompt`), parses JSON output, applies deterministic formatting, passes the result to your configured local Ollama model for cleanup, verifies the target window, inserts the cleaned text, and deletes the temporary files.

Still pending: persistent/streaming ASR, sidecar health checks, process recovery, hardware acceleration detection, and rolling partial transcription. (Cancellation, timeout enforcement, and model warm-up are implemented.)

## Ollama

Install Ollama and make sure the local model you want is available. The default is a fast
small model, `llama3.2:3b` — pull it once:

```powershell
ollama pull llama3.2:3b
```

It is chosen for low latency: LocalFlow's deterministic layer already handles punctuation,
self-corrections, filler cleanup, and your replacements/snippets, so the LLM only does light
semantic cleanup and a small model is plenty (this mirrors the small Llama-family model Wispr
Flow fine-tunes for its cleanup). You can change the model in Settings > Models — good
alternatives are `qwen2.5:3b` (quality) or the original `gemma4:12b-it-qat` (heaviest, slowest)
— and native dictation uses your choice (it never silently switches to a remote model). To
skip the LLM entirely for the fastest possible insertion, enable low-resource mode; the
deterministically formatted text is inserted directly.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

The shared provider calls `http://127.0.0.1:11434/api/tags` for discovery and `http://127.0.0.1:11434/api/generate` for cleanup. Native dictation calls `http://127.0.0.1:11434/api/generate` with the configured model, `stream: false`, and JSON-format output.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. If the model is unavailable or returns invalid JSON twice, LocalFlow falls back to the deterministically formatted transcript so the user does not lose dictated text.
