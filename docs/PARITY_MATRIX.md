# LocalFlow Parity Matrix & Benchmark Plan (Phase 0)

Compares LocalFlow's current behavior against the target Wispr Flow–class experience
described in the engineering spec. Status legend:

- ✅ **Production** — implemented on the real native path.
- 🟨 **Off-path** — implemented in the TS/mock layer only; not wired to native dictation.
- 🟦 **Stub** — trait/struct/schema exists but returns mock/default/no-op.
- ❌ **Absent** — no implementation.

All rows cite `file:line` evidence.

---

## 1. Universal dictation experience (spec §3.1)

| Capability | Status | Evidence / note |
|---|---|---|
| Background tray app | ✅ | `lib.rs:56` tray; main window hidden at start |
| Configurable push-to-talk shortcut | 🟨 | Default `Ctrl+Alt+Space` registered (`hotkeys/mod.rs`); not user-configurable at runtime |
| Hands-free start/stop | ✅ (tap/hold) | `should_toggle_stop` (`native_dictation.rs:200`) |
| Mouse-button shortcut | ❌ | — |
| Escape-to-cancel every phase | ✅ (recording) | Escape is registered while recording and cancels the session (Slice 6); mid-processing cancel is covered by supersede. Escape is not suppressed system-wide when idle |
| Non-focus-stealing overlay | ✅ | overlay window `focusable:false` (`tauri.conf.json`) |
| Mic selection + live level test | 🟨 | UI + `audio.ts` exist; native path always uses default device (`native_dictation.rs:210`) |
| Startup-at-login | 🟦 | `platform::NoOpStartupRegistration` |
| Insertion into arbitrary controls | ✅ (clipboard) | `paste_text` (`:986`) — works where Ctrl+V works |
| Recovery when insertion fails | 🟨 partial | Native path now **detects** focus-change and refuses to paste into the wrong window (Slice 2), showing a clear error; paste/copy-last, retry, and scratchpad are still pending |

## 2. Real-time ASR contract (spec §3.2)

| Capability | Status | Evidence |
|---|---|---|
| Explicit `AsrEvent` protocol (Partial/Committed/Final…) | 🟨 | Typed `AsrEvent` enum + `StreamingSession` coordinator built and tested (`asr/streaming.rs`, Slice 8); native path still emits string phases until wired |
| Continuous PCM streaming into ASR | ❌ | Full WAV then one-shot CLI; persistent provider is the next step |
| VAD auto-commit + manual commit | 🟨 partial | VAD auto-stop exists (`EndOfSpeechDetector`); streaming commit pending |
| Partial + committed transcript events | 🟨 | `StreamingSession` emits them from rolling-window hypotheses (tested, Slice 8); not yet driven by a live decoder |
| Word/segment timestamps | ❌ | Whisper called with `-nt` (no timestamps) |
| Language config / auto-detect | 🟨 | Fixed `-l en` on the native path |
| Keyterm/vocabulary biasing | ✅ | Dictionary terms seed whisper's `--prompt` on the native path, bounded to 800 chars (Slice 5) |
| Verbatim vs non-verbatim | 🟨 | Cleanup levels in TS only |
| No duplicate committed words | ✅ (engine) | `TranscriptStabilizer` guarantees committed text only grows by stable prefix; unit-tested for overlapping windows (Slice 8) |
| `StreamingAsrProvider` trait | ✅ | Defined + capabilities + tested via `MockStreamingProvider` (`asr/streaming.rs`, Slice 8); a persistent-whisper impl is next |

## 3. Smart writing (spec §3.3)

| Capability | Status | Evidence |
|---|---|---|
| Filler / repeated-word cleanup | ✅ | Deterministic in the native `transcript` module (Slice 4) |
| Spoken punctuation | ✅ | `transcript::apply_spoken_punctuation` on the native path (Slice 4) |
| Number/date/currency/URL/email normalization | 🟨 | URLs/emails/decimals are protected from mangling (Slice 4); rich normalization still deferred to the LLM |
| List formatting | 🟨 | "bullet point" starts list lines with capitalization (Slice 4); numbered lists pending |
| Self-correction / backtracking | ✅ | `transcript::resolve_self_corrections` (actually/no/sorry/restart) on the native path (Slice 4) |
| Cursor-aware capitalization/spacing | 🟨 | Sentence capitalization is native (Slice 4); cursor-context spacing still TS-only |
| App-aware trailing punctuation | 🟨 | `context.ts` classify (TS only) |
| Snippets / replacements | ✅ | Applied on the native path from user settings (Slice 5) |
| Protected literals (code/URLs/paths) | 🟨 | URLs/emails/decimals protected in formatting; the cleanup prompt also instructs verbatim preservation (Slices 4-5); span-level protection pending |
| Press-enter opt-in | ❌ | — |

## 4. Context awareness (spec §3.4)

| Capability | Status | Evidence |
|---|---|---|
| Active process / window / UIA element | ❌ | No Win32 window/UIA calls anywhere; `windows` crate lacks the feature (`Cargo.toml:35`) |
| Protected/sensitive/read-only detection | 🟨 | `context.ts` fields exist (TS); native captures nothing |
| Bounded before/after caret text | 🟨 | `context.ts` (TS only) |
| App category classification | 🟨 | `classifyApplicationCategory` (TS only) |
| Per-source privacy gates, bounding, redaction | 🟨 | `privacy.ts`/`context.ts` (TS only) |

## 5. Personalization (spec §3.5)

