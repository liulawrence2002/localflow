# Security

## Principles

- Keep dictated content local.
- Redact transcript text from logs by default.
- Treat selected text and cursor context as sensitive.
- Never execute arbitrary operating-system commands from command mode.
- Validate all local model responses before insertion.
- Protect against stale sessions inserting into the wrong window.

## Current Controls

- State machine prevents overlapping dictation sessions.
- Mock pipeline never contacts a remote service.
- Shared network policy blocks remote model-provider URLs during ordinary dictation.
- Shared Ollama provider blocks remote URLs before fetch and validates JSON output before cleanup is accepted.
- Native cleanup calls only local Ollama at `127.0.0.1` with pinned model `gemma4:12b-it-qat`.
- Native `whisper.cpp` launch keeps model/audio/output paths and CLI arguments explicit and local.
- Shared context policy blocks active-app, selected-text, and cursor-context collection for password or protected fields.
- Command-mode planning rejects operating-system command execution phrases.
- Insertion planning rejects protected fields and changed targets before insertion.
- Duplicate insertion guard prevents repeating the same generated text for one session.
- Tauri capabilities are limited to default window, opener, and global shortcut permissions.
- Logs use redaction helpers for dictated content.
- Diagnostics export excludes dictated content and local model paths by default.
- Native clipboard paste fallback restores prior text clipboard content after a short delay.
- The waveform overlay receives phase and audio-level metadata, not transcript text.

## Planned Controls

- Native UI Automation insertion.
- Native rich clipboard preservation and stronger target-window verification.
- Model sidecar health checks and bounded IPC.
- Diagnostics export with transcript exclusion by default.
- Retention enforcement jobs.
