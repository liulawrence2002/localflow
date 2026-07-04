# LocalFlow Handoff

## Goal

LocalFlow is a local-first Windows desktop dictation app. The intended user flow is:

1. The app runs quietly in the system tray.
2. The user focuses any text field in another app.
3. The user taps the global hotkey.
4. A small bottom waveform overlay appears.
5. LocalFlow records microphone audio.
6. Local `whisper.cpp` transcribes the audio.
7. Local Ollama model `gemma4:12b-it-qat` cleans the transcript.
8. The final text is pasted into the focused field.

The app is designed to avoid cloud services, telemetry, accounts, and silent remote fallback. Ordinary dictation is expected to work offline after the local Whisper runtime and Ollama model are installed.

## Current User Experience

- Main settings window starts hidden.
- The system tray icon can show or quit LocalFlow.
- Global hotkey is `Ctrl+Alt+Space` by default.
- If that shortcut is unavailable, LocalFlow falls back to `Ctrl+Alt+Shift+Space`.
- Quick hotkey taps start dictation and keep the overlay open.
- A second hotkey press or a longer hold-and-release can stop dictation manually.
- End-of-speech detection starts processing after the user pauses.
- If no voice arrives after a tap, LocalFlow times out instead of staying open forever.
- During dictation, only the small waveform overlay should appear.
- The `whisper-cli.exe` sidecar is launched with the Windows no-console flag to avoid terminal flashes.

## Architecture Overview

LocalFlow has two main layers:

- Native desktop layer: `src-tauri/`
- React/TypeScript UI and shared domain logic: `src/`

The native layer owns Windows desktop integration:

- Tray app lifecycle.
- Global hotkey registration.
- Microphone capture through `cpal`.
- Local Whisper sidecar execution.
- Local Ollama cleanup request.
- Clipboard paste insertion through Win32 keyboard simulation.
- Floating overlay window events.
- SQLite initialization.

The frontend layer owns:

- Settings and diagnostics UI.
- Mock workflow controls for development.
- Shared typed domain helpers.
- The waveform-only overlay view.
- Local provider planning and validation logic.

## Native Runtime Flow

The main native path is implemented in `src-tauri/src/native_dictation.rs`.

### Hotkey Handling

`src-tauri/src/hotkeys/mod.rs` registers global shortcuts through `tauri-plugin-global-shortcut`.

When a shortcut event arrives:

1. The hotkey module emits a `localflow://hotkey` event for the UI.
2. It calls `native_dictation::handle_hotkey`.
3. The native dictation runtime sends a command to a dedicated recorder thread.

The recorder thread prevents overlapping sessions and handles:

- `pressed`
- `released`
- internal `auto_stop`

Quick releases are treated as tap-to-start. Longer releases stop the active recording.

### Audio Capture

The native recorder:

- Opens the Windows default input device through `cpal`.
- Captures mono samples.
- Chooses the loudest input channel for multi-channel devices to avoid phase-cancellation silence.
- Tracks RMS, peak, duration, and nonzero sample ratio.
- Rejects near-silent recordings before Whisper runs.

The level meter thread also performs lightweight VAD:

- It watches speech RMS.
- It detects when real speech has started.
- It sends `auto_stop` after a short post-speech pause.
- It enforces a maximum recording duration.
- It enforces a no-speech timeout for abandoned tap-to-start sessions.

### Overlay Audio Features

Each microphone chunk produces overlay features:

- `level`: visible speech energy.
- `pitch`: normalized voice pitch estimate.
- `brightness`: zero-crossing based high-frequency estimate.

These features are emitted through the Tauri event:

```text
localflow://native-dictation
```

The overlay does not receive transcript text. It only receives state and audio features.

### Whisper Transcription

Current Whisper flow:

1. Captured audio is resampled to mono 16 kHz.
2. A temporary WAV is written under the OS temp directory.
3. `whisper-cli.exe` is launched locally.
4. JSON output is parsed.
5. Temporary WAV and JSON files are deleted after processing.

