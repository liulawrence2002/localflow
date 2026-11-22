# LocalFlow production-readiness review (architecture)

> Research appendix produced on 2026-11-22 by an independent research agent for the voice pipeline review. It is kept as written, apart from removing machine-specific paths. Line references to `native_dictation.rs` describe the code before the changes in `../VOICE_PIPELINE_REVIEW.md`. Figures are labelled by the author as vendor, third-party, reproduced, or unverified; treat unlabelled numbers as unverified.

Scope: read-only review of `the LocalFlow repository` on branch `feature/topographic-waveform-overlay` (HEAD `eb17641`).
Read in full: `src-tauri/src/native_dictation.rs`, `asr/*.rs`, `refinement/mod.rs`, `app_state.rs`, `lib.rs`, `hotkeys/mod.rs`, `desktop_health.rs`, `storage/mod.rs`, `tauri.conf.json`, `capabilities/default.json`, `scripts/*.ps1`, plus the six docs named in the brief.
All paths below are relative to the repo root; `nd.rs` abbreviates `src-tauri/src/native_dictation.rs`.

What was and was not verified:

- Everything here is from static reading. The app was not started and `cargo test` / `npm test` were not run, so no finding is confirmed at runtime.
- Third-party behaviour was checked against the vendored crate sources in the local cargo registry: reqwest 0.12.28 (proxy and redirect defaults), global-hotkey 0.8.0 (`MOD_NOREPEAT`), arboard 3.6.1 (clipboard exclusion API).
- Effort key: S = under half a day, M = 1 to 3 days, L = 1 to 2 weeks. Risk = regression risk of the fix.

---

## 0. Verdict

The live path works as a single-user prototype and has good safety instincts (session supersede guard, fail-closed target check, no transcript text in log statements, loopback URL as a compile-time constant). It is not production-ready, for four reasons:

1. **A hung or slow sidecar wedges a session with no timeout, no kill and no cancel** (P0-1).
2. **Two privacy leaks sit on the hot path**: the clipboard write is visible to Windows clipboard history and cloud sync (P0-2), and the Ollama HTTP client honours proxy environment variables and follows redirects (P0-3).
3. **The capture watchdog stops working exactly when capture breaks** (P0-4), leaving the session listening and Escape grabbed system-wide.
4. **Providers are not swappable.** The coordinator calls `whisper-cli.exe` and Ollama directly; the existing traits cannot express the live call and are only used by the mock commands.

---

## 1. Provider modularity

### 1.1 How hard is a swap today

Hard. There is no seam: the coordinator is the provider.

| Coupling | Evidence |
|---|---|
| Coordinator calls the whisper CLI function directly and passes it the `AppHandle` | `nd.rs:961`, `nd.rs:1562-1567` |
| Engine contract is "WAV on disk in, JSON file on disk out"; the coordinator owns both files | `nd.rs:945-950`, `nd.rs:957`, `nd.rs:963-964`, `nd.rs:1609` |
| Model file name hardcoded | `nd.rs:1970` |
| Language hardcoded to English | `nd.rs:1588-1589` |
| Refinement is a free function bound to one URL constant and the Ollama `/api/generate` wire format | `nd.rs:251`, `nd.rs:1698-1768` |
| Every error is a `String`, so the coordinator cannot tell "missing binary" from "timeout" from "bad output" and cannot decide whether to fall back | `nd.rs:1567`, `nd.rs:1702`, `nd.rs:1731` |
| Stage latency fields are named after the engines (`whisper_ms`, `ollama_cleanup_ms`) | `nd.rs:172`, `nd.rs:179` |
| No fallback chain: one engine, one error, session ends | `nd.rs:965` |

Adding a persistent `whisper-server` or Parakeet today means editing `process_session`, the latency snapshot, the diagnostics rows and the path-resolution helpers.

Settings that look like provider configuration are ignored by the live path. `ModelSettings.whisper_model_path`, `language` and `asr_threads`, and all of `MicrophoneSettings`, are declared at `app_state.rs:83-99` and never read outside `app_state.rs`. The live path reads only `dictionary`, `replacements`, `snippets`, `models.ollama_model` and `models.low_resource_mode` (`nd.rs:930-943`, `nd.rs:981-998`, `nd.rs:1018`). Two defaults contradict the code: `language: "auto"` (`app_state.rs:372`) against `-l en` with an English-only model, and `end_of_speech_ms: 900` (`app_state.rs:381`) against the 550 ms constant (`nd.rs:258`).

### 1.2 Are the existing traits usable as-is

No.

- **`AsrProvider`** (`src-tauri/src/asr/mod.rs:16-18`) is `fn transcribe(&self) -> Result<String, String>`. It takes no audio, no language, no keyterms, no deadline and no cancellation. Only `MockAsrProvider` implements it, and only `finish_mock_session` calls it (`app_state.rs:254-258`).
- **`WhisperCppProviderConfig`** (`asr/mod.rs:37-44`) has the right fields (model path, language, threads, prompt, timeout) and is never constructed.
- **`StreamingAsrProvider`** (`src-tauri/src/asr/streaming.rs:92-98`) is closer but not ready:
  - `&mut self` on every method means `cancel()` cannot be called while `push_audio()` or `commit()` is blocked on another thread, which is the only time cancel matters.
  - `start()` returns a provider-minted `SessionId`, competing with the coordinator's `SessionRegistry` (`nd.rs:50-74`).
  - Errors are `String`.
  - There is no lifecycle (`prepare` / `health` / `shutdown`) for a persistent sidecar.
  - There is no deadline.
  - The whole module is `#![allow(dead_code)]` (`streaming.rs:12`).
- **`RefinementProvider`** (`src-tauri/src/refinement/mod.rs:12-14`) takes only `&str`. The live call needs raw transcript, deterministic text and model (`nd.rs:1698-1702`). `OllamaRefinementProviderConfig` and `LlamaCppRefinementProviderConfig` (`refinement/mod.rs:54-66`) are never constructed.

The pieces worth keeping are `AsrCapabilities` and `AsrEvent` (`streaming.rs:20-89`), `TranscriptStabilizer`, `plan_rolling_windows` and `word_error_rate`.

### 1.3 Minimal abstraction

Trait signatures are in section 8.2. The shape, in short:

