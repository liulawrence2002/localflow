# LocalFlow Plan

## Wispr-class upgrade (spec-driven)

Phase 0 (evidence-based audit): completed. See `docs/REPO_AUDIT.md` and
`docs/PARITY_MATRIX.md` for current-state, discrepancies, risk-ranked debt, target
architecture, and the ASR/refinement benchmark plan. Baseline: 77 frontend tests,
14 Rust tests, all lint/format/type checks green.

Phase 1 (unify the production pipeline): in progress.

- **Slice 1 — session identity & cancellation (done).** Native `SessionRegistry` in
  `native_dictation.rs`; transcribe/refine/insert tail moved to a worker thread that
  revalidates the session before every side effect. A superseded or cancelled session
  never inserts (spec §4.4). +3 Rust tests.
- **Slice 2 — target-window revalidation (done).** Capture the foreground window
  (`TargetWindow { hwnd, pid }`) at record start; before pasting, skip insertion (fail
  closed) unless the same window is still focused (spec §6.1/§6.2). Addresses the top
  safety risk (wrong-window insertion). +2 Rust tests (19 total).
- **Slice 3 — copy/last-transcript recovery (done).** The last finalized transcript is
  kept in volatile memory and recoverable via a tray item and a Home > Recovery card
  (`get_last_transcript`/`copy_last_transcript`), so a skipped/failed insertion is never
  lost. Reveal-preview is opt-in; nothing is persisted. +3 frontend tests.
- **Slice 4 — deterministic smart formatting (done).** New authoritative Rust `transcript`
  module: spoken punctuation, explicit self-corrections, filler/stutter cleanup, sentence
  capitalization, URL/email/decimal protection. Applied before the LLM (seeds the cleanup
  prompt) and used as the safe fallback when the model is unavailable. +10 Rust tests.
- **Slice 5 — personalization + model config on the native path (done).** The native path
  now reads the user's settings: exact replacements + snippets are applied deterministically,
  dictionary terms bias whisper via its initial prompt, and the Ollama model is configurable
  (no longer hardcoded). +3 Rust tests.
- **Slice 6 — Escape-to-cancel (done).** Escape is registered only while recording and
  cancels the active dictation (routed to the existing `cancel` command); nothing inserts.
- **Slice 7 — honest diagnostics + labeled test controls (done).** Diagnostics describe the
  real native pipeline; the Home simulated-test panel is clearly labeled as a test, not
  production dictation.
- **Slice 8 — streaming ASR foundation (done, not yet on the default path).** New
  `src-tauri/src/asr` submodules: a typed `AsrEvent` contract + `StreamingAsrProvider` trait
  (spec §3.2/§4.3), an overlap-dedup `TranscriptStabilizer` and `StreamingSession` coordinator
  that never duplicate committed words, a rolling-window planner, and a `word_error_rate`
  metric for the benchmark harness. All unit-tested (+15 Rust tests, 47 total). This is the
  Phase 3 foundation; wiring a persistent whisper provider onto it + the benchmark harness is
  next (requires local models to verify).
- Follow-ups (roadmap): persistent/streaming whisper provider + benchmark harness; native
  SQLite persistence + retention jobs; UI Automation insertion; native context capture;
  paste-last-into-focus hotkey; module split of `native_dictation.rs`.

## Milestone 1: Foundation

Status: completed.

Completed:

