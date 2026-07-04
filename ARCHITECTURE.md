# Architecture

LocalFlow is split into a Tauri native layer and a React settings/history interface.

## Native Layer

- `src-tauri/src/workflow`: explicit dictation state machine.
- `src-tauri/src/audio`: microphone capture interface and mock capture.
- `src-tauri/src/asr`: ASR provider trait and initial mock provider; `WhisperCppProviderConfig` defines the sidecar boundary.
- `src-tauri/src/refinement`: refinement provider trait, mock provider, no-op provider, and future Ollama/llama.cpp configs.
- `src-tauri/src/context`: local context snapshot interface.
- `src-tauri/src/insertion`: text insertion interface and mock insertion.
- `src-tauri/src/hotkeys`: global hotkey registration through Tauri's global-shortcut plugin.
- `src-tauri/src/storage`: SQLite schema initialization.
- `src-tauri/src/privacy`: redaction helpers for logs and diagnostics.

The workflow is state-driven:

`Idle -> Preparing -> Listening -> Transcribing -> Refining -> Inserting -> Complete`

Cancellation and errors are terminal states until reset. Overlapping sessions are rejected with immediate warning state.

## Frontend Layer

- `src/domain`: shared state machine, deterministic cleanup, transcript stabilization, insertion spacing, and response validation.
- `src/services/localflowClient.ts`: Tauri command adapter with a browser fallback for frontend development.
- `src/components`: focused UI components.
- `src/App.tsx`: settings, status, history, diagnostics, and mock workflow controls.

## Provider Boundary

Providers are intentionally narrow:

- ASR accepts local audio/model configuration and returns transcript events.
- Refinement accepts compact dictation-cleanup inputs and returns strict JSON.
- Context providers never return password/protected-field content.
- Inserters must verify the intended target before writing text.

## Persistence

SQLite is initialized under the Tauri app data directory. Milestone 1 creates tables for settings, dictation history, and dictionary entries. Later milestones will add migrations and retention jobs.