- **One batch trait** `AsrEngine::transcribe(&self, &AsrRequest) -> Result<Transcript, ProviderError>` taking in-memory 16 kHz mono PCM. The WAV temp file becomes a private detail of the CLI engine.
- **Streaming as an optional capability** on the same trait (`open_stream`, default "unsupported"), reusing `AsrEvent`.
- **One refiner trait** with a request struct carrying raw text, deterministic text and style.
- **A typed `ProviderError`** whose variants drive fallback and whose `Display` never carries dictated content.
- **Lifecycle on the trait** (`prepare`, `health`, `shutdown`) so a persistent engine is warmed at app start and torn down at exit.
- **`deadline` and `CancelToken` on every request**, so the coordinator can abort without knowing what the engine is.
- **A `FallbackChain`** built from settings: an ordered list of engine ids, a per-engine circuit breaker, fall through only on errors that are not `Cancelled`.
- **A `LoopbackEndpoint` newtype** that is the only way to give an HTTP engine a URL, so loopback-only is enforced at construction on the Rust side.

---

## 2. Concurrency and resource management

### 2.1 Thread and channel map

| Thread | Created at | Lifetime | Blocks on |
|---|---|---|---|
| Recorder loop | `nd.rs:277` | Process | Unbounded `mpsc` receive (`nd.rs:648`); WASAPI device open (`nd.rs:734-754`); health-file I/O |
| cpal audio callback | `nd.rs:1381-1416` | Per recording | `samples` mutex, plus heap allocation (`nd.rs:1393-1397`, `nd.rs:1426-1427`) |
| Level meter / VAD | `nd.rs:1161` | Per recording | 55 ms sleep; `samples` mutex; health-file I/O through `emit_native_event` |
| Session worker | `nd.rs:874` | Per recording | `Command::output()` with no timeout (`nd.rs:1598-1600`) |
| Background cleanup | `nd.rs:1110` | Per recording | Blocking reqwest, 20 s per attempt, 2 attempts (`nd.rs:1723`, `nd.rs:1704-1710`) |
| Ollama warm-up | `nd.rs:1771` | Per recording | Blocking reqwest, 12 s (`nd.rs:1784`) |
| Clipboard restore | `nd.rs:1884` | Per paste | 400 ms sleep |
| Overlay hide | `nd.rs:2080` | Per ready / inserted / error event | 700 ms sleep |
| Launch-signal watcher | `desktop_health.rs:148` | Process | 550 ms poll, forever (`desktop_health.rs:161-173`) |

Locks: `sessions`, `last_transcript`, `latency_snapshots`, `command_tx` (`nd.rs:34-38`), the per-recording `samples` and `last_voice_at`, and the global `HEALTH_LOCK` (`desktop_health.rs:20`). No lock is taken while another is held, so I found no deadlock. Poison is recovered almost everywhere; the exceptions are `command_tx` (`nd.rs:633`, `nd.rs:760`) and `samples` at stop (`nd.rs:841`).

### 2.2 Findings

**C1 (P0-1). The whisper sidecar has no timeout, no kill and no cancel.**
`run_whisper` uses `command.output()` (`nd.rs:1598-1600`). If the process hangs, the worker thread never returns. The session guard only checks after whisper returns (`nd.rs:971`), so supersede or cancel does not stop the process. Consequences:
- The overlay stays on "processing", because hide is only scheduled for `ready`, `inserted` and `error` (`nd.rs:2057`).
- The temp WAV of the user's speech stays on disk, because deletion follows the call (`nd.rs:963-964`).
- Each further recording spawns another worker and another whisper process; nothing caps the count.
- `docs/MODEL_SETUP.md` states that cancellation and timeout enforcement "are implemented". That is true only for the Ollama HTTP timeout.

**C2 (P1). Sidecars are orphaned on app exit.**
Quit is `app.exit(0)` (`lib.rs:108`). There is no `RunEvent` handler and no Job Object, so a running `whisper-cli.exe` outlives the app and keeps its WAV. The launcher's `Stop-Process -Force` (`scripts/Start-LocalFlow.ps1:133`) has the same effect.

**C3 (P0-4). The level meter skips its watchdog when no samples arrive.**
An empty chunk hits `continue` before `detector.observe()` runs (`nd.rs:1181-1183`, `nd.rs:1189`). `observe()` is the only place the 120 s cap and the 2.5 s no-speech timeout are enforced (`nd.rs:1238-1244`). If the device stops delivering callbacks (unplugged mid-recording, driver stall), then:
- the session stays in `listening` indefinitely;
- the stream error callback only logs (`nd.rs:1378`);
- Escape stays registered as a global shortcut (`nd.rs:791`), so Escape is swallowed in every application until the user presses the dictation hotkey again.

**C4 (P1). Recorder commands carry no session id.**
`RecorderCommand` is `{ app, state: String, received_at }` (`nd.rs:155-159`). The level meter of recording A can enqueue `auto_stop` or `cancel` after A has been finished by the hotkey, because it only checks its stop channel once per 55 ms loop (`nd.rs:1166-1169`, `nd.rs:1200-1205`). The stale command is then applied to whatever is current:
- a stale `cancel` invalidates A's in-flight worker (`nd.rs:686-696`), so the dictation is dropped;
- a stale `auto_stop` stops recording B if B started in between (`nd.rs:679-685`).
The window is a few milliseconds per recording, so this will be rare but not reproducible when it happens.

**C5 (P1). The health file is rewritten about 18 times per second while recording.**
`emit_native_event` calls `desktop_health::record_overlay_event` for every event, including each audio-feature event (`nd.rs:2044`, called from `nd.rs:1187` via `nd.rs:2021-2023`). Each call takes a global lock, reads and parses `desktop-health.json`, re-serialises it and rewrites it (`desktop_health.rs:282-302`). This runs on the VAD thread, so end-of-speech timing depends on disk and antivirus latency. The hotkey handler does the same I/O on the event-loop thread for every press and release (`hotkeys/mod.rs:60`) and contends for the same lock. The write is not atomic (`desktop_health.rs:300`). The same function also calls `window.show()` on every feature event (`nd.rs:2039`).

**C6 (P1). A superseded dictation is discarded with no recovery.**
Starting a new recording bumps the session (`nd.rs:772`). When the earlier worker's whisper call returns, it fails the check at `nd.rs:971-974` and returns before `store_last_transcript` (`nd.rs:1016`). A user who starts the next sentence while the previous one is still transcribing loses the previous sentence, and "Copy last transcript" cannot recover it. Cold-spawn whisper makes that window as long as the transcription itself.

**C7 (P2). Capture buffer.**
- Bounded only by the level-meter thread (120 s, `nd.rs:262`). See C3 for how that bound is lost.
- Pre-allocated for 20 s (`nd.rs:748`); beyond that it reallocates inside the audio callback.
- The callback allocates a `Vec` per buffer and takes a mutex (`nd.rs:1421-1428`).
- Stop clones the whole buffer (`nd.rs:839-842`), about 23 MB at 48 kHz for 120 s; resampling and WAV conversion make two more copies (`nd.rs:953`, `nd.rs:957`).

