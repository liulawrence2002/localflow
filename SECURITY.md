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
- Tauri capabilities are limited to default window, opener, and global shortcut permissions.
- Logs use redaction helpers for dictated content.

## Planned Controls

- UI Automation target verification before insertion.
- Clipboard preservation and delayed restoration.
- Model sidecar health checks and bounded IPC.
- Strict JSON validation and repair retry for local LLM output.
- Diagnostics export with transcript exclusion by default.
- Retention enforcement jobs.
