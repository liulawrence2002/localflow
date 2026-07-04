# LocalFlow Plan

## Milestone 1: Foundation

Status: completed as far as the current environment permits.

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

Verified:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test` with 70 passing tests.
- `npm run build`
- Vite dev server at `http://127.0.0.1:1420/`
- Live dev-server smoke check: page status 200, root element present, transformed `App.tsx` contains LocalFlow Home, Privacy, Diagnostics, mock transcript UI markers, editable personalization UI markers, Undo cleanup marker, Ollama check markers, and command-mode module task marker.

Blocked:

- Native `npm run tauri:dev` and Rust checks, because `cargo`, `rustc`, `rustup`, and MSVC Build Tools are not installed in the current environment.
- `npm run tauri -- info` verified WebView2 is present and Tauri CLI is available, but native prerequisites are missing.

Re-run after prerequisites:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test`
- `npm run build`
- `cd src-tauri; cargo fmt --check; cargo test; cargo check`
- `npm run tauri:dev`

Risks:

- Rust toolchain and Windows C++ build tools may be missing on developer machines.
- Tauri plugin API drift can break native compilation; verify after installing Rust.
- Mock providers prove orchestration but not real latency, audio device, sidecar, or insertion behavior.

Acceptance criteria for this milestone:

- Frontend builds and tests pass.
- Native code is structured behind small interfaces and ready for compilation once prerequisites are installed.
- UI can exercise a mock dictation session end to end without remote calls.

## Milestone 2: Audio and Local ASR

Status: partial, shared logic verified.

Completed:

- Bounded audio ring buffer.
- Mono downmixing and sample-rate normalization helper.
- RMS-based VAD/end-of-speech detector.
- Rolling-window planner with overlap.
- Shared `whisper.cpp` sidecar invocation planner using model, audio, thread, language, prompt, JSON-output, and CPU/GPU flags.
- Shared `whisper.cpp` JSON transcript parser with segment timestamps.
- Initial prompt builder for dictionary terms and pronunciation hints.
- Timeout guard for local provider calls.

Not yet completed:

- Add `cpal` microphone capture.
- Add `whisper.cpp` sidecar process manager.
- Add final transcription with cancellation, timeout, model-not-found errors, and latency metrics.
- Wire rolling partial transcription into real ASR events.

## Milestone 3: Insertion and Cleanup

Status: partial, shared cleanup contract verified.

Completed:

- Strict local-cleanup JSON validation.
- One repair attempt for invalid cleanup responses.
- Deterministic fallback that preserves raw transcript.
- Timeout guard for provider calls.
- Undo AI cleanup helper and UI action for restoring deterministic/raw text.
- Insertion target validation, method ordering, clipboard restoration plan, and duplicate insertion guard.
- Local Ollama model discovery through `/api/tags`.
- Local Ollama cleanup requests through `/api/generate` with `stream: false` and strict JSON-format output.
- Remote Ollama URLs blocked before fetch.
- Clear shared errors for unavailable Ollama, no selected model, and missing local model.

Not yet completed:

- Add Windows target tracking.
- Add UI Automation insertion where safe.
- Add simulated keyboard and clipboard fallback.
- Wire the Ollama provider into the production native dictation workflow after real ASR and insertion are available.

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

- Build installer.
- Add startup-at-login.
- Add crash recovery, benchmarks, security review, and full manual acceptance testing.