**C8 (P2). Temp files.**
`%TEMP%\localflow\dictation-<unix-ms>.wav` and `.json` (`nd.rs:945-950`). The JSON holds the transcript in plain text. Both are removed on the normal path only (`nd.rs:963-964`). There is no sweep at startup, so a crash, kill or C1 hang leaves speech and text on disk. `privacy.delete_audio_after_processing` (`app_state.rs:110`) is never read.

**C9 (P2). Ollama warm-up runs a real generation on every recording.**
`warm_ollama_in_background` is called at every record start (`nd.rs:792-802`), including when `low_resource_mode` is on and cleanup is skipped. It sends a prompt and waits for output (`nd.rs:1772-1784`), which competes with whisper for CPU on short dictations and keeps a model resident for 30 minutes (`nd.rs:263`) for users who disabled the LLM. A new `reqwest` client is also built per request (`nd.rs:1732-1735`).

**C10 (P2). Clipboard restore is unconditional.**
After a fixed 400 ms the previous text is written back (`nd.rs:1889-1894`) without checking the clipboard still holds the dictated text. Anything the user copied in that window is overwritten.

---

## 3. Fault tolerance on the live path

| Failure | Handling today | Surfaced | Recoverable | Evidence |
|---|---|---|---|---|
| No default microphone | Error returned | Yes, overlay error | Yes | `nd.rs:735-737` |
| Mic muted or blocked by Windows privacy (stream of zeros) | No-speech timeout sends `cancel` | **No.** Overlay just closes after 2.5 s | Yes, but undiagnosable | `nd.rs:1195-1199`, `nd.rs:1242-1244` |
| Same, key released between 0.7 s and 2.5 s | Near-silent error with device name | Yes | Yes | `nd.rs:863-868` |
| Device disconnect mid-recording | Stream error logged only; watchdog disabled (C3) | **No** | Only by pressing the hotkey again | `nd.rs:1378`, `nd.rs:1181-1183` |
| Recording under 250 ms | Error | Yes | Yes | `nd.rs:846-848` |
| whisper binary or model missing | Error listing the paths checked | Yes | Yes | `nd.rs:2001-2015` |
| whisper non-zero exit | Error with stderr | Yes | Yes | `nd.rs:1602-1607` |
| whisper hang | **None** (C1) | **No**, overlay stuck | **No**, thread and process leak | `nd.rs:1598-1600` |
| whisper JSON missing or malformed | Error | Yes | Yes | `nd.rs:1657-1676` |
| whisper blank or marker output | Error | Yes | Yes | `nd.rs:967-969`, `nd.rs:1679-1696` |
| whisper hallucination on noise | Not detected; pasted as text | No | Manual undo | n/a |
| Ollama down, slow or 404 | Background failure; deterministic text already inserted | Diagnostics row only | Yes | `nd.rs:1131-1143` |
| Ollama invalid JSON | One repair attempt, then failure | Diagnostics row only | Yes | `nd.rs:1706-1716` |
| Clipboard busy | Error; transcript already stored for recovery | Yes | Yes, "Copy last transcript" | `nd.rs:1016`, `nd.rs:1046`, `nd.rs:1859-1865` |
| Target window changed | Fail closed; transcript stored | Yes | Yes | `nd.rs:1036-1044` |
| No foreground window at record start | Records and transcribes, then refuses to paste | Only at the end | Yes | `nd.rs:729-732`, `nd.rs:116-123` |
| Focus moved to another field in the same window | Not detected (HWND and PID only) | No | No | `nd.rs:116-123` |
| Hotkey still held when paste fires | Not handled (F2 below) | No | No | `nd.rs:1911-1917` |
| Settings row fails to deserialise | Defaults used | Log only | **No**, next save overwrites the row | `lib.rs:52-54`, `storage/mod.rs:83-86` |
| `save_settings` fails | Frontend silently falls back to `localStorage` | **No** | **No** | `src/services/localflowClient.ts:86-96` |
| Hotkeys fail to register | Error pulse and health record | Yes | Restart | `hotkeys/mod.rs:109-114` |

**F1 (P1). Errors are visible for 700 ms.**
The overlay hide timer for `error` is the same 700 ms used for `inserted` (`nd.rs:2057-2058`, `nd.rs:2081`). Multi-sentence messages such as the near-silent diagnostic (`nd.rs:865`) or the focus-changed recovery hint (`nd.rs:1041`) cannot be read in a 593 x 91 pill in that time.

**F2 (P1). Paste does not account for held modifiers.**
`send_ctrl_v` injects Ctrl down, V down, V up, Ctrl up (`nd.rs:1911-1917`) without checking key state. In hold-to-talk, end-of-speech auto-stop fires after 550 ms of silence while the user may still be holding Ctrl+Alt+Space (`nd.rs:1195-1204`). The target then receives Ctrl+Alt+V, which is a different command in many applications. Not reproduced at runtime; the code has no guard.

**F3 (P1). Cancel is only possible while recording.**
Escape is unregistered as soon as recording stops (`nd.rs:836`). The `cancel_session` command only drives the mock reducer (`app_state.rs:336-346`). During transcription the only way to cancel is to start another recording. `SECURITY.md:17-18` describes a cancel command that supersedes in-flight workers; no UI-reachable command does that.

**F4 (P1). Settings schema has no forward compatibility.**
No settings struct carries `#[serde(default)]` (`app_state.rs:60-159`). Adding one field in a future release makes every existing `settings.v1` row fail to parse; the app starts on defaults (`lib.rs:52-54`) and the first save replaces the user's dictionary, replacements and snippets (`storage/mod.rs:96-105`).

**F5 (P1). The frontend hides command failures.**
`invokeOrFallback` catches every `invoke` error and runs the browser-preview fallback (`src/services/localflowClient.ts:86-96`). In the desktop app a failed `save_settings` therefore looks like a success and writes to `localStorage`, which the native path never reads.

**F6 (P2). Closing the settings window probably makes it unrecoverable.**
There is no close-requested handler (none found in `src-tauri/src`). The tray action looks the window up by label (`lib.rs:98-101`) and does nothing if it is gone. Needs a one-minute runtime check.

---

## 4. Observability

### 4.1 What exists

- **Logs.** `tracing` to stdout with a fixed filter, no timestamps (`lib.rs:28-31`). The release build is a GUI-subsystem binary (`src-tauri/src/main.rs:2`), so in production these lines go nowhere. There is no log file.
- **Stage timings.** A ring of the last 5 snapshots in memory (`nd.rs:162-181`, `nd.rs:267`), covering hotkey to recording start, VAD tail, capture stop, resample, WAV write, whisper, deterministic formatting, clipboard set, paste send, clipboard restore, speech end to visible, recording start to insert, and Ollama cleanup. Only the latest is exposed, as display strings (`nd.rs:317-324`).
- **Health file.** `desktop-health.json` in app data: hotkey registration, last hotkey event, last overlay event, last recording start, last recording error (`desktop_health.rs:22-36`).

