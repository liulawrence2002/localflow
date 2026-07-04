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

Automated network-denial checks are planned before real dictation providers are enabled.
