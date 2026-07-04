# LocalFlow — Unified Handoff

This document is the single source of truth for the Wispr Flow–class upgrade of LocalFlow:
what the app is, everything that was done, the current architecture, how it was verified
(including live on this machine), what is known-incomplete, and what to do next.

- **App:** LocalFlow — a local-first Windows voice-dictation app (Tauri 2, Rust backend,
  React/TypeScript UI, SQLite, local `whisper.cpp`, local LLM refinement via Ollama).
- **Goal of this work:** move from a one-shot recorder with a monolithic "god path" and a
  mock UI toward a safe, personalized, recoverable, snappy dictation system with a
  streaming-ASR-shaped architecture — without cloud, telemetry, or unverifiable claims.
- **Status:** all changes are on the real native production path, each shipped with tests.
  Full suite green (**49 Rust tests, 81 TS tests**); models installed and the pipeline
  verified live; the dev app launches and runs.

---

## 1. Where it started (Phase 0 audit)

Evidence-based audit produced before any code change (see `../REPO_AUDIT.md` and
`../PARITY_MATRIX.md`). Key findings:

- **Three disjoint "session" representations** that shared no code:
  1. `src-tauri/src/native_dictation.rs` (1330 LOC) — the only path that touched a real mic,
     whisper, Ollama, or the keyboard; a monolithic "god module" (hotkey → capture → VAD →
     WAV → cold whisper spawn → Ollama → clipboard paste → overlay).
  2. A trait + state-machine "spec skeleton" (`workflow`, `app_state`, and thin `asr`/`audio`/
     `insertion`/… modules) reachable **only** through mock Tauri commands.
  3. A large, well-tested TypeScript `src/domain/*` layer that was **off** the production path.
- **Mock commands shipped in the production invoke handler**; the real dictation was
  hotkey-only.
- **No session identity, cancellation, or target verification** in the real path; insertion
  was clipboard+Ctrl+V into whatever had focus at paste time.
- Whisper spawned **cold per utterance**; models **hardcoded** (`gemma4:12b-it-qat`,
  `ggml-tiny.en-q5_1.bin`); personalization/formatting existed only in the unused TS layer.
- Baseline (all green): 77 TS tests, 14 Rust tests, lint/format/type checks clean.

---

## 2. Everything that was done (by slice)

Each slice left the repo buildable and green. Rust test count in parentheses is cumulative.

| #   | Slice                                               | Summary                                                                                                                                                                                                                                                                                                                                                                                                    |
| --- | --------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0   | **Audit**                                           | `../REPO_AUDIT.md` + `../PARITY_MATRIX.md`; baseline captured.                                                                                                                                                                                                                                                                                                                                         |
| 1   | **Session identity + cancellation** (17)            | `SessionRegistry` (session id + supersede/cancel). The transcribe→refine→insert tail moved to a worker thread that revalidates the session before every side effect, so a **superseded or cancelled session never inserts** (spec §4.4).                                                                                                                                                                   |
| 2   | **Target-window revalidation** (19)                 | Capture the foreground window (`TargetWindow{hwnd,pid}`) at record start; before pasting, **skip insertion (fail-closed)** unless the same window is still focused (spec §6.1/§6.2). Added `Win32_Foundation` + `Win32_UI_WindowsAndMessaging` crate features.                                                                                                                                             |
| 3   | **Copy/last-transcript recovery** (+3 TS)           | Last finalized transcript kept in volatile memory; recoverable via a **tray item** and a **Home → Recovery card** (`get_last_transcript`/`copy_last_transcript`). Reveal-preview is opt-in; nothing persisted. Pure helper `src/domain/recovery.ts`.                                                                                                                                                       |
| 4   | **Deterministic smart formatting** (29)             | New authoritative Rust `transcript` module: spoken punctuation ("comma", "new line", "bullet point"), explicit self-corrections ("Tuesday, actually Wednesday" / "no" / "sorry" / "let me restart"), filler/stutter cleanup, sentence capitalization, **URL/email/decimal protection**. Runs before the LLM (seeds the prompt) and is the fallback when the LLM is unavailable.                            |
| 5   | **Personalization + model config** (32)             | Native path reads user settings: exact **replacements + snippets** applied deterministically; **dictionary** terms bias whisper via `--prompt` (bounded 800 chars); the **Ollama model is configurable** (no longer hardcoded).                                                                                                                                                                            |
| 6   | **Escape-to-cancel** (32)                           | Escape is registered **only while recording** and routed to the `cancel` command; not suppressed system-wide otherwise.                                                                                                                                                                                                                                                                                    |
| 7   | **Honest diagnostics + labeled test controls** (32) | Diagnostics describe the real pipeline (and flag one-shot ASR as a known limit); the Home panel is clearly labeled "Simulated test" so mock controls don't masquerade as production (spec §4.2).                                                                                                                                                                                                           |
| 8   | **Streaming ASR foundation** (47)                   | New `src-tauri/src/asr/` submodules: typed `AsrEvent` contract + `AsrCapabilities` + `StreamingAsrProvider` trait; overlap-dedup `TranscriptStabilizer` + `StreamingSession` coordinator (**committed words never duplicated**, spec §3.2); rolling-window planner; `word_error_rate` metric for the benchmark harness. Unit-tested but **not yet on the default one-shot path** (`#![allow(dead_code)]`). |
| 9   | **Snappier, Wispr-like feel** (48)                  | Timings tightened; silent idle close; faster default model + bounded timeout; instant no-LLM mode. Detail below.                                                                                                                                                                                                                                                                                           |
| 10  | **Cleanup prompt hardening** (49; +1 TS = 81)       | Cleanup prompt now requires the model to preserve the deterministic text's capitalization/punctuation/line-breaks/technical casing unless the raw transcript proves them wrong. Shared by the native and TS preview prompts, with regression tests on both.                                                                                                                                                |

