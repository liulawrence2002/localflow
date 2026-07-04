# Architecture

LocalFlow is split into a Tauri native layer and a React settings/history interface.

## Native Layer

- `src-tauri/src/workflow`: explicit dictation state machine. **Note:** this reducer is
  currently exercised only by the mock-session Tauri commands, not by the live hotkey path.
  See `REPO_AUDIT.md` §2 for the three current session representations and the plan to
  unify them.
- `src-tauri/src/audio`: microphone capture interface, mock capture, and shared audio helpers.
- `src-tauri/src/asr`: ASR provider trait and initial mock provider; `WhisperCppProviderConfig` defines the sidecar boundary.
- `src-tauri/src/refinement`: refinement provider trait, mock provider, no-op provider, and future native Ollama/llama.cpp configs.
- `src-tauri/src/context`: local context snapshot interface.
- `src-tauri/src/insertion`: text insertion interface and mock insertion.
- `src-tauri/src/hotkeys`: global hotkey registration through Tauri's global-shortcut plugin.
- `src-tauri/src/native_dictation`: the live Windows native push-to-talk path using `cpal`, local `whisper-cli.exe`, a configurable local Ollama cleanup model (default `llama3.2:3b`), a waveform overlay event stream, and clipboard paste fallback. Carries a native `SessionRegistry` (session identity + supersede/cancel guard); the transcribe/refine/insert tail runs on a worker thread and revalidates the session before every side effect so a superseded or cancelled session never inserts.
- `src-tauri/src/transcript`: **authoritative** deterministic formatting for the native path — spoken punctuation, explicit self-corrections, filler/stutter cleanup, sentence capitalization, user replacements/snippets, and URL/email/decimal protection. Runs before the LLM and is the fallback when the LLM is unavailable.
- `src-tauri/src/storage`: SQLite schema initialization.
- `src-tauri/src/privacy`: redaction helpers for logs and diagnostics.

The native dictation pipeline is: capture (`cpal`) → deterministic formatting
(`transcript`) seeded from the user's settings (replacements, snippets, dictionary → whisper
`--prompt`) → local Ollama cleanup with the configured model → session + target-window
revalidation → clipboard paste. Each recording carries a session id; the transcribe/refine/
insert tail runs on a worker thread that never inserts for a superseded, cancelled
(Escape), or focus-changed session, and the last transcript is recoverable.

The workflow is state-driven:

`Idle -> Preparing -> Listening -> Transcribing -> Refining -> Inserting -> Complete`

Cancellation and errors are terminal states until reset. Overlapping sessions are rejected with immediate warning state.

## Frontend Layer

- `src/domain`: shared state machine, deterministic cleanup, transcript stabilization, insertion spacing, and response validation.
- `src/domain/audio.ts`: bounded PCM buffers, RMS/VAD, mono downmixing, and linear resampling helpers.
- `src/domain/asrWindows.ts`: rolling-window planning for overlapped ASR hypotheses.
- `src/domain/whisperSidecar.ts`: shared `whisper.cpp` command planning, vocabulary prompt building, and JSON transcript parsing.
- `src/domain/refinementPipeline.ts`: strict local-LLM JSON contract runner with repair retry and deterministic fallback.
- `src/domain/context.ts`: context snapshot privacy policy, application categorization, and cleanup input mapping.
- `src/domain/privacy.ts`: history retention and redaction helpers.
- `src/domain/diagnostics.ts`: redacted diagnostics export assembly for support bundles.
- `src/domain/performance.ts`: latency and peak-memory measurement helpers with diagnostic formatting.
- `src/domain/settings.ts`: typed personalization and style-profile mutations used by the UI and local fallback.
- `src/domain/commandMode.ts`: selected-text command planning, OS-command rejection, preview decisions, and command response parsing.
- `src/domain/insertionPlan.ts`: target validation, insertion method ordering, clipboard fallback restoration timing, and duplicate insertion guards.
- `src/domain/undo.ts`: deterministic/raw restore helpers for undoing cleanup.
- `src/domain/networkPolicy.ts`: local-only network allowlist for ordinary dictation provider URLs.
- `src/domain/ollama.ts`: shared local Ollama discovery and cleanup provider with localhost-only enforcement.
- `src/services/localflowClient.ts`: Tauri command adapter with a browser fallback for frontend development.
- `src/components`: focused UI components.
- `src/components/VoiceOverlay.tsx`: compact waveform-only overlay rendered in the Tauri `overlay` window.
- `src/App.tsx`: settings, status, history, diagnostics, and mock workflow controls.

## Provider Boundary

Providers are intentionally narrow:

- ASR accepts local audio/model configuration and returns transcript events.
- Refinement accepts compact dictation-cleanup inputs and returns strict JSON.
- Context providers never return password/protected-field content.
- Inserters must verify the intended target before writing text.

## Persistence

SQLite is initialized under the Tauri app data directory. Milestone 1 creates tables for settings, dictation history, and dictionary entries. Later milestones will add migrations and retention jobs.
