# Privacy

LocalFlow is local-first.

Default behavior:

- No account.
- No analytics.
- No telemetry.
- No cloud sync.
- No remote ASR or LLM calls.
- No screenshot capture.
- No permanent audio storage.

The current native hotkey path writes a temporary WAV file under the OS temp directory for `whisper.cpp` CLI processing, then deletes the WAV and JSON output after transcription.

The floating overlay receives listening/processing/refining/inserted/error state and microphone level values. It does not receive or display transcript text.

## Stored Fields

Milestone 1 initializes SQLite tables for:

- `settings`: key, JSON value, update timestamp.
- `dictation_history`: id, creation timestamp, target application, raw transcript, final text, cleanup level.
- `dictionary_entries`: phrase, pronunciation hint, category, capitalization setting.

History storage is controlled by retention settings. Later milestones must enforce:

- Do not store history.
- Store transcript only.
- Store original and cleaned transcript.
- Delete after 24 hours.
- Delete after 7 days.
- Keep until manually deleted.

The shared domain layer includes build-tested retention helpers for disabled history, transcript-only history, original-and-cleaned history, and delete-after windows. Native SQLite writes still need to use those helpers before production dictation is enabled.

During browser-only development, settings edits use local browser storage as a fallback when Tauri commands are unavailable. The production desktop path is expected to persist settings through the native local storage layer.

## Context

Context awareness is optional and narrowly scoped. LocalFlow may read:

- Active app name.
- Window title.
- App category.
- Selected text.
- Limited text before and after the cursor.
- Sentence-start and code-field signals.

LocalFlow must never collect context from password or protected fields.

The shared context policy enforces separate gates for active-app metadata, accessibility text around the cursor, and selected text. It also trims before-cursor, after-cursor, and selected-text context to bounded lengths before those values can be sent to cleanup logic.

## Network Connections

Allowed:

- Localhost calls to Ollama.
- Localhost or file/process IPC for `whisper.cpp` sidecars.

Not allowed during ordinary dictation:

- Cloud ASR.
- Cloud LLMs.
- Telemetry.
- Analytics.
- Silent remote fallback.

Automated checks cover the shared network policy and the shared Ollama provider. Remote Ollama URLs are rejected before `fetch` is called.

The shared domain layer includes an allowlist check that permits localhost provider URLs and rejects remote URLs for ordinary dictation. The Ollama provider calls this policy for model discovery and cleanup requests. Native providers and future local server providers must keep using the same policy before any production network-capable path is enabled.

Native dictation cleanup is pinned to local Ollama model `gemma4:12b-it-qat` at `http://127.0.0.1:11434/api/generate`.

## Diagnostics

The shared diagnostics export excludes dictated transcript text and local model paths by default. A transcript-inclusive export path exists only as an explicit option for user-approved troubleshooting.
