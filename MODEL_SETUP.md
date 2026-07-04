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

The current native hotkey path records audio to a temporary mono 16 kHz WAV, runs `whisper-cli.exe` locally, parses JSON output, passes the transcript to local Ollama `gemma4:12b-it-qat` for cleanup, inserts the cleaned text, and deletes the temporary files.

Still pending: sidecar health checks, process recovery, cancellation, timeout enforcement, model warm-up, hardware acceleration detection, and rolling partial transcription.

## Ollama

Install Ollama and make sure the local model name `gemma4:12b-it-qat` is available. Native dictation is pinned to `gemma4:12b-it-qat` and does not silently choose another model.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

The shared provider calls `http://127.0.0.1:11434/api/tags` for discovery and `http://127.0.0.1:11434/api/generate` for cleanup. Native dictation calls `http://127.0.0.1:11434/api/generate` with model `gemma4:12b-it-qat`, `stream: false`, and JSON-format output.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. If `gemma4:12b-it-qat` is unavailable or returns invalid JSON twice, LocalFlow falls back to the raw Whisper transcript so the user does not lose dictated text.