| Capability | Status | Evidence |
|---|---|---|
| Dictionary / replacements / snippets / styles editors | 🟨 | `App.tsx` editors + `settings.ts`; persisted only to in-memory mutex / localStorage (durable SQLite pending) |
| Applied in native production path | ✅ (dictionary/replacements/snippets) | Read from settings in `process_session`; dictionary biases whisper `--prompt`, replacements/snippets applied deterministically (Slice 5). Style profiles not yet applied |
| Configurable refinement model | ✅ | Ollama model read from settings on the native path (Slice 5) |
| Import/export with schema validation | 🟨 | TS helpers; not wired to durable storage |
| Opt-in auto-learning from corrections | ❌ | — |

## 6. Command / transform mode (spec §3.6)

| Capability | Status | Evidence |
|---|---|---|
| Command-mode state machine | 🟨 | `commandMode.planCommandMode` (TS); no native command hotkey |
| Selected-text transforms | 🟨 | TS planning only |
| OS-command rejection | 🟨 | `commandMode.ts` (TS) |
| Preview/diff for large changes | ❌ | — |

## 7. Recovery (spec §3.7)

| Capability | Status | Evidence |
|---|---|---|
| Paste/copy last transcript | 🟨 | Copy-last is done (tray + Home > Recovery card, `get_last_transcript`/`copy_last_transcript`, Slice 3); paste-last-into-focus via a global hotkey is pending |
| Retry insertion | ❌ | — |
| Undo AI cleanup → raw | 🟨 | `undo.ts` (TS, main-window only) |
| Scratchpad target | ❌ | — |
| Clear "transcribed but insertion failed" notice | 🟨 | Focus-change now emits a specific "not pasted — use Copy last transcript" message (Slice 2/3); a richer in-UI notice is pending |

## 8. Architecture non-negotiables (spec §4)

| Requirement | Status | Evidence |
|---|---|---|
| No production god path | ❌ | `native_dictation.rs` (1330 LOC) |
| One shared production pipeline | ❌ | 3 divergent representations (audit §2) |
| Provider traits used in production | 🟨 | `StreamingAsrProvider` defined + tested (Slice 8); a persistent impl and native wiring are next. `RefinementProvider` still mock-only |
| session_id + cancellation everywhere | 🟨 | Native path carries a session id + supersede/cancel/Escape guard and revalidates before insertion (Slices 1–2, 6); full state-machine routing pending |
| Persistent local runtime | 🟨 | Cold whisper per utterance today; streaming contract/coordinator/stabilizer built (Slice 8), persistent provider next |
| Model registry + checksums | ❌ | Hardcoded model consts (`:99,1063`) |

## 9. Storage & retention (spec §7)

| Requirement | Status | Evidence |
|---|---|---|
| Versioned migrations + typed repos | 🟦 | Tables created once, never used (`storage/mod.rs`) |
| Retention enforced in persistence layer | ❌ | TS-only policy (`privacy.ts`) |
| Sensitive content excluded from metrics | n/a | No metrics tables written |

## 10. Security & privacy (spec §9)

| Requirement | Status | Evidence |
|---|---|---|
| No cloud ASR/LLM by default | ✅ | Loopback only (`:100`) |
| No telemetry | ✅ | None present |
| No transcript in logs | ✅ | Char counts only (`:336`) |
| No stale-session insertion | ✅ | `SessionRegistry` supersede/cancel guard (Slice 1) |
| No wrong-window insertion | ✅ | `target_matches` foreground revalidation, fail-closed (Slice 2) |
| No arbitrary OS command execution | ✅ | Native runs no shell from model output |
| No model download without user action | ✅ | Models resolved from disk only |
| Network-policy test (fails on non-loopback) | 🟨 | `network-policy.test.ts` covers TS helper; native path untested |

---

## ASR benchmark plan (spec §4.5, §8)

**Goal:** choose the default ASR engine on measured evidence, not marketing.

### Providers under test (behind a future `StreamingAsrProvider` trait)
1. **P0 — current one-shot `whisper-cli.exe`** (compatibility baseline). No code change; measured as-is.
2. **P1 — persistent whisper.cpp** — long-lived process fed rolling PCM windows, or in-process binding (`whisper-rs`), emitting partials + committed segments.
3. **P2 — sherpa-onnx streaming** — evaluated only if license (Apache-2.0), Windows x64 packaging size, model quality, and Rust integration are acceptable.

### Fixed corpus (committed as WAV + reference text; never model binaries)
Short messages · long paragraphs · numbered/bulleted lists · names & technical terms ·
explicit corrections/false starts · numbers/dates/currency/URLs/emails · quiet speech ·
background noise · ≥2 accents · code identifiers & filenames. (spec §8 corpus.)

### Metrics (distributions, not single numbers)
- Accuracy: WER (implemented — `asr::metrics::word_error_rate`, Slice 8) + entity-level accuracy (names/numbers/code symbols) vs reference.
- Latency: hotkey→capture, first-partial, stable-partial, silence→final, end-to-end.
- Resource: peak RAM, cold vs warm model load, CPU%.
- Stability: process crashes/restarts over N runs; duplicate-word incidence on overlap.

### Harness (to build in Slice 5)
A `cargo` bench binary (`src-tauri/src/bin/asr_bench.rs`) that feeds each corpus WAV
through each provider, writes a JSON/CSV of per-utterance metrics to `docs/benchmarks/`,
and a summary table. Runs offline; requires locally installed models (documented, not
downloaded by the harness). No transcripts written outside the local benchmark dir.

### Refinement-model benchmark (spec §5)
Same harness pattern: compare `gemma4:12b-it-qat` against ≥1 smaller local model
(e.g. a 3–4B instruct-tuned model) on cleanup quality (schema-valid rate, semantic-drift
rate, protected-span preservation) and warm latency. Default chosen on evidence; UI
auto-disables refinement if warm latency misses the target tier.
