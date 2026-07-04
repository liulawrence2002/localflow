# LocalFlow Handoff

## Goal

LocalFlow is a local-first Windows desktop dictation app. The intended user flow is:

1. The app runs quietly in the system tray.
2. The user focuses any text field in another app.
3. The user taps the global hotkey.
4. A small bottom waveform overlay appears.
5. LocalFlow records microphone audio.
6. Local `whisper.cpp` transcribes the audio.
7. A configurable local Ollama model (default `llama3.2:3b`) cleans the transcript — or, in low-resource mode, the deterministically formatted text is inserted directly with no LLM.
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
- `npm run test` with 80 frontend tests (77 baseline + 3 recovery tests, Slice 3).
- `npm run build`
- `cargo fmt --check`
- `cargo test` with 48 Rust tests (14 baseline; +3 session-guard Slice 1; +2 target-window Slice 2; +10 deterministic-formatting Slice 4; +3 personalization Slice 5; +15 streaming-ASR foundation Slice 8; +1 no-speech-branch Slice 9).
- `cargo check`
- `npm run tauri:build`

Known Rust warnings (14) are from scaffolded provider traits and future-facing structs that are defined but not yet fully wired.

### Phase 0 audit + Slice 1 (session identity & cancellation)

- `docs/REPO_AUDIT.md` and `docs/PARITY_MATRIX.md` capture the evidence-based current
  state, doc-vs-code discrepancies, risk-ranked debt, target architecture, and the ASR
  benchmark plan.
- Slice 1 adds a native `SessionRegistry` in `native_dictation.rs`. Each recording gets a
  session id; the transcribe -> refine -> insert tail now runs on a worker thread. The
  worker revalidates its session id before whisper, before refinement, and immediately
  before pasting, so a **superseded (new recording started) or cancelled session never
  inserts text** (spec §4.4). A `"cancel"` recorder command invalidates the active session
  (mechanism for Escape-to-cancel; the Escape hotkey itself is a later slice).
- Not yet runtime-verifiable here (no mic/whisper/Ollama in this environment). Manual
  validation: (1) start dictation, immediately start a second dictation before the first
  finishes → only the second inserts; (2) once a `cancel` hotkey is wired, cancel mid-
  processing → nothing inserts.

### Slice 2 (target-window revalidation before insertion)

- `start_recording` captures the foreground window (`TargetWindow { hwnd, pid }`) via
  `GetForegroundWindow` + `GetWindowThreadProcessId`. Before the `Ctrl+V` paste,
  `process_session` calls `target_matches(captured, foreground_target())` and **skips the
  paste** (with a clear "focus changed" error) unless the same window is still focused.
  Fails closed if either target is unknown (spec §6.2 — never insert into a target that
  cannot be revalidated). Added `Win32_Foundation` + `Win32_UI_WindowsAndMessaging`
  features to the `windows` crate. +2 Rust tests.
- Manual validation: start dictation in Notepad, alt-tab to another app before it finishes
  → transcript is NOT pasted and the overlay shows the focus-changed error; staying in
  Notepad → normal paste.

### Slice 3 (copy/last-transcript recovery)

- `NativeDictationRuntime` now keeps the last finalized transcript in volatile memory
  (`last_transcript`), stored just before the insertion attempt so it survives a skipped
  paste. Nothing is written to disk (retention-safe).
- New Tauri commands `get_last_transcript` and `copy_last_transcript`, a tray item
  "Copy last transcript" (needs no focus, always safe), and a Home > Recovery card that
  shows availability + char count, a Copy button, and an opt-in Reveal preview (the
  transcript is not shown by default). Pure helper `src/domain/recovery.ts` + 3 tests.