### 4.2 Transcript leakage

No log statement prints transcript text; the insert log records a character count (`nd.rs:1092`). Three indirect paths remain:

- **L1 (P2). Serde error text.** `parse_cleanup_text` formats the `serde_json::from_value` error (`nd.rs:1792-1793`). Serde's "invalid type" messages quote the offending scalar, so a model reply that is a bare JSON string, or has a non-string `text`, puts model output into the error, which is logged at `nd.rs:1136-1141`. Ollama's `format: "json"` makes this unlikely, and release builds have no log sink today. It becomes real once a log file is added.
- **L2 (P2). Upstream bodies and stderr in errors.** The Ollama HTTP body is interpolated into errors (`nd.rs:1754-1760`); whisper stderr is interpolated (`nd.rs:1603-1606`) and that error is persisted to `desktop-health.json` (`nd.rs:892`, `desktop_health.rs:138-145`) and emitted to the webview.
- **L3 (P2). Local paths.** "Missing ... Checked: ..." errors carry absolute paths including the user name (`nd.rs:2014`) and are persisted the same way.

Events to the overlay carry phase, message and audio features only (`nd.rs:215-223`). `get_last_transcript` returns the transcript to any window that has the default capability, including the overlay (`nd.rs:406-409`, `capabilities/default.json:5`).

### 4.3 What is missing for reproducible benchmarks

- **Snapshots exist only for successful pastes.** The record call sits after the paste (`nd.rs:1050-1073`). Failures, cancels and focus-changed sessions leave no timing.
- **No engine identity in the snapshot**: no ASR engine id, model file, thread count or cold/warm flag.
- **No split of whisper time** into process spawn, model load and decode. That split is the number that justifies a persistent server.
- **No machine-readable export.** Values are formatted strings (`nd.rs:559-563`).
- **No offline harness.** The pipeline can only be driven by a real microphone and hotkey. `docs/PARITY_MATRIX.md:154` describes `src-tauri/src/bin/asr_bench.rs`; it does not exist. `word_error_rate` is ready (`src-tauri/src/asr/metrics.rs:15`) and a corpus is already on this machine under `.localflow-runtime/bench-corpus/` (untracked).
- **No persistent, redacted log** to attach to a bug report.

---

## 5. Security

**S1 (P0-2). Dictated text is exposed through the clipboard beyond the 400 ms paste window.**
- `clipboard.set_text` is called with no exclusion formats (`nd.rs:1863-1865`). With Windows clipboard history enabled the text is retained in history; with cloud clipboard sync enabled it is uploaded. That conflicts with the AGENTS.md rule of no cloud path for dictated content.
- Only text is saved and restored (`nd.rs:1861`, `nd.rs:1891-1895`). If the clipboard held an image or files, that content is destroyed and the transcript stays on the clipboard indefinitely.
- arboard 3.6.1, already in `Cargo.lock`, provides `exclude_from_history`, `exclude_from_cloud` and `exclude_from_monitoring` on Windows.
- Fix: S effort, low risk. Preserving non-text formats is M.

**S2 (P0-3). The Ollama client can send transcripts off the loopback interface.**
The URL is a constant (`nd.rs:251`), which is good, but the client is built with defaults (`nd.rs:1732-1735`):
- reqwest 0.12.28 applies `HTTP_PROXY` / `ALL_PROXY` from the environment unless `.no_proxy()` is called. Only `NO_PROXY` exempts a host; there is no implicit loopback exemption (checked in the vendored `hyper-util` matcher). On a machine with a proxy variable set, the POST containing the transcript goes to the proxy.
- The default redirect policy follows up to 10 redirects. Whatever answers on port 11434 can redirect the request, body included, to another host.
- Fix: `.no_proxy()` and `.redirect(Policy::none())`. S effort, low risk.

**S3 (P1). Nothing checks that the listener on 11434 is Ollama.**
Loopback ports are machine-wide. Any local process, including one owned by another user on a shared machine, can bind 11434 when Ollama is not running and receive every transcript. `docs/network-policy.md:19` says native integrations must call the shared policy; the Rust path has no policy function at all, only the constant. Longer-term fix: an app-supervised refiner on an ephemeral port with listener-PID verification (section 8.4).

**S4 (P1). Executable and model path resolution.**
- `LOCALFLOW_WHISPER_CLI` and `LOCALFLOW_WHISPER_MODEL` are honoured in release builds (`nd.rs:1946-1948`, `nd.rs:1962-1964`). Any process that can set a user environment variable can make LocalFlow run an arbitrary binary and hand it the user's speech.
- `workspace_runtime_dir()` compiles the build machine's absolute repo path into the binary through `env!("CARGO_MANIFEST_DIR")` (`nd.rs:1985-1991`) and searches it in production (`nd.rs:1981`). The release binary therefore contains the builder's user name and will execute `whisper-cli.exe` from that path if the bundled copy is absent.
- The sidecar runs with the inherited environment and with its own folder as working directory (`nd.rs:1575-1577`); no hash or signature check.
- Fix: gate both fallbacks behind `cfg(debug_assertions)`. S effort, low risk for installed builds; the repo-based launcher still works because resources are copied next to the release exe.

**S5 (P1). CSP is disabled and both windows share one capability.**
- `"csp": null` (`src-tauri/tauri.conf.json:44-46`).
- The overlay window, which only needs to listen to events, gets the same capability as the main window (`src-tauri/capabilities/default.json:5`), including `global-shortcut:allow-register` / `allow-unregister` and `opener:default` (`default.json:8-11`). No frontend code imports either plugin (searched `src`, `packages`, `tests`).
- All commands, including `get_last_transcript`, are callable from the overlay.
- Fix: set a CSP that allows `'self'`, the IPC scheme and `http://127.0.0.1:11434` for the Models screen check; split capabilities; drop the unused permissions. M effort, medium risk (CSP can break inline styles and the dev server; needs a UI pass).

**S6 (P2). Command surface.**
Seven commands (`lib.rs:71-79`). The three mock commands ship in production (`lib.rs:74-76`). `save_settings` accepts the whole settings object with no validation (`app_state.rs:212-223`): no length limits on dictionary terms, no check on the model name.

**S7 (P2, no injection found). The whisper `--prompt` argument.**
The prompt is passed as a separate argument through `Command::arg` (`nd.rs:1594-1596`), so there is no shell. It always starts with `Glossary: ` (`nd.rs:1637`), so a term cannot be read as a flag. Residual points:
- the cap counts bytes while the constant is named chars (`nd.rs:1615`, `nd.rs:1628`);
- control characters and newlines in terms are passed through;
- dictionary terms appear on the process command line, readable by other processes of the same user.

