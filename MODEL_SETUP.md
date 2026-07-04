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

Install Ollama and pull a local instruct model explicitly. LocalFlow will discover installed local models in a later milestone.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

Localhost communication with Ollama is allowed. Remote model fallback is not allowed.
