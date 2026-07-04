# LocalFlow

LocalFlow is a local-first Windows desktop voice-dictation app built with Tauri 2, Rust, React, TypeScript, SQLite, `whisper.cpp`, and local LLM refinement through Ollama.

The current build is a lightweight Tauri tray app. It launches quietly, keeps the settings window hidden until opened from the tray, and shows a small floating waveform while dictation is active. Hold the global hotkey, speak into the default microphone, release the hotkey, and LocalFlow transcribes with local `whisper.cpp`, cleans with your local Ollama `gemma4:12b-it-qat` model, then pastes the result into the focused field.

## Prerequisites

- Windows 11
- Node.js 22 or newer
- npm 10 or newer
- Rust stable with Cargo
- Microsoft C++ Build Tools with the VCTools workload
- WebView2 Runtime
- Ollama with local model `gemma4:12b-it-qat` for native cleanup

Install/check common prerequisites:

```powershell
.\scripts\Install-Prereqs.ps1 -Install
npm install
```

## Local Whisper Runtime

For this workstation, the local runtime assets are in `.localflow-runtime/`:

- `.localflow-runtime\whisper\Release\whisper-cli.exe`
- `.localflow-runtime\models\ggml-tiny.en-q5_1.bin`

The folder is ignored by Git. Tauri dev mode uses it directly, and Tauri bundles copy the Whisper CLI, DLLs, and tiny English model into app resources. You can override paths with:

```powershell
$env:LOCALFLOW_WHISPER_CLI = "C:\path\to\whisper-cli.exe"
$env:LOCALFLOW_WHISPER_MODEL = "C:\path\to\ggml-base.en.bin"
```

## Development

Start the desktop app:

```powershell
.\scripts\Start-Dev.ps1
```

Or directly:

```powershell
npm run tauri:dev
```

If `Ctrl+Alt+Space` is already registered by another app, LocalFlow automatically tries `Ctrl+Alt+Shift+Space`.

## Native Dictation Test

1. Start LocalFlow.
2. Open Notepad, VS Code, or any text box.
3. Click into the target field.
4. Hold `Ctrl+Alt+Shift+Space` if the primary hotkey is unavailable; otherwise hold `Ctrl+Alt+Space`.
5. Speak for 2-5 seconds.
6. Release the hotkey and wait for local Whisper transcription plus local `gemma4:12b-it-qat` cleanup.

The floating waveform appears while listening and processing. The current native path inserts the cleaned transcript through clipboard paste and restores the previous text clipboard afterward. If local Ollama or `gemma4:12b-it-qat` is unavailable, LocalFlow preserves the raw Whisper transcript instead of losing the dictation.

For multi-channel microphones, LocalFlow selects the loudest active input channel instead of averaging channels, which avoids phase-cancellation recordings that sound like blank audio to Whisper.

## Checks

```powershell
.\scripts\Run-Checks.ps1
```

This runs frontend formatting, linting, tests, build, and Rust format/test/check when Cargo is available.

## Packaging

```powershell
.\scripts\Build-Installer.ps1
```

The Tauri bundle config includes the local Whisper CLI, required DLLs, and the tiny English model as resources. A successful Windows build creates:

- `src-tauri\target\release\localflow.exe`
- `src-tauri\target\release\bundle\msi\LocalFlow_0.1.0_x64_en-US.msi`
- `src-tauri\target\release\bundle\nsis\LocalFlow_0.1.0_x64-setup.exe`

## Current Limitations

- Native hotkey dictation currently uses the default input device.
- The native hotkey path always requests cleanup from local Ollama model `gemma4:12b-it-qat`; deterministic personalization is still pending.
- Clipboard fallback restores only prior text clipboard content, not full rich clipboard formats.
- UI Automation insertion, target-window verification, and startup-at-login are still pending.
- Manual speech insertion has to be checked by the user in real target apps; automated checks cannot speak into the microphone.