**S8 (P2). Temp files.**
`%TEMP%` is per-user, so other accounts cannot read the files. Names are predictable (`nd.rs:948-950`), creation is not exclusive, and there is no crash cleanup (C8).

**S9 (P2). Target identity.**
HWND plus PID (`nd.rs:116-123`) does not detect a different field in the same window, including a password field, and there is a small gap between the check (`nd.rs:1036`) and the paste (`nd.rs:1046`). `SECURITY.md:46-47` lists the stronger checks as planned.

**S10 (P2). No single-instance guard in the app.**
Only the launcher script de-duplicates (`scripts/Start-LocalFlow.ps1:117-130`). A second instance fails hotkey registration and both write the same health file and database.

---

## 6. Technical debt

### 6.1 Dead or mock-only Rust

| Item | Evidence |
|---|---|
| `AsrProvider`, `MockAsrProvider`, `WhisperCppProviderConfig` | `src-tauri/src/asr/mod.rs:16-44`; used only at `app_state.rs:254-258` |
| `StreamingAsrProvider`, `StreamingSession`, stabilizer, window planner, WER | `asr/streaming.rs:12` (`allow(dead_code)`), `asr/stabilizer.rs`, `asr/windows.rs`, `asr/metrics.rs` |
| `RefinementProvider` mocks and two unused config structs | `src-tauri/src/refinement/mod.rs:12-66` |
| `AudioCapture`, `TextInserter`, `ContextProvider`, `StartupRegistration` (trait plus mock or no-op only) | `src-tauri/src/audio/mod.rs`, `insertion/mod.rs`, `context/mod.rs`, `platform/mod.rs` |
| Workflow reducer, driven only by mock commands | `src-tauri/src/workflow/mod.rs:158`; `app_state.rs:226-346` |
| `dictation_history` and `dictionary_entries` tables, never read or written | `src-tauri/src/storage/mod.rs:32-47` |
| `refining` overlay phase, never emitted | `nd.rs:2038` |
| Settings fields never read by the live path | `app_state.rs:75-111` (hotkeys, three model fields, all microphone fields, all privacy fields) |

### 6.2 Rust and TypeScript duplication

- **State machine, three versions**: `src/domain/stateMachine.ts`, `src-tauri/src/workflow/mod.rs`, and the string phases in `nd.rs:2037-2042`.
- **Cleanup prompt contract**: `src/domain/refinementPipeline.ts` and `nd.rs:1819-1840`.
- **Deterministic formatting**: `src/domain/personalization.ts` and `src-tauri/src/transcript/mod.rs`.
- **Whisper argument and prompt building**: `src/domain/whisperSidecar.ts` and `nd.rs:1575-1596`, `nd.rs:1614-1639`.
- **VAD, resampling, ring buffer**: `src/domain/audio.ts` and `nd.rs:1217-1273`, `nd.rs:1516-1538`.
- **Stabilizer and window planner**: `src/domain/transcriptStabilizer.ts`, `src/domain/asrWindows.ts` and `src-tauri/src/asr/stabilizer.rs`, `asr/windows.rs`.
- **Network policy**: enforced in `src/domain/networkPolicy.ts`; the Rust side has only a constant.

Eight TypeScript domain modules have no importer outside `tests/`: `whisperSidecar`, `asrWindows`, `audio`, `insertionPlan`, `transcriptStabilizer`, `context`, `commandMode`, `performance` (searched `src` and `packages` for imports).

### 6.3 Unused dependencies (confirmed by search)

| Dependency | Where declared | Finding |
|---|---|---|
| `thiserror` | `src-tauri/Cargo.toml:33` | No reference anywhere in `src-tauri/src` |
| `tauri-plugin-opener` | `src-tauri/Cargo.toml:28`, initialised at `lib.rs:34` | No frontend or Rust caller |
| `@tauri-apps/plugin-opener` | `package.json:27` | No import in `src`, `packages`, `tests` |
| `@tauri-apps/plugin-global-shortcut` | `package.json:26` | No import in `src`, `packages`, `tests` (the Rust plugin is used) |

`regex`, `hound`, `arboard`, `cpal`, `rusqlite`, `chrono`, `reqwest`, `lucide-react` and `@tauri-apps/api` are all in use. I did not audit the devDependencies individually.

### 6.4 Stale documentation

- `docs/MODEL_SETUP.md`: says cancellation and timeout enforcement are implemented (see C1), and that the cleaned text is inserted (the deterministic text is inserted; cleanup runs afterwards, `nd.rs:1046`, `nd.rs:1078-1086`).
- `docs/ARCHITECTURE.md:22-27`: describes cleanup before paste; `ARCHITECTURE.md:29-33` presents the reducer phases as the workflow.
- `SECURITY.md:9`: "Validate all local model responses before insertion"; model output is never inserted. `SECURITY.md:22-23` and `docs/network-policy.md:19`: the shared policy is not on the Rust path.
- `docs/development/PLAN.md:229`: still says recording start warms `gemma4:12b-it-qat`. `PLAN.md:152` lists target-window verification as pending although Slice 2 is done.
- `docs/REPO_AUDIT.md`: line references are to the 1,330-line version of the file.
- `scripts/Set-WhisperModelPath.ps1:12-19` writes `%APPDATA%\LocalFlow\model-path.json`; nothing reads that file.
- `docs/PARITY_MATRIX.md:154` refers to a benchmark binary that does not exist.

---

## 7. Deployment

**D1 (P1). The installer bundles every executable in the whisper build folder.**
The resource globs are `*.exe` and `*.dll` (`src-tauri/tauri.conf.json:52-53`). On this machine that folder holds about 25 executables besides `whisper-cli.exe`, including `whisper-server.exe`, `whisper-talk-llama.exe`, `wchess.exe`, `parakeet-cli.exe`, test binaries and `SDL2.dll`. All of them ship. List the required files explicitly.

**D2 (P1). The build is not reproducible from the repository.**
`.localflow-runtime/` is git-ignored (`.gitignore`) and no script fetches or verifies it. `docs/MODEL_SETUP.md` says "this workstation has" the runtime. `npm run tauri:build` on a clean clone fails on the missing resources. There is no manifest of versions or checksums for the whisper build or the model.

**D3 (P1). Model distribution is one English-only tiny model.**
`ggml-tiny.en-q5_1.bin` is bundled (`tauri.conf.json:54`) and hardcoded (`nd.rs:1970`). Larger models already on this machine (`base.en`, `small.en`, `large-v3-turbo`) cannot be selected without an environment variable. There is no model picker, no import flow and no integrity check.

