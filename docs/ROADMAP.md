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

- Speech-to-text provider layer: persistent `whisper-server` engine with supervision and a
  local CLI fallback, opt-in base.en and Parakeet engines, per-utterance deadlines.
- Band-limited resampler, locked capture channel, watchdog timeouts on device loss.
- Loopback-only HTTP clients (no proxy, no redirects), clipboard history exclusion, bounded
  Ollama context.
- Reproducible speech-to-text benchmark harness and a measured baseline
  (`benchmarks/ASR_BENCHMARK.md`).

- Parakeet TDT 0.6B v3 kept loaded in-process as the default speech model (0.93 % WER here
  against 3.31 % for tiny.en), with tiny.en and the Whisper CLI as the fallback.
- Read-aloud wiring for Fish Audio S2 Pro through s2.cpp (implemented and unit-tested; the
  engine itself not yet built or verified).

- Dictation code split into shared modules (`audio/capture`, `audio/vad`, `asr/engine_select`,
  `llm`, `insertion`, `overlay`, `runtime_paths`); settings tolerate missing fields; content
  security policy and per-window command permissions; clipboard tidy-up; CI workflow.

- Conversation mode, first version: hotkey-toggled hands-free conversation with the local
  model (Silero voice detection, Parakeet, streaming Ollama chat, Phonon voice), interrupt by
  hotkey, time limits, and a reproducible end-to-end benchmark
  (`benchmarks/CONVERSATION_BENCHMARK.md`).

## Now

- Try conversation mode with a real microphone and speaker; judge the voice.
- Screen-aware reading and the remaining conversation milestones: see `VOICE_ASSISTANT_PLAN.md` for
  the audit, measured baselines, technology choices and milestones.

- Manually verify native dictation in Notepad, a browser field, and VS Code (needs hardware).
- Wire the streaming event contract into the native coordinator behind a feature flag.

## Next

- Record real 48 kHz dictations and compare old and new builds end to end; the benchmark
  so far covers the speech-to-text stage on clean read speech only.
- Build and verify the Fish S2 read-aloud engine (`scripts/Build-FishS2.ps1`), measure its
  speed on CPU and through Vulkan, and decide whether it is usable or whether a small CPU
  voice (Pocket TTS, Kokoro) should be offered alongside it.
- Vulkan GPU builds for the speech model, and unloading it after a period without use.
- Decide whether the installer should carry the Parakeet model or download it on request.
- Update the bundled whisper.cpp runtime from 1.9.1 to 1.9.5 or later.
- Streaming partial transcripts on the `StreamingAsrProvider` foundation.
- Persist personalization/settings/history through native SQLite; enforce retention there.
- Add microphone selection and device-disconnect handling.
- Add Windows UI Automation insertion before clipboard fallback; native context capture.

## Later

- Apply style profiles to the cleanup prompt; command mode and selected-text transforms.
- Startup-at-login, crash recovery, sleep/resume, and full manual acceptance testing.
- Package and sign the Windows installer.