### Slice 9 detail (the "make it feel like Wispr Flow" pass)

- **Timing constants** (`native_dictation.rs`): `END_OF_SPEECH_TIMEOUT_MS` 760→**550**,
  `NO_SPEECH_TIMEOUT_MS` 6000→**2500**, `MIN_AUTO_STOP_RECORDING_MS` 420→**350**;
  `schedule_overlay_hide` 1200→**700 ms**; `paste_text` clipboard-restore sleep 700→**400 ms**.
- **Silent idle close:** when speech was never detected, the level meter now sends `cancel`
  (overlay just hides) instead of `auto_stop` (which flashed a "too short" error).
- **Faster default model:** default cleanup model changed from the 12B `gemma4:12b-it-qat` to
  **`llama3.2:3b`**. Researched basis: **Wispr Flow's cleanup is itself a fine-tuned Llama
  model**; ElevenLabs Scribe is an ASR model (our local whisper equivalent), not a cleanup
  LLM. Our deterministic layer already does the heavy formatting, so a small Llama-family
  model is the right fit. Still configurable in Settings (`qwen2.5:3b` and the original 12B
  documented as alternatives). Cleanup HTTP timeout **60→20 s** so a slow model falls back to
  deterministic text quickly.
- **Instant mode:** the previously-unused `low_resource_mode` setting now **skips the LLM**
  and inserts the deterministically formatted text for the lowest latency.

### Slice 10 — cleanup prompt hardening (49 Rust, 81 TS)

- Added an explicit cleanup rule requiring the local model to **preserve the
  `deterministicText`'s capitalization, punctuation, line breaks, and technical casing**
  unless the raw transcript clearly proves them wrong. This fixes the observed case where a
  small model would undo capitalization the deterministic layer had already applied. The
  native prompt (`native_dictation.rs`) and the TypeScript preview prompt
  (`refinementPipeline.ts`) share the contract, with regression tests on both sides.

---

## 3. Current architecture & pipeline

Native dictation (the live hotkey path):

```
hotkey ─▶ cpal capture ─▶ VAD / end-of-speech ─▶ 16 kHz WAV
        ─▶ whisper-cli.exe  (dictionary-biased --prompt)
        ─▶ deterministic formatting  (transcript module + replacements/snippets)
        ─▶ configurable Ollama cleanup  (or skipped in low-resource mode)
        ─▶ session + target-window revalidation
        ─▶ clipboard paste (Ctrl+V, prior clipboard restored)
        ─▶ overlay status
```

Guarantees on this path:

- Every recording has a **session id**; a superseded/cancelled/Escape'd session **never
  inserts**.
- Insertion is **revalidated** against the originating window (fail-closed) — no wrong-window
  pastes.
- The finalized transcript is **recoverable** (tray / Home "Copy last transcript") if the
  paste is skipped or the LLM is unavailable.
- If the LLM fails or times out, the **deterministically formatted** text is inserted.

Key source files:

- `src-tauri/src/native_dictation.rs` — the whole live pipeline (session guard, VAD, whisper,
  refine, target check, paste, overlay, recovery commands).
- `src-tauri/src/transcript/mod.rs` — authoritative deterministic formatting + personalization.
- `src-tauri/src/asr/{stabilizer,windows,streaming,metrics}.rs` — streaming ASR foundation
  (Phase 3; not yet wired to the default path).
- `src-tauri/src/hotkeys/mod.rs` — global hotkey + Escape routing.
- `src-tauri/src/app_state.rs` — settings snapshot, mock/verification commands, diagnostics,
  `current_settings()` accessor used by the native path.
- `src-tauri/src/lib.rs` — command registration + tray (incl. "Copy last transcript").
- `src/services/localflowClient.ts`, `src/domain/recovery.ts`, `src/App.tsx` — recovery UI +
  model settings.
- `../REPO_AUDIT.md`, `../PARITY_MATRIX.md` — audit + capability status.

The TypeScript `src/domain/*` layer is now the **browser-dev fallback / preview** only; Rust
is authoritative for native production behavior.

---

## 4. Models & runtime (verified live on this machine)

- **Whisper (ASR):** `.localflow-runtime/whisper/Release/whisper-cli.exe` +
  `.localflow-runtime/models/ggml-tiny.en-q5_1.bin` present and wired (the native path
  resolves them from env override → bundled resources → `.localflow-runtime/`). The runtime
  also contains `whisper-server.exe`, `whisper-stream.exe`, and `parakeet-cli.exe` — real
  engines available for a future persistent/streaming provider.