- Created a Tauri 2 + React + TypeScript project.
- Added root documentation, agent instructions, prompt contract, and Windows scripts.
- Added explicit dictation workflow state machine in TypeScript and Rust.
- Added mock ASR, mock refinement, mock insertion, and provider traits.
- Added settings UI screens for home, models, microphone, hotkeys, dictionary, replacements, snippets, styles, privacy, history, diagnostics, and about.
- Added SQLite schema initialization for settings, history, and dictionary data.
- Added Tauri tray setup and global-hotkey plugin registration for `Ctrl+Alt+Space`.
- Added unit tests for state transitions, transcript stabilization, deterministic personalization, insertion spacing, and LLM JSON validation.
- Added session-id validation for result-bearing workflow events to reject stale ASR/refinement/insertion results.
- Added build-tested audio helpers for bounded PCM ring buffers, RMS calculation, mono downmixing, linear resampling, and VAD/end-of-speech detection.
- Added rolling ASR window planning for overlap-based incremental transcription.
- Added shared `whisper.cpp` sidecar command planning, vocabulary prompt construction, and JSON transcript parsing.
- Added local cleanup JSON contract runner with one repair attempt and deterministic fallback that preserves the raw transcript.
- Added privacy retention helpers for disabled history, transcript-only history, delete-after windows, context retention, and redacted diagnostics.
- Added diagnostics export generation that excludes dictated content and local model paths by default.
- Added shared context snapshot policy with protected-field blocking, context limits, cleanup mapping, and application category classification.
- Added shared performance recorder and diagnostics formatter for latency and peak-memory metrics without invented values.
- Added timeout guard for local providers.
- Added editable dictionary, replacement, snippet, and style-profile controls in the settings UI.
- Added settings mutation helpers for personalization CRUD and browser fallback persistence.
- Extended style profiles with greeting, sign-off, filler-removal, and sentence-fragment behavior fields.
- Added command-mode planning with selected-text requirement, OS-command rejection, preview decisions, strict JSON parsing, and undo text.
- Added insertion planning helpers for target validation, ordered accessibility/keyboard/clipboard fallback, delayed clipboard restoration, and duplicate insertion rejection.
- Added “Undo AI cleanup” UI action for the mock workflow output.
- Added localhost-only dictation network policy checks.
- Added a tested Ollama refinement provider that discovers local models, blocks remote URLs, sends non-streaming JSON generate requests, and reports unavailable or missing-model errors clearly.
- Added Models screen Ollama discovery and local model selection for the browser/dev UI path.
- Installed Rust stable through rustup and Microsoft Visual Studio C++ Build Tools for native Windows builds on this workstation.
- Added a first native push-to-talk path behind the Tauri global hotkey: default microphone capture, local `whisper.cpp` transcription, and clipboard paste insertion.
- Added a fallback global hotkey, `Ctrl+Alt+Shift+Space`, when `Ctrl+Alt+Space` is already registered.
- Added bundled-resource lookup for the local Whisper CLI, DLLs, and tiny English model, with `.localflow-runtime/` as the dev fallback.
- Added native local Ollama cleanup pinned to model `gemma4:12b-it-qat`, with strict JSON parsing, one repair request, and raw-transcript fallback if cleanup fails.
- Added a hidden-on-start settings window and a compact always-on-top waveform overlay for active dictation.
- Added native microphone level events so the overlay reacts to speech volume while listening.
- Added native end-of-speech auto-stop so dictation starts processing after a short post-speech pause, while still allowing hotkey release to finish immediately.
- Added hybrid tap/hold hotkey handling: quick releases now keep recording open for tap-to-start dictation, while a second tap or longer hold release can stop manually.
- Added no-speech timeout after tap-to-start so the overlay does not stay open forever when no voice is detected.
- Added native `gemma4:12b-it-qat` warmup on recording start and longer Ollama keep-alive to reduce cleanup wait on repeated dictations.
- Refined the floating overlay into a wider polished waveform with live bars, generated wave paths, processing sheen, and success/error state colors.
- Hid the `whisper-cli.exe` sidecar process window on Windows so hotkey dictation only surfaces the waveform overlay.
- Added live pitch and brightness extraction from microphone chunks and replaced the overlay with a dark, layered audio-ribbon canvas: higher pitch lifts warmer upper strands and lower pitch deepens cooler lower strands.

Verified:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test` with 77 passing tests.
- `npm run build`
- `cd src-tauri; cargo fmt --check`
- `cd src-tauri; cargo test` with 14 passing tests.
- `cd src-tauri; cargo check`
- `npm run tauri:build`, producing:
  - `src-tauri\target\release\localflow.exe`
  - `src-tauri\target\release\bundle\msi\LocalFlow_0.1.0_x64_en-US.msi`
  - `src-tauri\target\release\bundle\nsis\LocalFlow_0.1.0_x64-setup.exe`
- Vite dev server at `http://127.0.0.1:1420/`
- Live dev-server smoke check: page status 200, root element present, transformed `App.tsx` contains LocalFlow Home, Privacy, Diagnostics, mock transcript UI markers, editable personalization UI markers, Undo cleanup marker, Ollama check markers, and command-mode module task marker.
- `npm run tauri -- info` after prerequisite installation.
- `.\scripts\Check-Ollama.ps1`, confirming local model `gemma4:12b-it-qat`.
- Direct local Ollama generate smoke test with `gemma4:12b-it-qat`, returning strict cleanup JSON.

Known native limitations:

- Manual microphone dictation and text insertion must still be exercised by a human in Notepad, a browser field, and VS Code.
- Native dictation now runs local Ollama `gemma4:12b-it-qat` cleanup before insertion; deterministic personalization is still not wired into the native hotkey path.
- Clipboard fallback restores prior text clipboard content, but not rich clipboard formats.
- UI Automation insertion, target-window verification, and startup-at-login are still pending.

