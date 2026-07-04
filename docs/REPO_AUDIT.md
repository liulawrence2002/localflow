# LocalFlow Repository Audit (Phase 0)

_Evidence-based audit produced before any code changes. All claims cite `file:line`._
_Baseline captured on branch `master`; toolchain: cargo 1.96.1, node/vitest 3.2.6._

> Note on the spec file: the engineering spec was delivered inline to the agent as
> `localflow_upgrade_coding_agent_prompt.md`. There is **no** `LOCALFLOW_UPGRADE_SPEC.md`
> file in the repo. The inline document is treated as authoritative.

---

## 1. Baseline check status (run before editing)

| Check | Command | Result |
|---|---|---|
| Frontend tests | `npm run test` (`vitest run`) | **77 passed**, 20 files |
| Type check | `tsc --noEmit` | pass (exit 0) |
| Lint | `eslint .` | pass (exit 0) |
| Format | `prettier --check ...` | pass (exit 0) |
| Rust format | `cargo fmt --check` | pass (exit 0) |
| Rust tests | `cargo test` | **14 passed**, 0 failed |
| Rust build | `cargo test` build | pass, **14 dead-code warnings** (mock/trait layer unused) |

No environmental blockers for building or running the test suites. **Blocker for
runtime validation:** this environment has no microphone, no bundled `whisper-cli.exe`,
and no running Ollama, so the *live* dictation path cannot be exercised here. Live
behavior changes are validated by deterministic unit tests plus documented manual steps.

---

## 2. Actual current architecture

LocalFlow currently contains **two disjoint backends plus a third, untyped runtime phase
vocabulary** — three separate representations of "a dictation session" that do not share code.

### 2a. The real production path — the "god module"
`src-tauri/src/native_dictation.rs` (1330 LOC) is the only path that touches a real
microphone, whisper, Ollama, or the keyboard. It is triggered by the global hotkey
(`hotkeys/mod.rs:46` → `native_dictation::handle_hotkey`), **not** by any Tauri command.
It directly owns every concern the spec says to separate (§4.1):

- Hotkey command dispatch + tap/hold logic — `recorder_loop` (`native_dictation.rs:145`), `should_ignore_quick_release`/`should_toggle_stop` (`:196`).
- Microphone capture via `cpal`, F32/I16/U16, loudest-channel downmix — `start_recording` (`:204`), `build_stream` (`:552`), `downmix_input_callback` (`:612`).
- Live VAD / end-of-speech + overlay audio features (level/pitch/brightness) — `start_level_meter` (`:350`), `EndOfSpeechDetector` (`:398`), `estimate_pitch_normalized` (`:476`).
- Resample to 16 kHz + WAV write — `resample_to_16khz` (`:697`), `write_wav` (`:721`).
- **Cold-start whisper per utterance** — `run_whisper` (`:743`) spawns `whisper-cli.exe` via `Command::...output()` (`:751-768`); model on disk read fresh each time.
- Ollama refinement (blocking reqwest, strict JSON, one repair retry, raw-transcript fallback) — `refine_with_pinned_ollama` (`:837`), `request_ollama_generate` (`:859`).
- Clipboard + `Ctrl+V` insertion via Win32 `SendInput`, restore prior **text** clipboard — `paste_text` (`:986`), `send_ctrl_v` (`:1004`).
- Overlay window show/hide/position — `emit_native_event` (`:1118`), `position_overlay` (`:1185`).

Concurrency guard is a single `Option<RecordingSession>` on the recorder thread
(`:146`) plus an `overlay_epoch` AtomicU64 (`:30`) for overlay-hide races. There is
**no `session_id`, no phase enum, no cancellation** of in-flight work: `finish_recording`
runs the blocking whisper `.output()` and Ollama `.send()` to completion on the single
recorder thread (`:260-347`), so nothing can interrupt or supersede a transcription.

### 2b. The mock/spec skeleton — trait + state machine, never wired
`app_state.rs` (413 LOC) + `workflow/mod.rs` (432 LOC) + the thin modules
(`asr/`, `audio/`, `context/`, `insertion/`, `refinement/`, `platform/`) implement the
*intended* architecture — but only as **mocks/stubs reachable exclusively through the
`begin_mock_session`/`finish_mock_session` Tauri commands**:

- `workflow/mod.rs:158` — a real reducer `transition(state, event)` with `DictationPhase` (`:4`), `session_id` threading, stale-event rejection (`:159-171`), overlap rejection (`:193`), `Cancel` (`:181`). Tested (`rejects_stale_session_results`, `rejects_overlapping_sessions`).
- `asr/mod.rs` — trait `AsrProvider` + `MockAsrProvider` (canned string); `WhisperCppProviderConfig` unused.
- `refinement/mod.rs` — trait `RefinementProvider` (non-streaming) + `Mock`/`NoOp`; `OllamaRefinementProviderConfig`/`LlamaCppRefinementProviderConfig` **never constructed** (dead-code warnings).
- `context/mod.rs` — rich `TextContext` struct but only `EmptyContextProvider` returning `default()`.
- `insertion/mod.rs`, `audio/mod.rs`, `platform/mod.rs` — trait + mock only.
- `storage/mod.rs` — creates SQLite tables `settings`, `dictation_history`, `dictionary_entries` (`:12-37`) but **nothing ever reads or writes rows**; real settings/history live in in-memory mutexes in `app_state.rs`.

`finish_mock_session` (`app_state.rs:203`) drives the mock providers through the reducer
and is what the main-window UI "Start/Finish/Cancel" buttons call. It never touches the
mic, whisper, or Ollama.

### 2c. The TypeScript domain layer — a third, well-tested parallel pipeline
`src/domain/*` (24 modules, 77 tests) is a nearly complete, pure-function reimplementation
of the dictation pipeline that is **disconnected from the native production path**. It
contains genuinely valuable, well-tested logic that is not on the shipping path:

- State machine `stateMachine.ts` (duplicate of Rust `workflow`), `transcriptStabilizer.ts` (LCP partial-commit), `personalization.ts` (spoken punctuation / replacements / snippets / self-correction), `insertion.ts`/`insertionPlan.ts` (spacing, target validation, duplicate guard, clipboard planning), `commandMode.ts`, `context.ts` (app classification + privacy gating), `privacy.ts` (retention), `networkPolicy.ts` (`evaluateDictationNetworkUrl`), `asrWindows.ts`, `audio.ts` (ring buffer/VAD), `whisperSidecar.ts` (arg builder + dictionary initial-prompt), `refinementPipeline.ts`, `diagnostics.ts`, `performance.ts`.
- Only `ollama.ts` does live I/O, and only for the Models screen "Check" button — not for dictation.
- `App.tsx` triggers **mock** sessions only (`beginMockSession`/`finishMockSession`, `App.tsx:124,131`). The real overlay lives in a separate window `VoiceOverlay.tsx` consuming the `localflow://native-dictation` event, with its **own** phase union (`"idle"|"listening"|"processing"|"refining"|"inserted"|"error"`, `VoiceOverlay.tsx:4`) that matches neither state machine.

### 2d. Build/config facts
- `Cargo.toml`: `windows = "0.61"` enables **only** `Win32_UI_Input_KeyboardAndMouse` — **no UI Automation / accessibility feature**, so context capture and UIA insertion are currently impossible without a dependency change. `reqwest` uses `blocking`.
- `tauri.conf.json`: windows `main` (hidden at start) + `overlay` (transparent, always-on-top, non-focusable, skip-taskbar); `security.csp: null`; bundles the whisper runtime under `localflow-runtime/`.
- `capabilities/default.json`: `core:default`, `opener:default`, `global-shortcut:*` only.

---

## 3. Documentation vs. code discrepancies

The docs are unusually honest about *pending* work (development/PLAN.md marks Milestones 2–5 "partial";
development/handoff.md lists UIA insertion, target verification, native persistence as pending). The
material discrepancies are about **which layer a described behavior actually lives in**:

| # | Doc claim | Reality | Severity |
|---|---|---|---|
| D1 | `ARCHITECTURE.md:20` presents `Idle→Preparing→…→Complete` as *the* state machine | That reducer exists only in the mock `workflow`/TS layers; the real native path emits ad-hoc **string** phases and uses no reducer (`native_dictation.rs:248,274,320,335`) | High — misrepresents production |
| D2 | `SECURITY.md:6,15`, `PLAN.md:17` "state machine prevents overlapping + stale sessions inserting into wrong window"; "Duplicate insertion guard per session" | True **only** in the mock workflow; the native path has no session id, no stale/overlap/duplicate guard, and no target check before paste (`native_dictation.rs:986`) | High — security claim overstated for the shipping path |
| D3 | `network-policy.md:18-19`, `SECURITY.md:17` remote URLs rejected by `evaluateDictationNetworkUrl` before fetch | Enforcement is **TS-only** (`networkPolicy.ts`); the native Rust dictation posts to a hardcoded `127.0.0.1` const with no allowlist helper (`native_dictation.rs:100,870`). Safe in practice, but the described guard is not on the production path | Medium |
| D4 | `PRIVACY.md`/`SECURITY.md` retention, context redaction, diagnostics redaction as controls | Implemented in TS `privacy.ts`/`diagnostics.ts`; **not enforced** in the native SQLite layer (which is never written). Docs partly acknowledge this (`PRIVACY.md:36`) | Medium |
| D5 | `README.md`/`ARCHITECTURE.md` imply personalization (dictionary/replacements/snippets/styles) participates in cleanup | Native `build_cleanup_prompt` receives only the raw transcript and hardcodes `"cleanupLevel":"balanced"` (`native_dictation.rs:951`); no personalization reaches the LLM. Docs acknowledge this at `handoff.md:312`, `PLAN.md:74` | Medium |
| D6 | Test counts "77 frontend / 14 Rust" | Confirmed accurate (`vitest` 77, `cargo test` 14) | None (verified true) |
| D7 | Model IDs `gemma4:12b-it-qat`, `ggml-tiny.en-q5_1.bin` | Confirmed hardcoded (`native_dictation.rs:99,1063`); settings value is ignored by the native path | Note only |

---

## 4. Dead code & duplicated logic

- **Duplicated state machine (×3):** TS `stateMachine.ts`, Rust `workflow/mod.rs`, and the untyped native string phases.
- **Duplicated cleanup-prompt contract:** TS `refinementPipeline.buildCleanupPrompt` vs Rust `build_cleanup_prompt` (`native_dictation.rs:951`) emit the same `localflow.dictation_cleanup` shape but are independent implementations; only the Rust one runs in production, and it omits the personalization fields the TS one includes.
- **Dead Rust:** `OllamaRefinementProviderConfig`, `LlamaCppRefinementProviderConfig`, `WhisperCppProviderConfig`, workflow `WorkflowEvent::Fail`/`Reset`, `MockAsrProvider`/`MockTextInserter` — 14 dead-code warnings on build.
- **Unimported TS domain modules** (only referenced by `tests/` + `mockPipeline`): `whisperSidecar.ts`, `asrWindows.ts`, `audio.ts`, `insertionPlan.ts`, `transcriptStabilizer.ts`, `context.ts`, `commandMode.ts`, `performance.ts` — high-quality logic stranded off the production path.
- **Unused SQLite schema:** `storage/mod.rs` tables created, never used.

---

## 5. Risk-ranked technical-debt list

| Rank | Risk | Evidence | Impact |
|---|---|---|---|
| R1 | **Insertion into an unverified target.** ✅ **Addressed (Slice 2).** LocalFlow now captures the foreground window (HWND + PID) at record start and revalidates it before pasting (`TargetWindow`/`target_matches`, fail-closed). Remaining hardening: process-start-time (PID reuse), UIA element identity, protected-field detection, paste-last recovery. | `native_dictation.rs` `target_matches` | Was the single biggest safety hole |
| R2 | **No session identity or cancellation.** ✅ **Addressed (Slice 1).** Native `SessionRegistry` + worker thread; superseded/cancelled sessions revalidate and never insert (spec §4.4). Remaining: wire an Escape-to-cancel hotkey to the existing `cancel` command. | `native_dictation.rs` `SessionRegistry` | Was: wrong/duplicate insertion |
| R3 | **God module.** All concerns in one 1330-line file; no provider boundaries; untestable end-to-end. Violates §4.1. | `native_dictation.rs` | Blocks every later feature safely |
| R4 | **Mock commands shipped in production invoke handler.** UI can only trigger mock dictation; real path is hotkey-only. Violates §4.2. | `lib.rs:45-51`, `App.tsx:124` | Confusing; mock path masquerades as product |
| R5 | **Cold-start whisper per utterance.** 🟨 **Foundation landed (Slice 8).** Typed `AsrEvent` contract, `StreamingAsrProvider` trait, overlap-dedup stabilizer, rolling-window planner, and WER metric are built and tested (`src-tauri/src/asr`). Still needed (requires local models to verify): a persistent whisper provider on the trait + the benchmark harness. | `native_dictation.rs run_whisper`, `asr/streaming.rs` | Latency; no real-time UX yet |
| R6 | **Personalization/formatting not on production path.** 🟨 **Largely addressed (Slices 4-5).** Deterministic formatting (`transcript` module) plus dictionary→whisper biasing, replacements, snippets, and a configurable model now run natively. Remaining: style profiles in the cleanup prompt, and native context capture. | `native_dictation.rs process_session`, `transcript/mod.rs` | Was a product parity gap |
| R7 | **Privacy guarantees enforced only in the unused TS/mock layer.** Retention, redaction, network allowlist not enforced natively. | see D3/D4 | Claims exceed enforcement |
| R8 | **SQLite unused; settings not persisted.** `save_settings` writes an in-memory mutex only. | `app_state.rs:172`, `storage/mod.rs` | No durable settings/history/retention |
| R9 | **Clipboard fallback loses non-text formats** and uses a fixed 700 ms sleep as "paste done." | `native_dictation.rs:989-999` | Clobbers rich clipboard; racy |
| R10 | **CSP disabled** (`security.csp: null`). | `tauri.conf.json` | Hardening gap |