- **Cleanup LLM:** Ollama running; **`llama3.2:3b` was pulled (2.0 GB) and verified live** —
  a real cleanup-contract call returned valid strict JSON and cleaned correctly. Measured
  **warm latency ~1.5–1.9 s**; a one-time **cold model load** (first call after launch / 30 min
  idle) can take several seconds up to ~45 s (mitigated by background warm-up at record start
  - 30 min keep-alive). `gemma4:12b-it-qat` (7.2 GB) remains installed as the slower option.
- **Default hotkey:** `Ctrl+Alt+Space`, with automatic fallback to **`Ctrl+Alt+Shift+Space`**
  when the primary is already registered by another app (as observed on this machine).

### Run it

```powershell
.\scripts\Start-Dev.ps1        # or: npm run tauri:dev
# ensure the cleanup model is present:
ollama pull llama3.2:3b
```

Main window starts hidden — open it from the **tray icon**. Dictate: focus a text field, tap
the active hotkey, speak, stop. Tray/Settings expose the model, `low_resource_mode` (instant,
no-LLM), dictionary/replacements/snippets, and "Copy last transcript".

---

## 5. Verification status

- `npm run test` → **81 frontend tests** pass.
- `npx tsc --noEmit`, `npx eslint .`, `npx prettier --check …` → clean.
- `npm run build` → succeeds.
- `cargo fmt --check` → clean.
- `cargo test` → **49 Rust tests** pass (14 baseline; +3 Slice 1; +2 Slice 2; +10 Slice 4;
  +3 Slice 5; +15 Slice 8; +1 Slice 9; +1 Slice 10). Build emits dead-code warnings only, all in the unused
  mock/scaffold layer.
- **Live:** Ollama cleanup call verified end-to-end; whisper runtime present; dev app compiles,
  initializes its SQLite DB, and runs.

Not verifiable in the build/CI environment (no mic/whisper/Ollama there): the live GUI +
microphone loop. That was validated separately on this workstation (models + app launch);
per-utterance feel should be confirmed by dictating.

---

## 6. Known limitations (honest)

- **ASR is still one-shot** whisper-cli per utterance. The streaming foundation (events,
  trait, stabilizer, windows, WER) is built and tested but **not wired** to the default path.
- **First dictation** after launch (or 30 min idle) can cold-load the model; with the 20 s
  cleanup timeout it may fall back to deterministic text once, then run warm (~1.5 s).
- **Settings are not persisted** across restarts (SQLite schema exists but is unused; settings
  live in an in-memory mutex). So the **default model governs each fresh launch** — if you
  select a model in Settings it applies for that session only.
- ASR language is fixed to English; whisper model path/threads are not settings-driven yet.
- **Style profiles** are not yet applied to the cleanup prompt.
- No **native context capture** or **UI Automation insertion** (clipboard paste only).
- `native_dictation.rs` has **not been split** into modules yet (still large).
- Mock `begin/finish_mock_session` commands remain in the invoke handler (now clearly labeled
  "Simulated test" in the UI).
- (Fixed in Slice 10) A small model could undo capitalization the deterministic layer applied;
  the cleanup prompt now explicitly requires preserving `deterministicText` casing/punctuation.

---

## 7. Remaining roadmap (recommended order)

1. **Persistent/streaming whisper provider** on the existing `StreamingAsrProvider` trait
   (your runtime already ships `whisper-server.exe` / `whisper-stream.exe` / `parakeet-cli.exe`),
   feed rolling windows through `StreamingSession`, emit partials to the overlay; then build
   the **benchmark harness** using `asr::metrics::word_error_rate` and the documented corpus.
2. **Native SQLite persistence + retention** for settings/dictionary/replacements/snippets/
   history (closes the "settings don't survive restart" gap; enforce retention in the
   persistence layer).
3. Optional **style-profile** application in the cleanup prompt (formality/concision/
   greetings/sign-offs). _(Capitalization-preservation hardening — done in Slice 10.)_
4. **UI Automation insertion** + **native context capture** (app-aware formatting,
   protected-field avoidance).
5. **Paste-last-into-focus** global hotkey; **module split** of `native_dictation.rs` behind
   the provider traits; cold-start warm-up tuning.

---

## 8. Security & privacy posture

- Local-only by default: cleanup hits `127.0.0.1:11434` (configured local model only); whisper
  runs locally. No telemetry, no cloud ASR/LLM, no silent remote fallback.
- Transcripts are **not logged** (only character counts / timings); temp WAV + JSON are deleted
  after transcription.
- The recovery transcript is held in **volatile memory only** (never written to disk); the UI
  shows it only on explicit reveal; the overlay shows audio features, never transcript text.
- Insertion safety: session guard + fail-closed target-window revalidation + Escape-to-cancel
  ensure text never lands in an unintended or unverifiable target.

---

_Detailed per-slice reasoning, file-by-file evidence, and the capability matrix live in
`../REPO_AUDIT.md`, `../PARITY_MATRIX.md`, and `PLAN.md`._