Re-run:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test`
- `npm run build`
- `cd src-tauri; cargo fmt --check; cargo test; cargo check`
- `npm run tauri:dev`

Risks:

- Rust toolchain and Windows C++ build tools may be missing on other developer machines; `scripts/Install-Prereqs.ps1 -Install` now covers rustup and MSVC Build Tools.
- Tauri plugin API drift can break native compilation; keep `npm run tauri -- info` and Rust checks in the verification loop.
- The first native path proves local recording/transcription/paste plumbing, but not final latency, cleanup quality, target-window safety, or broad app compatibility.

Acceptance criteria for this milestone:

- Frontend builds and tests pass.
- Native code is structured behind small interfaces and ready for compilation once prerequisites are installed.
- UI can exercise a mock dictation session end to end without remote calls.

## Milestone 2: Audio and Local ASR

Status: partial, shared logic verified and first native local-ASR path runnable.

Completed:

- Bounded audio ring buffer.
- Mono downmixing and sample-rate normalization helper.
- RMS-based VAD/end-of-speech detector.
- Rolling-window planner with overlap.
- Shared `whisper.cpp` sidecar invocation planner using model, audio, thread, language, prompt, JSON-output, and CPU/GPU flags.
- Shared `whisper.cpp` JSON transcript parser with segment timestamps.
- Initial prompt builder for dictionary terms and pronunciation hints.
- Timeout guard for local provider calls.
- Shared performance recorder for hotkey-to-recording, ASR partial, release-to-final, LLM, insertion, model-load, and peak-memory metrics.
- Native `cpal` default-microphone capture on a dedicated recorder thread.
- Multi-channel input downmixing selects the loudest active channel to avoid phase-cancellation silence.
- Native end-of-speech detection stops recording after speech is heard and a short silence follows.
- Quick hotkey taps now start dictation without closing the overlay on key release.
- No-speech timeout ends abandoned tap-to-start sessions.
- Native recording has a maximum-duration cap for the current hotkey path.
- Capture diagnostics compute duration, peak, RMS, and nonzero sample ratio before Whisper runs.
- Temporary mono 16 kHz WAV writing for the current sidecar path, with cleanup after transcription.
- Local `whisper-cli.exe` launch against `ggml-tiny.en-q5_1.bin` with JSON output parsing, no timestamps, and adaptive CPU thread count.
- Hidden Windows sidecar launch prevents terminal flashes during transcription.
- Near-silence captures and blank Whisper markers are rejected with clearer errors.
- Clear missing-runtime and missing-model errors for the native path.
- Live microphone level, pitch, and brightness sampling for the floating waveform overlay.
- Polished floating overlay shows pitch-reactive colored ribbon waves and distinct processing, inserted, and error states without opening the full settings UI.

Not yet completed:

- Replace the temporary WAV path with a longer-term sidecar process manager and narrower IPC boundary.
- Add cancellation, timeout enforcement, sidecar crash recovery, and native latency metric feeds.
- Wire rolling partial transcription into real ASR events.
- Drive overlay motion from stabilized partial transcript timing once live partial ASR is wired; the current overlay reacts to live acoustic features.
- Add microphone selection and device-disconnect recovery.

## Milestone 3: Insertion and Cleanup

Status: partial, shared cleanup contract verified and native clipboard insertion started.

Completed:

- Strict local-cleanup JSON validation.
- One repair attempt for invalid cleanup responses.
- Deterministic fallback that preserves raw transcript.
- Timeout guard for provider calls.
- Undo AI cleanup helper and UI action for restoring deterministic/raw text.
- Insertion target validation, method ordering, clipboard restoration plan, and duplicate insertion guard.
- Local Ollama model discovery through `/api/tags`.
- Local Ollama cleanup requests through `/api/generate` with `stream: false` and strict JSON-format output.
- Native Ollama cleanup requests pinned to `gemma4:12b-it-qat`.
- Native recording start warms pinned `gemma4:12b-it-qat` in the background and keeps it loaded longer for repeated dictation.
- Remote Ollama URLs blocked before fetch.
- Clear shared errors for unavailable Ollama, no selected model, and missing local model.
- Native clipboard paste fallback using Win32 `SendInput` for `Ctrl+V`.
- Delayed restoration of the previous text clipboard after paste.

Not yet completed:

- Add Windows target tracking.
- Add UI Automation insertion where safe.
- Preserve rich clipboard formats during native fallback.
- Wire deterministic personalization into the production native dictation workflow.

## Milestone 4: Personalization

Status: partial, editable UI and shared mutations verified.

Completed:

- Editable dictionary entries.
- Editable replacements.
- Editable exact snippets.
- Editable style profiles with conciseness, formality, contractions, emoji, paragraph length, bullet preference, greeting/sign-off behavior, filler removal, and sentence-fragment controls.
- Shared settings mutation helpers with tests.
- Shared context snapshot policy for optional active-app, accessibility text, and selected-text collection.
- Shared application-category classifier for messaging, email, documents, code editors, terminals, search fields, and generic text fields.

Not yet completed:

- Persist dictionary, replacements, snippets, and style profiles through native SQLite across desktop restarts.
- Inject relevant vocabulary into ASR prompts.
- Wire native Windows context capture into the shared context policy.

## Milestone 5: Advanced Workflow

Status: partial, command-mode planning verified.

Completed:

- Command mode requires selected text.
- Command mode rejects operating-system command execution.
- Command mode classifies common editing intents and chooses preview for large selected text.
- Command mode preserves undo text.

Not yet completed:

- Add optional live insertion.
- Wire command mode to native selected-text capture, preview UI, local model execution, and insertion.
- Add native diagnostics file export if needed, broader privacy controls, and retention enforcement.

## Milestone 6: Packaging and Hardening

- Tauri bundle config now includes the Whisper CLI, required DLLs, and tiny English model as app resources.
- Build installer.
- Add startup-at-login.
- Add crash recovery, benchmarks, security review, and full manual acceptance testing.
