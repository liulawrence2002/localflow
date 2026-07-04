# Roadmap

Status reflects the spec-driven upgrade. See `development/PLAN.md` for per-slice detail and
`PARITY_MATRIX.md` for capability-by-capability status.

## Done (native production path)

- Session identity + supersede/cancel guard; a stale or cancelled session never inserts.
- Target-window revalidation before insertion (fail-closed); Escape cancels while recording.
- Copy/last-transcript recovery (tray + Home card), transcript kept in volatile memory.
- Deterministic smart formatting (spoken punctuation, self-corrections, filler/stutter
  cleanup, sentence capitalization, URL/email/decimal protection).
- Personalization on the native path: replacements, snippets, dictionary → whisper prompt.
- Configurable local Ollama refinement model; honest diagnostics.
- Streaming ASR foundation: typed event contract, `StreamingAsrProvider` trait, overlap-dedup
  stabilizer, rolling-window planner, and a WER metric — all unit-tested (`src-tauri/src/asr`).

## Now

- Manually verify native dictation in Notepad, a browser field, and VS Code (needs hardware).
- Wire the streaming event contract into the native coordinator behind a feature flag.

## Next

- Implement a persistent/streaming `whisper.cpp` provider behind `StreamingAsrProvider` and
  build the benchmark harness (spec §8) using the WER metric and the documented corpus.
- Persist personalization/settings/history through native SQLite; enforce retention there.
- Add microphone selection and device-disconnect handling.
- Add Windows UI Automation insertion before clipboard fallback; native context capture.

## Later

- Apply style profiles to the cleanup prompt; command mode and selected-text transforms.
- Startup-at-login, crash recovery, sleep/resume, and full manual acceptance testing.
- Split `native_dictation.rs` into `audio`/`asr`/`insertion`/`dictation` modules behind the
  provider traits.
- Package and sign the Windows installer.