**D4 (P2). The documented "normal usage" runs from the repository.**
`scripts/Start-LocalFlow.ps1:109-110` starts `src-tauri\target\release\localflow.exe` and builds it if missing; `Start-LocalFlow.vbs` and `package.json:10` run PowerShell with `-ExecutionPolicy Bypass`. The MSI and NSIS outputs exist (`docs/development/PLAN.md:137-140`) but the scripts and the boot test (`scripts/Test-DesktopBoot.ps1`) assume a repo checkout.

**D5 (P2). Packaging gaps.**
- No code-signing configuration and no Ollama prerequisite check in the installer.
- No start-at-login (`src-tauri/src/platform/mod.rs` is a no-op).
- No LICENSE or third-party notices at the repo root, although whisper.cpp binaries and SDL2 are redistributed.
- No CI configuration (`.github` absent).
- `targets: "all"` (`tauri.conf.json:50`) for a Windows-only product.
- `scripts/Install-Prereqs.ps1:66` only prints a reminder about WebView2.

---

## 8. Target architecture

### 8.1 Module layout

```
src-tauri/src/
  dictation/
    mod.rs            // DictationCoordinator: owns the session, drives the phases, no engine knowledge
    session.rs        // SessionRegistry + typed Phase enum (reuse workflow::transition)
    command.rs        // enum RecorderCommand { Pressed, Released, AutoStop{session}, Cancel{session, reason} }
    pipeline.rs       // capture -> asr chain -> format -> insert -> optional refine; pure over traits
  audio/
    capture.rs        // cpal stream, bounded buffer, device-loss detection
    vad.rs            // EndOfSpeechDetector (moved as-is, tests included)
    features.rs       // level / pitch / brightness for the overlay
    resample.rs  wav.rs
  asr/
    mod.rs            // AsrEngine, AsrStream, AsrRequest, Transcript, AsrCapabilities
    chain.rs          // FallbackChain + circuit breaker
    whisper_cli.rs    // today's one-shot path, behind the trait
    whisper_server.rs // persistent whisper-server.exe over loopback HTTP
    parakeet_cli.rs   // optional
    streaming.rs  stabilizer.rs  windows.rs  metrics.rs   // existing
  refinement/
    mod.rs            // Refiner, RefineRequest, Refinement
    prompt.rs         // cleanup + repair prompts (single source of truth)
    ollama.rs  noop.rs  llama_server.rs
  insertion/
    mod.rs            // Inserter, InsertTarget
    target.rs         // foreground capture + revalidation
    clipboard_paste.rs// clipboard exclusion formats, modifier release, verified restore
  sidecar/
    job.rs            // Win32 Job Object, kill-on-close
    oneshot.rs        // run_with_deadline(cmd, deadline, cancel)
    supervisor.rs     // persistent child: spawn, ready probe, health, backoff restart, shutdown
  net/loopback.rs     // LoopbackEndpoint + hardened HTTP client factory
  telemetry/
    latency.rs        // StageTimings recorded for every session outcome
    log.rs            // rotating file sink + redaction layer
  overlay/mod.rs      // show / hide / position, event emission, throttled health
  error.rs            // ProviderError, DictationError (thiserror)
  bin/asr_bench.rs    // offline harness: corpus WAV -> chain -> CSV (latency, WER)
```

### 8.2 Traits

```rust
// error.rs
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("{engine} is not available: {reason}")]
    Unavailable { engine: EngineId, reason: UnavailableReason }, // MissingBinary, MissingModel, NotRunning
    #[error("{engine} timed out after {elapsed_ms} ms")]
    Timeout { engine: EngineId, elapsed_ms: u64 },
    #[error("{engine} was cancelled")]
    Cancelled { engine: EngineId },
    #[error("{engine} exited abnormally (code {code:?})")]
    Crashed { engine: EngineId, code: Option<i32> },
    #[error("{engine} returned output that did not match the contract ({kind})")]
    InvalidOutput { engine: EngineId, kind: InvalidOutputKind }, // enum, never the payload
    #[error("{engine} I/O failure: {kind:?}")]
    Io { engine: EngineId, kind: std::io::ErrorKind },
}
impl ProviderError {
    pub fn should_fall_back(&self) -> bool { !matches!(self, Self::Cancelled { .. }) }
    pub fn trips_breaker(&self) -> bool { matches!(self, Self::Unavailable { .. } | Self::Crashed { .. } | Self::Timeout { .. }) }
}

// Shared by every request.
#[derive(Clone)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken { pub fn cancel(&self); pub fn is_cancelled(&self) -> bool; }

// asr/mod.rs
pub struct PcmAudio { pub samples: Arc<[f32]>, pub sample_rate: u32 }   // mono, engine input is 16 kHz

pub struct AsrRequest<'a> {
    pub session_id: SessionId,
    pub audio: &'a PcmAudio,
    pub language: Option<&'a str>,       // None = engine default / auto
    pub keyterms: &'a [String],
    pub deadline: Instant,
    pub cancel: &'a CancelToken,
}

pub struct Transcript {
    pub text: String,
    pub segments: Vec<AsrSegment>,
    pub engine: EngineId,
    pub timings: AsrTimings,             // spawn_ms, model_load_ms, decode_ms (each Option<u64>)
}

pub enum EngineHealth { Ready, Warming, Degraded(UnavailableReason), Down(UnavailableReason) }

pub trait AsrEngine: Send + Sync {
    fn id(&self) -> EngineId;
    fn capabilities(&self) -> AsrCapabilities;                          // existing struct
    fn prepare(&self) -> Result<(), ProviderError>;                     // load model / start sidecar; idempotent
    fn health(&self) -> EngineHealth;                                   // cheap, non-blocking
    fn transcribe(&self, request: &AsrRequest<'_>) -> Result<Transcript, ProviderError>;
    fn open_stream(&self, _config: AsrSessionConfig, _events: mpsc::Sender<AsrEvent>, _cancel: CancelToken)
        -> Result<Box<dyn AsrStream>, ProviderError> {
        Err(ProviderError::Unavailable { engine: self.id(), reason: UnavailableReason::StreamingUnsupported })
    }
    fn shutdown(&self);                                                 // called on app exit
}

pub trait AsrStream: Send {
    fn push_audio(&mut self, pcm: &[f32]) -> Result<(), ProviderError>;
    fn finish(self: Box<Self>, deadline: Instant) -> Result<Transcript, ProviderError>;
}

// refinement/mod.rs
pub struct RefineRequest<'a> {
    pub session_id: SessionId,
    pub raw_transcript: &'a str,
    pub deterministic_text: &'a str,
    pub style: Option<&'a StyleProfile>,
    pub deadline: Instant,
    pub cancel: &'a CancelToken,
}
pub struct Refinement { pub text: String, pub engine: EngineId, pub elapsed_ms: u64, pub repaired: bool }

pub trait Refiner: Send + Sync {
    fn id(&self) -> EngineId;
    fn prepare(&self) -> Result<(), ProviderError>;                     // load-only warm-up, no generation
    fn health(&self) -> EngineHealth;
    fn refine(&self, request: &RefineRequest<'_>) -> Result<Refinement, ProviderError>;
    fn shutdown(&self);
}

// insertion/mod.rs
pub trait Inserter: Send + Sync {
    fn capture_target(&self) -> Option<InsertTarget>;
    fn insert(&self, target: &InsertTarget, text: &str) -> Result<InsertReceipt, InsertError>;
}

// net/loopback.rs
pub struct LoopbackEndpoint(url::Url);
impl LoopbackEndpoint {
    pub fn parse(raw: &str) -> Result<Self, NetPolicyError>;            // host must be an IP literal with is_loopback()
    pub fn client(&self, timeout: Duration) -> reqwest::blocking::Client; // .no_proxy(), redirect::Policy::none()
}
```

