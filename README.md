# LocalFlow

LocalFlow is a local-first Windows desktop voice-dictation app built with Tauri 2, Rust, React, TypeScript, SQLite, `whisper.cpp`, and local LLM refinement through Ollama.

The current build is a lightweight Tauri tray app. It launches quietly, keeps the settings window hidden until opened from the tray, and shows a small frosted waveform pill while dictation is active. Tap the global hotkey, speak into the default microphone, then pause briefly after speaking; LocalFlow transcribes with local `whisper.cpp`, applies deterministic formatting, pastes quickly into the field where you started, then runs configured local Ollama cleanup in the background for recovery and diagnostics. Longer hold-and-release dictation is still supported.

On the native path LocalFlow:

- **Formats deterministically before the LLM** — spoken punctuation ("comma", "new line", "bullet point"), explicit self-corrections ("meet Tuesday, actually Wednesday"), filler/stutter cleanup, and sentence capitalization, while protecting URLs, emails, and decimals.
- **Personalizes from your settings** — exact replacements and snippets are applied, and your dictionary biases recognition via whisper's initial prompt. The refinement model is configurable (default `llama3.2:3b`, a fast small model; low-resource mode skips the LLM for instant text).
- **Inserts safely** — each dictation has a session id; it never pastes into a different window than where you started, a superseded dictation never inserts, and **Escape cancels** while you are speaking.
- **Never loses a transcript** — if insertion is skipped (focus changed) or the LLM is unavailable, the transcript is recoverable with "Copy last transcript" (tray or Home) and the deterministically formatted text is used as a fallback.

## Documentation

Full documentation lives in [docs/](docs/README.md), including [architecture](docs/ARCHITECTURE.md), [model setup](docs/MODEL_SETUP.md), [testing](docs/TESTING.md), [troubleshooting](docs/TROUBLESHOOTING.md), [privacy](docs/PRIVACY.md), and the [roadmap](docs/ROADMAP.md). Security policy is in [SECURITY.md](SECURITY.md).

## Prerequisites

- Windows 11
- Node.js 22 or newer
- npm 10 or newer
- Rust stable with Cargo
- Microsoft C++ Build Tools with the VCTools workload
- WebView2 Runtime
- Ollama with a local model for native cleanup — default `llama3.2:3b` (`ollama pull llama3.2:3b`); any installed model can be selected in Settings, or enable low-resource mode to skip the LLM

Install/check common prerequisites:

```powershell
.\scripts\Install-Prereqs.ps1 -Install
npm install
```

## Local Whisper Runtime

For this workstation, the local runtime assets are in `.localflow-runtime/`:

- `.localflow-runtime\whisper\Release\whisper-cli.exe`
- `.localflow-runtime\models\ggml-tiny.en-q5_1.bin`

The folder is ignored by Git. The packaged app bundles the Whisper executables, DLLs, and tiny English model into app resources; developer builds can also use `.localflow-runtime/` directly. You can override paths with:

```powershell
$env:LOCALFLOW_WHISPER_CLI = "C:\path\to\whisper-cli.exe"
$env:LOCALFLOW_WHISPER_MODEL = "C:\path\to\ggml-base.en.bin"
```

## Run The Desktop App

Start the packaged background app without Vite or a browser dev server:

```powershell
.\scripts\Start-LocalFlow.ps1
# or
npm start
```

The app starts hidden in the tray. Open settings from the tray icon when needed; normal dictation only shows the floating waveform overlay. A desktop shortcut can point at `scripts\Start-LocalFlow.vbs` to run the same launcher without a visible terminal window.

## Development Only

Start the Tauri dev app with Vite:

```powershell
.\scripts\Start-Dev.ps1
```

Or directly:

```powershell
npm run tauri:dev
```

If `Ctrl+Alt+Space` is already registered by another app, LocalFlow automatically tries `Ctrl+Alt+Shift+Space`.

## Native Dictation Test

1. Start LocalFlow with `.\scripts\Start-LocalFlow.ps1`.
2. Open Notepad, VS Code, or any text box.
3. Click into the target field.
4. Tap `Ctrl+Alt+Shift+Space` if the primary hotkey is unavailable; otherwise tap `Ctrl+Alt+Space`.
5. Speak for 2-5 seconds.
6. Stop speaking and pause briefly, or press the hotkey again, then wait for local Whisper transcription and deterministic quick insertion. Local cleanup runs afterward in the background unless low-resource mode is enabled.

The floating waveform appears while listening and processing. It uses live microphone level, pitch, and brightness estimates to draw a polished layered wave: higher-pitch speech lifts warmer upper harmonics, while lower-pitch speech deepens cooler lower harmonics. The native path inserts the deterministically formatted transcript through clipboard paste (restoring the previous text clipboard afterward) only after confirming the original window still has focus. Diagnostics records per-stage latency, including speech-end-to-visible-text, Whisper sidecar time, and background Ollama cleanup time.

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

The Tauri bundle config includes the local Whisper executables, required DLLs, and the tiny English model as resources. A successful Windows build creates:

- `src-tauri\target\release\localflow.exe`
- `src-tauri\target\release\bundle\msi\LocalFlow_0.1.0_x64_en-US.msi`
- `src-tauri\target\release\bundle\nsis\LocalFlow_0.1.0_x64-setup.exe`

## Current Limitations

- Native hotkey dictation currently uses the default input device.
- The native path applies deterministic formatting + your replacements/snippets/dictionary before insertion, then runs a configurable local cleanup model (default `llama3.2:3b`) in the background unless low-resource mode is on. ASR language is fixed to English for now.
- End-of-speech detection uses fixed native thresholds tuned for a snappy feel (~550 ms after you stop; ~2.5 s idle close); a settings control is still pending.
- Clipboard fallback restores only prior text clipboard content, not full rich clipboard formats.
- UI Automation insertion and startup-at-login are still pending.
- Manual speech insertion has to be checked by the user in real target apps; automated checks cannot speak into the microphone.
