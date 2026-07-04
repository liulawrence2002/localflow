# LocalFlow

LocalFlow is a local-first Windows desktop voice-dictation app built with Tauri 2, Rust, React, TypeScript, SQLite, `whisper.cpp`, and local LLM refinement through Ollama.

The current repository contains Milestone 1 foundation work plus build-tested shared logic for early Milestone 2/3/4 concerns: session-id stale-result rejection, audio ring buffers, VAD/end-of-speech detection, rolling ASR windows, `whisper.cpp` sidecar planning/parsing, cleanup JSON repair/fallback, a local-only Ollama provider, timeout guards, performance instrumentation, privacy retention, context policy, diagnostics export helpers, and editable personalization/style settings.

## Prerequisites

- Windows 11
- Node.js 22 or newer
- npm 10 or newer
- Rust stable with Cargo
- Microsoft C++ build tools required by Tauri on Windows
- WebView2 Runtime
- Ollama for shared local refinement and later production native workflow wiring
- A `whisper.cpp` build and model file for local ASR in later milestones

Run:

```powershell
.\scripts\Install-Prereqs.ps1
npm install
```

## Development

```powershell
npm run dev
```

For the desktop shell:

```powershell
npm run tauri:dev
```

## Checks

```powershell
npm run format
npm run lint
npm run test
npm run build
```

Or:

```powershell
.\scripts\Run-Checks.ps1
```

## Model Setup

LocalFlow does not download large models during builds or tests. Configure a `whisper.cpp` model path manually:

```powershell
.\scripts\Set-WhisperModelPath.ps1 -ModelPath "C:\models\ggml-base.en.bin"
```

Check Ollama:

```powershell
.\scripts\Check-Ollama.ps1
```

## Current Limitations

- The current environment used to create this milestone did not have Rust/Cargo on PATH, so native Tauri compilation must be run after prerequisites are installed.
- Real `cpal` audio capture, `whisper.cpp` sidecar execution, Windows UI Automation insertion, and production native Ollama workflow wiring are planned for later milestones.
- The shared `whisper.cpp` command/JSON contract is implemented, but the native sidecar process manager is not yet wired.
- The shared Ollama provider and Models screen discovery are implemented, but the production native dictation workflow still uses the mock local pipeline until real ASR and insertion are wired.
- Shared context privacy/categorization is implemented, but native Windows context capture is not yet wired.
- Shared performance metric helpers are implemented, but native services do not yet feed real measurements.
- The UI currently exercises the mock local pipeline, editable settings model, Ollama discovery, redacted diagnostics export preparation, and local browser fallback persistence.
