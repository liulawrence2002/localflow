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

Verified:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test` with 11 passing tests.
- `npm run build`
- Vite dev server at `http://127.0.0.1:1420/`
- Live dev-server smoke check: page status 200, root element present, module script present, transformed `App.tsx` contains LocalFlow Home, Privacy, Diagnostics, and mock transcript UI markers.

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

- Add `cpal` microphone capture.
- Add VAD and bounded ring buffers.
- Add `whisper.cpp` sidecar process manager.
- Add final transcription with cancellation, timeout, model-not-found errors, and latency metrics.
- Keep rolling partial transcription in a tested stabilizer.

## Milestone 3: Insertion and Cleanup

- Add Windows target tracking.
- Add UI Automation insertion where safe.
- Add simulated keyboard and clipboard fallback.
- Add Ollama provider, JSON repair retry, and undo cleanup.

## Milestone 4: Personalization

- Persist dictionary, replacements, snippets, and style profiles.
- Inject relevant vocabulary into ASR prompts.
- Add context-aware formatting categories.

## Milestone 5: Advanced Workflow

- Add optional live insertion.
- Add command mode with selected-text preview and undo.
- Add history, diagnostics export, privacy controls, and retention enforcement.

## Milestone 6: Packaging and Hardening

- Build installer.
- Add startup-at-login.
- Add crash recovery, benchmarks, security review, and full manual acceptance testing.
