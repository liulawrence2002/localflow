# Model Setup

LocalFlow does not download large model files during normal builds or tests.

## Whisper.cpp

1. Build or install `whisper.cpp`.
2. Download a compatible local model through the source you trust.
3. Configure the model path:

```powershell
.\scripts\Set-WhisperModelPath.ps1 -ModelPath "C:\models\ggml-base.en.bin"
```

Milestone 2 still needs sidecar health checks, process-launch error recovery, native model-not-found checks, and hardware acceleration detection.

The shared sidecar contract now plans `whisper-cli` invocations with a configured model path, local audio path, language, thread count, optional vocabulary prompt, JSON output, and optional CPU-only mode. The native process manager that launches the sidecar is still pending.

## Ollama

Install Ollama and pull a local instruct model explicitly. LocalFlow discovers installed local models from the local Ollama API and lets the selected model be saved in the Models screen during browser/dev UI runs.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

The shared provider calls `http://127.0.0.1:11434/api/tags` for discovery and `http://127.0.0.1:11434/api/generate` for cleanup. Cleanup requests use `stream: false` and request JSON-format output.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. The production native dictation workflow still needs to be wired to this provider after real ASR and insertion are enabled.