---

## 6. Privacy & security review (current state)

Positives that are real in the native path: transcripts are **not** logged (only char
counts / timings — `native_dictation.rs:336-342`); temp WAV + JSON are deleted
(`:344-345`); overlay receives only audio features, never transcript text; refinement
targets a hardcoded loopback URL; `privacy::redact_for_log` exists. Gaps: R1 (unverified
target), R2 (stale insertion), R7 (retention/allowlist enforced only off-path), CSP null.

---

## 7. Performance baseline

Not yet measured on real hardware (no mic/whisper/Ollama in this environment). Structural
facts that bound latency today: cold-process whisper spawn + disk model load **per
utterance** (`native_dictation.rs:751`); a 12B refinement model (`gemma4:12b-it-qat`) with
a 60 s timeout and background warm-up + 30 min keep-alive (`:112,902`); fixed 700 ms
post-paste sleep (`:995`). A benchmark harness (spec §8) does not yet exist. See
`PARITY_MATRIX.md` §"ASR benchmark plan".

---

## 8. Recommended engine decision points

1. **whisper.cpp persistent/streaming** (stream partials from a long-lived process or in-process binding) — first candidate; reuses existing model/runtime, keeps offline default.
2. **sherpa-onnx** streaming — evaluate only if license, Windows packaging size, and Rust integration are acceptable.
3. Keep the current one-shot `whisper-cli.exe` as a labeled compatibility provider behind the new `StreamingAsrProvider` trait; do not let it define the architecture.
Decision to be made on measured accuracy/latency/memory/stability, not marketing. Harness spec in PARITY_MATRIX.md.

---

## 9. File-by-file implementation plan (sequenced, small slices)

Following the spec's phase order, each slice must leave the repo buildable + green:

- **Slice 1 (this PR) — Session identity + supersede/cancel guard in the native path.**
  Add a testable session registry; run the transcribe→refine→insert tail on a worker so a
  new or cancelled session supersedes an in-flight one and a **stale/cancelled session
  never pastes** (addresses R2, part of §4.4). Files: `native_dictation.rs` (+ unit tests).
- **Slice 2 — Target-window capture + revalidation before paste** (R1, §6.1/6.2): capture
  foreground HWND/PID/process-start at record start; abort paste if focus changed. Requires
  adding `Win32_UI_WindowsAndMessaging` feature.
- **Slice 3 — Module split** of `native_dictation.rs` into `dictation/`, `audio/`, `asr/`,
  `insertion/` behind the provider traits (R3, §4.1) — pure refactor, behavior preserved.
- **Slice 4 — Remove mock commands from production; wire the real state machine** into the
  native coordinator (R4, §4.2/§4.4).
- **Slice 5 — StreamingAsrProvider trait + persistent whisper**; benchmark harness (R5, §3.2/§4.5).
- **Slice 6 — Wire deterministic personalization + context into the native cleanup path** (R6, §3.3/§3.4).
- **Slice 7 — Native SQLite repositories + retention enforcement** (R8, §7).

---

## 10. Commands run to establish baseline

```
npm run test          # → 77 passed (20 files)
npx tsc --noEmit      # → exit 0
npx eslint .          # → exit 0
npx prettier --check "src/**/*.{ts,tsx,css}" "tests/**/*.ts" "*.{json,ts,js,md}"  # → exit 0
cd src-tauri && cargo fmt --check   # → exit 0
cd src-tauri && cargo test          # → 14 passed; 14 dead-code warnings
```