- Manual validation: after a dictation whose paste was skipped (focus changed), click the
  tray "Copy last transcript" (or the Recovery card's Copy) → transcript is on the
  clipboard for manual Ctrl+V.

### Slices 4-7 (smart formatting, personalization, escape-to-cancel, honesty)

- **Slice 4 — `src-tauri/src/transcript/mod.rs`.** Authoritative deterministic formatting:
  spoken punctuation ("comma"/"new line"/"bullet point"...), explicit self-corrections
  ("actually", "no", "sorry", "let me restart"), filler/stutter removal, sentence
  capitalization, and URL/email/decimal protection. Runs before the LLM and is the fallback
  when Ollama is down. The TS `personalization.ts` is now the browser-dev-only copy.
- **Slice 5 — personalization + model config.** `process_session` reads
  `LocalFlowRuntime::current_settings()`: enabled replacements + snippets are applied by
  `apply_deterministic_formatting_with`; dictionary phrases seed whisper's `--prompt`
  (bounded to 800 chars); the Ollama model comes from settings (falls back to the default
  constant). Warm-up uses the configured model too.
- **Slice 6 — Escape-to-cancel.** `set_escape_cancel` registers bare Escape only while
  recording; the hotkey handler routes it to the `cancel` command. It is unregistered when
  recording ends or is cancelled, so Escape is not suppressed system-wide otherwise. Manual
  validation: start dictation, press Escape while speaking → nothing inserts, overlay hides.
- **Slice 7 — honesty.** Diagnostics describe the real pipeline and flag the one-shot ASR as
  a known limitation; the Home panel is labeled "Simulated test" with a note that real
  dictation uses the hotkey.

### Slice 8 (streaming ASR foundation — `src-tauri/src/asr/`)

- `stabilizer.rs` — `TranscriptStabilizer`: commits the longest stable token prefix across
  overlapping-window hypotheses so committed words are never duplicated or rewritten
  (spec §3.2). Authoritative Rust port of `transcriptStabilizer.ts`.
- `windows.rs` — `plan_rolling_windows`: rolling/overlapping window boundaries for a
  persistent runtime. Port of `asrWindows.ts`.
- `streaming.rs` — typed `AsrEvent` contract, `AsrCapabilities`, `StreamingAsrProvider`
  trait, and a `StreamingSession` coordinator turning rolling-window decodes into
  SessionStarted/SpeechStarted/Committed/Partial/SpeechEnded/Final events. A scripted
  `MockStreamingProvider` proves the trait end to end in tests.
- `metrics.rs` — `word_error_rate` (token Levenshtein), the accuracy measure for the
  benchmark harness.
- **Status:** these are the Phase 3 foundation, exercised by unit tests but not yet on the
  default one-shot path (marked `#![allow(dead_code)]`). Next: implement a persistent
  whisper provider behind `StreamingAsrProvider`, feed rolling windows through
  `StreamingSession`, emit partials to the overlay, and build the benchmark harness. That
  step needs local models + a mic to verify, which this environment lacks.

### Slice 9 (snappier, more Wispr Flow-like feel)

- Timing constants tightened in `native_dictation.rs`: `END_OF_SPEECH_TIMEOUT_MS`
  760→550, `NO_SPEECH_TIMEOUT_MS` 6000→2500, `MIN_AUTO_STOP_RECORDING_MS` 420→350;
  `schedule_overlay_hide` 1200→700 ms; `paste_text` clipboard-restore sleep 700→400 ms.
- **Silent idle close:** when speech was never detected, the level meter now sends `cancel`
  (overlay just hides) instead of `auto_stop` (which flashed a "too short" error). New test
  `no_speech_timeout_is_distinguishable_from_end_of_speech`.
- **Faster default model:** default cleanup model changed from `gemma4:12b-it-qat` (12B) to
  `llama3.2:3b` (fast, small, same Llama family Wispr Flow fine-tunes for its cleanup; our
  deterministic layer already does the heavy formatting). Still configurable; `qwen2.5:3b`
  and the original 12B are documented alternatives. Cleanup HTTP timeout 60→20 s so a slow
  model falls back to deterministic text quickly.
- **Instant mode:** the previously-unused `low_resource_mode` setting now skips the LLM and
  inserts the deterministically formatted text for the lowest latency.
- Insertion safety (session guard, target revalidation, Escape, recovery) is unchanged.
- Manual validation needs a mic + `ollama pull llama3.2:3b`; steps in the plan file.

## Known Limitations

- Native hotkey dictation currently uses the default Windows input device.
- Native dictation cleanup model is configurable via settings (default `llama3.2:3b`, a fast
  small model; low-resource mode skips the LLM entirely). ASR language is still fixed to
  English and the whisper model path/threads are not yet settings-driven.
- Deterministic formatting + user replacements/snippets/dictionary now run on the native
  hotkey path (Slices 4-5). Style profiles are not yet applied to the cleanup prompt.
- UI Automation text insertion is pending (insertion is still clipboard + Ctrl+V).
- Target-window verification before insertion is implemented (HWND + PID, fail-closed);
  hardening (process start time, UIA element identity, protected-field detection) is pending.
- When the target changes, the transcript is preserved for recovery: "Copy last transcript"
  is available from the tray and the Home > Recovery card. Paste-last-into-focus via a
  dedicated global hotkey is still pending.
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