Design notes:

- `&self` plus interior state lets one engine instance be shared across the worker and the supervisor, and lets `cancel` work while `transcribe` is blocked.
- The coordinator keeps owning `SessionId`; engines receive it for logging only.
- `ProviderError` has no free-text field, so it can be logged and persisted without redaction.
- `LoopbackEndpoint::parse` rejects host names, including `localhost`, so there is no DNS step to subvert.

### 8.3 Fallback chain

```rust
pub struct FallbackChain<E: ?Sized> { links: Vec<Link<E>> }
struct Link<E: ?Sized> { engine: Arc<E>, breaker: CircuitBreaker }      // open after N failures, half-open after cooldown

impl FallbackChain<dyn AsrEngine> {
    pub fn transcribe(&self, request: &AsrRequest<'_>) -> Result<Transcript, ChainError>;
}
```

Rules:

- Order comes from settings, for example `asr.chain = ["whisper-server", "whisper-cli"]` and `refine.chain = ["ollama", "noop"]`, with `#[serde(default)]` on every field.
- An engine whose breaker is open or whose `health()` is `Down` is skipped without being called.
- `Cancelled` stops the chain. Any other error records the failure and tries the next link if the deadline allows.
- `ChainError` carries the per-engine errors, so the overlay can say which engine failed and why.
- The registry of engine constructors contains local engines only. A remote engine cannot be configured, because every HTTP engine takes a `LoopbackEndpoint`.
- The fallback is visible: the latency snapshot records which engine answered, and diagnostics show breaker state.

### 8.4 Sidecar supervisor

One-shot runner (replaces `Command::output()`):

```rust
pub fn run_with_deadline(mut command: Command, deadline: Instant, cancel: &CancelToken, job: &Job)
    -> Result<Output, SidecarError>;
```

- Spawn with piped stdout and stderr, assign the child to the Job Object, drain both pipes on helper threads into bounded buffers.
- Poll `try_wait` every 10 to 20 ms against `deadline` and `cancel`; on either, `child.kill()` then `wait()`.
- Temp files are created through a guard type that deletes on drop, so every exit path cleans up.

Process-wide job:

- One Job Object created at startup with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. Every sidecar is assigned to it. When LocalFlow exits for any reason, including a crash or `Stop-Process -Force`, Windows terminates the children.
- A startup sweep deletes leftovers in `%TEMP%\localflow`.

Persistent supervisor (for `whisper-server.exe` or a llama.cpp server):

```rust
pub struct SidecarSupervisor { /* spec, state: Mutex<State>, job: Arc<Job> */ }
pub struct SidecarSpec {
    pub program: PathBuf, pub args: Vec<OsString>, pub working_dir: PathBuf,
    pub ready_probe: ReadyProbe,          // HTTP GET on the loopback endpoint, or a stdout marker
    pub ready_timeout: Duration,
    pub restart: RestartPolicy,           // exponential backoff, max attempts per window
}
impl SidecarSupervisor {
    pub fn ensure_running(&self) -> Result<LoopbackEndpoint, ProviderError>;  // lazy start, returns endpoint
    pub fn health(&self) -> EngineHealth;
    pub fn restart(&self, reason: RestartReason);
    pub fn shutdown(&self);
}
```

- Bind to `127.0.0.1` on a port chosen by the supervisor (reserve an ephemeral port, pass it to the child).
- After the ready probe, confirm that the PID owning the listening port is the child's PID before sending any audio. That closes the port-squatting gap in S3 for app-owned sidecars.
- A watcher thread waits on the child; an unexpected exit marks the engine `Down`, trips the breaker and schedules a restart with backoff. The chain serves the next engine in the meantime.
- Binary and model paths come only from the bundled resource directory in release builds, with SHA-256 values from a build-time manifest checked in `prepare()`.

### 8.5 Incremental migration (each step leaves the app working)

| Step | Change | Covers | Effort | Risk |
|---|---|---|---|---|
| 1 | Hot fixes inside `native_dictation.rs`: whisper deadline and kill; `.no_proxy()` and no redirects; clipboard exclusion formats; watchdog runs on empty chunks; debug-only env and workspace fallbacks | P0-1 to P0-4, S4 | M | Low |
| 2 | Small correctness fixes: session id on recorder commands; store transcript before the supersede check; release modifiers before paste; throttle health writes; `#[serde(default)]` on settings; stop swallowing `invoke` errors | C4, C6, F2, C5, F4, F5 | M | Low to medium |
| 3 | Add `sidecar::job` and `sidecar::oneshot`; exit handler; temp-file guard and startup sweep | C2, C8 | M | Low |
| 4 | Add `error.rs`, `net::loopback`, new `AsrEngine` / `Refiner` traits; wrap today's code as `WhisperCliEngine` and `OllamaRefiner`; coordinator calls the traits. Pure refactor, same behaviour | Section 1 | M | Medium |
| 5 | `FallbackChain` with a single link each; provider config in settings; wire `whisper_model_path`, `language`, `asr_threads` | Section 1, D3 | M | Low |
| 6 | Latency snapshot for every outcome, engine id and split timings, JSON export; `bin/asr_bench.rs` over the local corpus | Section 4 | M | Low |
| 7 | Split the remaining concerns out of `native_dictation.rs` into `audio/`, `insertion/`, `overlay/`, `dictation/`; typed phase enum from `workflow` | Section 6 | L | Medium |
| 8 | `SidecarSupervisor` and `WhisperServerEngine` as first link, CLI as fallback; measure with step 6 before changing the default | C1, latency | L | Medium |
| 9 | CSP and capability split; remove mock commands from release builds; remove unused dependencies; explicit bundle list; runtime manifest with checksums | S5, S6, D1, D2 | M | Medium |
| 10 | Delete or relocate the unused TypeScript domain modules and the superseded Rust mocks; refresh docs | Section 6 | M | Low |

