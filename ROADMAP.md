# Roadmap

## Now

- Keep the Tauri tray app running reliably in dev mode.
- Manually verify native dictation in Notepad, a browser field, and VS Code.
- Wire deterministic personalization into the native hotkey path.
- Feed native ASR/insertion timings into diagnostics.

## Next

- Replace one-shot temporary WAV transcription with a managed `whisper.cpp` sidecar boundary.
- Add microphone selection, device-disconnect handling, cancellation, and timeout enforcement.
- Add target-window tracking and safer Windows UI Automation insertion before clipboard fallback.
- Wire local Ollama cleanup into the native dictation path with raw-transcript undo.

## Later

- Persist personalization settings fully through native SQLite.
- Add live stable partial dictation.
- Add command mode and selected-text transforms to the native workflow.
- Add startup-at-login, crash recovery, benchmarks, and full manual acceptance testing.
- Package and sign the Windows installer.
