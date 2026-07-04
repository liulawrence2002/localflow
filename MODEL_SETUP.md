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

The current native hotkey path records audio to a temporary mono 16 kHz WAV, runs `whisper-cli.exe` locally, parses JSON output, inserts the transcript, and deletes the temporary files.

Still pending: sidecar health checks, process recovery, cancellation, timeout enforcement, model warm-up, hardware acceleration detection, and rolling partial transcription.

## Ollama

Install Ollama and pull a local instruct model explicitly. LocalFlow discovers installed local models from the local Ollama API and lets the selected model be saved in the Models screen during browser/dev UI runs.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

The shared provider calls `http://127.0.0.1:11434/api/tags` for discovery and `http://127.0.0.1:11434/api/generate` for cleanup. Cleanup requests use `stream: false` and request JSON-format output.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. The native hotkey path still needs deterministic cleanup and Ollama refinement wiring; it currently inserts raw local Whisper output.