---

## 9. Prioritised findings

### P0: bug or safety issue

| ID | Finding | Evidence | Effort | Risk |
|---|---|---|---|---|
| P0-1 | Whisper sidecar has no timeout, kill or cancel; a hang leaves the overlay on "processing", leaks a thread and process, and leaves the speech WAV on disk | `nd.rs:1598-1600`, `nd.rs:971`, `nd.rs:2057`, `nd.rs:963-964` | M | Low |
| P0-2 | Dictated text is written to the clipboard without history / cloud exclusion; non-text clipboard content is destroyed and the transcript is left behind | `nd.rs:1861-1865`, `nd.rs:1891-1895` | S (exclusion), M (format preservation) | Low |
| P0-3 | Ollama client honours proxy environment variables and follows redirects, so a transcript can leave loopback | `nd.rs:1732-1735`, `nd.rs:251` | S | Low |
| P0-4 | Level meter skips the max-duration and no-speech checks when no samples arrive; device loss leaves the session listening and Escape grabbed globally | `nd.rs:1181-1183`, `nd.rs:1238-1244`, `nd.rs:1378`, `nd.rs:791` | S | Low |

### P1: high value

| ID | Finding | Evidence | Effort | Risk |
|---|---|---|---|---|
| P1-1 | No provider seam; existing traits cannot express the live call | `asr/mod.rs:16-18`, `asr/streaming.rs:92-98`, `refinement/mod.rs:12-14`, `nd.rs:961`, `nd.rs:1112` | M to L | Medium |
| P1-2 | Superseded dictation is dropped before it is stored for recovery | `nd.rs:971-974`, `nd.rs:1016` | S | Low |
| P1-3 | Recorder commands have no session id; stale `auto_stop` / `cancel` can hit the wrong session | `nd.rs:155-159`, `nd.rs:679-696`, `nd.rs:1200-1205` | S | Low |
| P1-4 | Health file read and rewritten about 18 times per second on the VAD thread, and on the event loop per hotkey event | `nd.rs:2044`, `desktop_health.rs:282-302`, `hotkeys/mod.rs:60` | S | Low |
| P1-5 | Paste ignores held modifiers; hold-to-talk plus auto-stop sends Ctrl+Alt+V | `nd.rs:1911-1917`, `nd.rs:1195-1204` | S | Medium |
| P1-6 | Sidecars orphaned on exit; no Job Object or exit handler | `lib.rs:108`, `nd.rs:1641-1648` | M | Low |
| P1-7 | Release build runs executables from environment overrides and from the build machine's repo path | `nd.rs:1946-1948`, `nd.rs:1962-1964`, `nd.rs:1981-1991` | S | Low |
| P1-8 | Nothing verifies the listener on 11434; any local process can collect transcripts | `nd.rs:251`, `nd.rs:1737-1747` | L (supervised refiner) | Medium |
| P1-9 | CSP disabled; overlay shares the main capability, including unused global-shortcut and opener permissions | `tauri.conf.json:44-46`, `capabilities/default.json:5-11` | M | Medium |
| P1-10 | Settings have no serde defaults; a schema change resets and then overwrites user data | `app_state.rs:60-159`, `lib.rs:52-54`, `storage/mod.rs:83-86` | S | Low |
| P1-11 | Frontend swallows command errors and reports success from the browser fallback | `src/services/localflowClient.ts:86-96` | S | Low |
| P1-12 | Cancel only works while recording; no UI-reachable native cancel | `nd.rs:836`, `app_state.rs:336-346` | S | Low |
| P1-13 | Muted or privacy-blocked microphone gives a silent close; errors shown for 700 ms | `nd.rs:1195-1199`, `nd.rs:2057-2058`, `nd.rs:2081` | S | Low |
| P1-14 | No production log sink; latency recorded only for successful pastes; no benchmark harness | `lib.rs:28-31`, `main.rs:2`, `nd.rs:1050-1073` | M | Low |
| P1-15 | Installer bundles about 25 unneeded executables; runtime not reproducible from the repo; single hardcoded English model | `tauri.conf.json:52-54`, `nd.rs:1970`, `.gitignore` | M | Low |

### P2: nice to have

| ID | Finding | Evidence | Effort | Risk |
|---|---|---|---|---|
| P2-1 | Ollama warm-up runs a generation on every recording, including in low-resource mode; new HTTP client per request | `nd.rs:792-802`, `nd.rs:1770-1788`, `nd.rs:1732` | S | Low |
| P2-2 | Background cleanup result is never inserted; it only replaces the "copy last" text, which then differs from what was pasted | `nd.rs:1116-1119` | Product decision | n/a |
| P2-3 | Serde error text and upstream bodies can carry model output into logs and the health file | `nd.rs:1792-1793`, `nd.rs:1754-1760`, `nd.rs:1603-1606`, `nd.rs:892` | S | Low |
| P2-4 | Temp files: predictable names, no crash sweep, transcript JSON on disk; `delete_audio_after_processing` unread | `nd.rs:945-950`, `app_state.rs:110` | S | Low |
| P2-5 | Clipboard restore is unconditional after a fixed 400 ms | `nd.rs:1889-1894` | S | Low |
| P2-6 | Audio callback allocates and locks; buffer preallocated for 20 s; three full copies at stop | `nd.rs:748`, `nd.rs:1421-1428`, `nd.rs:839-842` | M | Medium |
| P2-7 | Mock commands and mock providers ship in release | `lib.rs:74-76`, `app_state.rs:226-346` | S | Low |
| P2-8 | Unused dependencies: `thiserror`, `tauri-plugin-opener`, two npm plugin packages | `Cargo.toml:28`, `Cargo.toml:33`, `package.json:26-27` | S | Low |
| P2-9 | Eight TypeScript domain modules and most Rust mock modules are test-only duplicates | Section 6 | M | Low |
| P2-10 | Stale docs and a script that writes a file nothing reads | Section 6.4 | S | None |
| P2-11 | No single-instance guard; settings window may not be reopenable after close | `lib.rs:98-101`, `scripts/Start-LocalFlow.ps1:117-130` | S | Low |
| P2-12 | Target identity is HWND plus PID only | `nd.rs:116-123` | L (UI Automation) | Medium |
| P2-13 | `save_settings` accepts unvalidated input; whisper prompt cap counts bytes | `app_state.rs:212-223`, `nd.rs:1615-1628` | S | Low |
| P2-14 | No signing, licence notices, CI or start-at-login | Section 7 | M | Low |