The app resolves the Whisper executable/model in this order:

1. `LOCALFLOW_WHISPER_CLI` and `LOCALFLOW_WHISPER_MODEL`.
2. Bundled Tauri resources.
3. `.localflow-runtime/` in the repository.

The sidecar is launched with:

- JSON output.
- English language.
- No timestamps.
- Adaptive CPU thread count.
- Hidden Windows child process flags.

### Ollama Cleanup

Native dictation is pinned to:

```text
gemma4:12b-it-qat
```

The native path calls:

```text
http://127.0.0.1:11434/api/generate
```

The cleanup prompt asks for strict JSON:

```json
{ "text": "final text", "confidence": 0.0, "resolved_corrections": [], "warnings": [] }
```

If the first response is invalid JSON, LocalFlow retries once with a repair prompt. If the repair fails or Ollama is unavailable, LocalFlow falls back to the raw Whisper transcript so dictation is not lost.

The app warms `gemma4:12b-it-qat` in the background when recording starts and keeps it alive longer for repeated dictations.

### Text Insertion

The current native insertion path:

1. Saves previous text clipboard content when possible.
2. Writes the final transcript to the clipboard.
3. Sends `Ctrl+V` through Win32 `SendInput`.
4. Waits briefly.
5. Restores previous text clipboard content.

UI Automation insertion, rich clipboard preservation, and target-window verification are still pending.

## Overlay Architecture

The floating overlay is a separate Tauri window:

```text
label: overlay
url: index.html?view=overlay
transparent: true
decorations: false
alwaysOnTop: true
skipTaskbar: true
focusable: false
```

The frontend entry point in `src/main.tsx` detects `?view=overlay` and renders only `VoiceOverlay`.

The overlay implementation lives in:

```text
src/components/VoiceOverlay.tsx
src/App.css
```

The current visual is a dark audio-ribbon canvas:

- The black surface is fixed-size and centered in the transparent overlay window.
- The canvas is absolutely positioned to fill the surface.
- The ribbon is drawn around an explicit vertical midpoint.
- Soft vertical clamping keeps the waves centered inside the GUI.
- Warmer upper strands react more to higher pitch.
- Cooler lower strands react more to lower pitch.
- Processing/refining/inserted/error states change the visual tone without showing a full app window.

Important limitation: the ribbon currently reacts to live acoustic features, not live partial transcript words. Word-synchronous animation requires the future rolling partial-ASR pipeline to emit stabilized partial transcript timing.

## React UI And Shared Domain Logic

Important frontend files:

- `src/App.tsx`: settings, status, diagnostics, personalization, mock workflow.
- `src/components/VoiceOverlay.tsx`: waveform-only overlay.
- `src/services/localflowClient.ts`: Tauri command adapter with browser fallback.
- `src/domain/workflow.ts`: TypeScript state machine.
- `src/domain/audio.ts`: bounded audio buffers, RMS, VAD, resampling helpers.
- `src/domain/asrWindows.ts`: rolling ASR window planner.
- `src/domain/whisperSidecar.ts`: Whisper command planning and JSON parsing.
- `src/domain/refinementPipeline.ts`: cleanup JSON contract, repair retry, fallback.
- `src/domain/ollama.ts`: local Ollama discovery and cleanup provider.
- `src/domain/networkPolicy.ts`: localhost-only provider enforcement.
- `src/domain/context.ts`: context privacy gates and app categorization.
- `src/domain/insertionPlan.ts`: insertion method ordering and duplicate guards.
- `src/domain/settings.ts`: personalization and style profile mutation helpers.
- `src/domain/commandMode.ts`: selected-text command planning.

## Tauri And Rust Modules

Important native files:

- `src-tauri/src/lib.rs`: Tauri builder, tracing setup, tray setup, command registration.
- `src-tauri/src/main.rs`: GUI subsystem entry point for Windows release builds.
- `src-tauri/src/hotkeys/mod.rs`: global shortcut registration and fallback.
- `src-tauri/src/native_dictation.rs`: production native dictation path.
- `src-tauri/src/workflow/mod.rs`: Rust workflow state machine.
- `src-tauri/src/storage/mod.rs`: SQLite initialization.
- `src-tauri/src/privacy/mod.rs`: log redaction helper.
- `src-tauri/src/asr/mod.rs`: ASR provider trait and config shapes.
- `src-tauri/src/refinement/mod.rs`: refinement provider trait and config shapes.
- `src-tauri/src/insertion/mod.rs`: text inserter trait.
- `src-tauri/src/context/mod.rs`: context provider trait.

## Runtime Assets

Development runtime assets live in `.localflow-runtime/` and are ignored by Git:

```text
.localflow-runtime/whisper/Release/whisper-cli.exe
.localflow-runtime/models/ggml-tiny.en-q5_1.bin
```

Tauri bundle config copies these assets into release resources:

```text
localflow-runtime/whisper/Release/whisper-cli.exe
localflow-runtime/whisper/Release/*.dll
localflow-runtime/models/ggml-tiny.en-q5_1.bin
```

## Build And Test Commands

Install/check prerequisites:

```powershell
.\scripts\Install-Prereqs.ps1 -Install
npm install
```

Run all checks:

```powershell
.\scripts\Run-Checks.ps1
```

Start dev mode:

```powershell
.\scripts\Start-Dev.ps1
```

Build release and installers:

```powershell
npm run tauri:build
```

Check Ollama:

```powershell
.\scripts\Check-Ollama.ps1
```

## Current Verification

Recent verification on this workstation:

- `npm run format`
- `npm run lint`
- `npm run test` with 77 frontend tests.
- `npm run build`
- `cargo fmt --check`
- `cargo test` with 14 Rust tests.
- `cargo check`
- `npm run tauri:build`

Known Rust warnings are from scaffolded provider traits and future-facing structs that are defined but not yet fully wired.

## Known Limitations

- Native hotkey dictation currently uses the default Windows input device.
- Native dictation cleanup is pinned to `gemma4:12b-it-qat`.
- Deterministic personalization is implemented in shared logic/UI but not yet wired into the production native hotkey path.
- UI Automation text insertion is pending.
- Target-window verification before insertion is pending.
- Rich clipboard preservation is pending.
- Startup-at-login is pending.
- Rolling partial transcription and live word-synchronous overlay animation are pending.
- Native SQLite persistence for all personalization/settings mutations is still incomplete.
- Manual insertion tests in Notepad, browser fields, and VS Code still need human confirmation.

## Recommended Next Work

1. Wire deterministic personalization into the native dictation pipeline before Ollama cleanup.
2. Add microphone selection and device-disconnect recovery.
3. Add target-window tracking before insertion.
4. Replace clipboard-only insertion with UI Automation where safe.
5. Add rolling partial Whisper windows and transcript stabilization to emit partial transcript events.
6. Use partial transcript events to animate the overlay by committed word timing.
7. Persist dictionary, replacements, snippets, and styles through native SQLite.
8. Add startup-at-login and an onboarding/diagnostics screen for microphone/model checks.

## Manual Test Checklist For The Current Build

1. Launch `src-tauri/target/release/localflow.exe`.
2. Open Notepad or another text field.
3. Tap `Ctrl+Alt+Space` or `Ctrl+Alt+Shift+Space`.
4. Confirm only the bottom waveform overlay appears.
5. Speak a low phrase and then a higher phrase.
6. Confirm the colored ribbon stays centered in the black panel and shifts shape with the voice.
7. Pause briefly.
8. Confirm the overlay switches to processing/refining.
9. Confirm final text is pasted into the focused field.
10. Confirm no terminal window flashes during transcription.
