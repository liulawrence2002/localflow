# Security

## Principles

- Keep dictated content local.
- Redact transcript text from logs by default.
- Treat selected text and cursor context as sensitive.
- Never execute arbitrary operating-system commands from command mode.
- Validate all local model responses before insertion.
- Protect against stale sessions inserting into the wrong window.

## Current Controls

- Native dictation assigns each recording a session id and runs the
  transcribe/refine/insert tail on a worker thread; a superseded or cancelled session
  is revalidated before every side effect and **never pastes** (`native_dictation.rs`
  `SessionRegistry`, `process_session`). Starting a new recording or a cancel command
  supersedes any in-flight worker.
- The mock/TS state machine prevents overlapping mock sessions (main-window verification
  path only; see `docs/REPO_AUDIT.md` §3 for which controls are native vs off-path).
- Mock pipeline never contacts a remote service.
- Shared network policy blocks remote model-provider URLs during ordinary dictation.
- Shared Ollama provider blocks remote URLs before fetch and validates JSON output before cleanup is accepted.
- Native cleanup calls only local Ollama at `127.0.0.1` with pinned model `gemma4:12b-it-qat`.
- Native `whisper.cpp` launch keeps model/audio/output paths and CLI arguments explicit and local.
- Shared context policy blocks active-app, selected-text, and cursor-context collection for password or protected fields.
- Command-mode planning rejects operating-system command execution phrases.
- Native insertion revalidates the target window before pasting: LocalFlow captures the
  foreground window (HWND + owning process id) when dictation starts and, immediately
  before `Ctrl+V`, aborts if the current foreground window is not the same one. It **fails
  closed** — if the original or current target cannot be confirmed, it does not paste
  (`native_dictation.rs` `TargetWindow`/`target_matches`).
- Insertion planning also rejects protected fields and changed targets in the shared TS
  `insertionPlan` layer (used by the mock/verification path).
- Duplicate insertion guard prevents repeating the same generated text for one session
  (shared TS layer; the native path now supersedes stale sessions via `SessionRegistry`).
- Tauri capabilities are limited to default window, opener, and global shortcut permissions.
- Logs use redaction helpers for dictated content.
- Diagnostics export excludes dictated content and local model paths by default.
- Native clipboard paste fallback restores prior text clipboard content after a short delay.
- The waveform overlay receives phase and audio-level metadata, not transcript text.

## Planned Controls

- Native UI Automation insertion.
- Native rich clipboard preservation, and stronger target verification (process start time
  to defeat PID reuse; UIA element identity; protected/read-only field detection).
- Paste-last-into-focus recovery via a dedicated hotkey (copy-last recovery already exists;
  the transcript is held in volatile memory only and never written to disk).
- Model sidecar health checks and bounded IPC.
- Diagnostics export with transcript exclusion by default.
- Retention enforcement jobs.
