# Model Setup

LocalFlow does not download large model files during normal builds or tests.

## Whisper.cpp

1. Build or install `whisper.cpp`.
2. Download a compatible local model through the source you trust.
3. Configure the model path:

```powershell
.\scripts\Set-WhisperModelPath.ps1 -ModelPath "C:\models\ggml-base.en.bin"
```

Milestone 2 will add sidecar health checks, model-not-found errors, CPU thread settings, and hardware acceleration detection.

## Ollama

Install Ollama and pull a local instruct model explicitly. LocalFlow discovers installed local models from the local Ollama API and lets the selected model be saved in the Models screen during browser/dev UI runs.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

The shared provider calls `http://127.0.0.1:11434/api/tags` for discovery and `http://127.0.0.1:11434/api/generate` for cleanup. Cleanup requests use `stream: false` and request JSON-format output.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. The production native dictation workflow still needs to be wired to this provider after real ASR and insertion are enabled.
