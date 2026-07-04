# LocalFlow

LocalFlow is a local-first Windows desktop voice-dictation app built with Tauri 2, Rust, React, TypeScript, SQLite, `whisper.cpp`, and local LLM refinement through Ollama.

The current repository contains Milestone 1 foundation work: the Tauri shell, tray setup, global-hotkey integration, typed dictation state machine, mock ASR/refinement/insertion providers, SQLite schema initialization, settings UI, and automated TypeScript tests.

## Prerequisites

- Windows 11
- Node.js 22 or newer
- npm 10 or newer
- Rust stable with Cargo
- Microsoft C++ build tools required by Tauri on Windows
- WebView2 Runtime
- Ollama for local refinement in later milestones
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
- Audio capture, real `whisper.cpp` sidecar execution, Windows UI Automation insertion, and Ollama calls are planned for later milestones.
- The UI currently exercises the mock local pipeline and settings model.
