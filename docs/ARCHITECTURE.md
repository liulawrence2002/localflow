# Architecture

LocalFlow is split into a Tauri native layer and a React settings/history interface.

## Native Layer

- `src-tauri/src/workflow`: explicit dictation state machine. **Note:** this reducer is
  currently exercised only by the mock-session Tauri commands, not by the live hotkey path.
  See `REPO_AUDIT.md` §2 for the three current session representations and the plan to
  unify them.
- `src-tauri/src/audio`: microphone capture interface and mock capture, plus the audio helpers on the live path: `resample.rs` (band-limited windowed-sinc resampling to 16 kHz) and `wav.rs` (in-memory 16-bit PCM WAV encoding).
- `src-tauri/src/asr`: the speech-to-text provider boundary used by the live path. `provider.rs` defines `SpeechToTextProvider`, the typed `AsrError`, the per-utterance deadline, and `FallbackProvider`. Engines: `whisper_server.rs` (persistent `whisper-server.exe`, model kept loaded, loopback only), `whisper_cli.rs` (one process per utterance; fallback and low-resource engine), and `parakeet_cli.rs` (opt-in Parakeet TDT accuracy engine). `sidecar.rs` holds hidden-process launch, the Windows kill-on-close job object, and memory measurement. The older `AsrProvider` mock trait and the streaming foundation remain for the mock sessions and future streaming work.
- `src-tauri/src/refinement`: refinement provider trait, mock provider, no-op provider, and future native Ollama/llama.cpp configs.
- `src-tauri/src/context`: local context snapshot interface.
- `src-tauri/src/insertion`: text insertion interface and mock insertion.
- `src-tauri/src/hotkeys`: global hotkey registration through Tauri's global-shortcut plugin.
- `src-tauri/src/native_dictation`: the live Windows native push-to-talk path using `cpal`, a local speech-to-text engine chosen through `SpeechToTextProvider`, a configurable local Ollama cleanup model (default `llama3.2:3b`), a waveform overlay event stream, and clipboard paste fallback. Carries a native `SessionRegistry` (session identity + supersede/cancel guard); the transcribe/refine/insert tail runs on a worker thread and revalidates the session before every side effect so a superseded or cancelled session never inserts.
- `src-tauri/src/transcript`: **authoritative** deterministic formatting for the native path — spoken punctuation, explicit self-corrections, filler/stutter cleanup, sentence capitalization, user replacements/snippets, and URL/email/decimal protection. Runs before the LLM and is the fallback when the LLM is unavailable.
- `src-tauri/src/storage`: SQLite schema initialization.
- `src-tauri/src/privacy`: redaction helpers for logs and diagnostics.

The native dictation pipeline is: capture (`cpal`, one locked channel per recording) →
band-limited resample to 16 kHz → local speech-to-text through the provider chain (dictionary
terms bias Whisper through its prompt) → deterministic formatting (`transcript`) seeded from
the user's settings (replacements, snippets) → session + target-window revalidation →
clipboard paste (excluded from clipboard history and cloud sync) → background local Ollama
cleanup with the configured model. Each recording carries a session id; the transcribe/refine/
insert tail runs on a worker thread that never inserts for a superseded, cancelled
(Escape), or focus-changed session, and the last transcript is recoverable.

The workflow is state-driven:

`Idle -> Preparing -> Listening -> Transcribing -> Refining -> Inserting -> Complete`

Cancellation and errors are terminal states until reset. Overlapping sessions are rejected with immediate warning state.

## Frontend Layer

- `src/domain`: shared state machine, deterministic cleanup, transcript stabilization, insertion spacing, and response validation.
- `src/domain/audio.ts`: bounded PCM buffers, RMS/VAD, mono downmixing, and linear resampling helpers.
- `src/domain/asrWindows.ts`: rolling-window planning for overlapped ASR hypotheses.
- `src/domain/whisperSidecar.ts`: shared `whisper.cpp` command planning, vocabulary prompt building, and JSON transcript parsing.
- `src/domain/refinementPipeline.ts`: strict local-LLM JSON contract runner with repair retry and deterministic fallback.
- `src/domain/context.ts`: context snapshot privacy policy, application categorization, and cleanup input mapping.
- `src/domain/privacy.ts`: history retention and redaction helpers.
- `src/domain/diagnostics.ts`: redacted diagnostics export assembly for support bundles.
- `src/domain/performance.ts`: latency and peak-memory measurement helpers with diagnostic formatting.
- `src/domain/settings.ts`: typed personalization and style-profile mutations used by the UI and local fallback.
- `src/domain/commandMode.ts`: selected-text command planning, OS-command rejection, preview decisions, and command response parsing.
- `src/domain/insertionPlan.ts`: target validation, insertion method ordering, clipboard fallback restoration timing, and duplicate insertion guards.
- `src/domain/undo.ts`: deterministic/raw restore helpers for undoing cleanup.
- `src/domain/networkPolicy.ts`: local-only network allowlist for ordinary dictation provider URLs.
- `src/domain/ollama.ts`: shared local Ollama discovery and cleanup provider with localhost-only enforcement.
- `src/services/localflowClient.ts`: Tauri command adapter with a browser fallback for frontend development.
- `src/components`: focused UI components.
- `src/components/VoiceOverlay.tsx`: compact waveform-only overlay rendered in the Tauri `overlay` window. Subscribes to `localflow://native-dictation`; phase changes flow through React state while ~18 Hz audio features flow through a ref polled by the renderer, so React never re-renders at microphone-event frequency.
- `packages/localflow-sdk/src/react/topology/`: the topographic waveform renderer behind `LocalFlowOverlay`. `topologyMath.ts` (deterministic surface heights + oblique projection), `audioFeatureSmoothing.ts` (frame-rate-independent attack/release smoothing, bounded level history), `overlayStateVisuals.ts` (dictation phase → visual parameters, reduced-motion variants), and `topologyRenderer.ts` (single-rAF Canvas 2D loop with DPI handling, visibility pause, and full listener cleanup). The renderer contains no dictation business logic.
- `src/App.tsx`: settings, status, history, diagnostics, and mock workflow controls.

## Provider Boundary

Providers are intentionally narrow:

- ASR accepts mono 16 kHz samples, a language, and an optional vocabulary prompt, and returns text plus the engine that produced it. Every engine enforces a deadline and reports a typed error; a failure falls back to another local engine, never to a remote one.
- Refinement accepts compact dictation-cleanup inputs and returns strict JSON.
- Context providers never return password/protected-field content.
- Inserters must verify the intended target before writing text.

## Persistence

SQLite is initialized under the Tauri app data directory. Milestone 1 creates tables for settings, dictation history, and dictionary entries. Later milestones will add migrations and retention jobs.

## Speech-to-Text Engines

`plan_asr_engine` in `native_dictation.rs` decides the engine chain per dictation from the
resolved model and settings; the engine is reused until that plan changes:

| Condition | Engine chain |
|---|---|
| Model is a `.gguf` file (default: Parakeet TDT 0.6B v3) | `transcribe-cpp` in-process, kept loaded → `whisper-cli` with tiny.en if installed |
| Same, with low-resource mode and a Whisper model installed | `whisper-cli` only; nothing stays loaded between dictations |
| Model is a `ggml-*.bin` Whisper file | `whisper-server` (persistent) → `whisper-cli` (same model) |
| Same, with low-resource mode or `whisper-server.exe` missing | `whisper-cli` only |

Model resolution order: `LOCALFLOW_WHISPER_MODEL`, then Settings > Models > Whisper model
path (if the file exists), then the first installed model of
`parakeet-tdt-0.6b-v3-Q5_K_M.gguf` and `ggml-tiny.en-q5_1.bin`.

The in-process engine (`asr/transcribe_cpp.rs`) loads the model when recording starts, so it
is warm by the end of the utterance, and holds one session behind a mutex. A watchdog cancels
a run that passes its deadline through the library's abort callback. A failed run drops the
session so the next dictation reloads it.

The whisper server sidecar listens on `127.0.0.1` on an ephemeral port, receives audio as an
in-memory WAV, is restarted if it exits, is killed if a request exceeds its deadline, and is
stopped on app exit. On Windows a kill-on-close job object also stops it if LocalFlow
crashes, and the listener's owning process is verified before audio is sent.

To add an engine, implement `SpeechToTextProvider` and return it from `build_asr_provider`
in `native_dictation.rs`. Capture, formatting, and insertion do not change. Measurements
behind the defaults are in `benchmarks/ASR_BENCHMARK.md`; the wider model review is in
`research/VOICE_PIPELINE_REVIEW.md`.

## Read-Aloud (Text-to-Speech)

`src-tauri/src/tts` mirrors the speech-to-text layout:

- `provider.rs`: `TextToSpeechProvider` (prepare, synthesize with a deadline, shutdown), a
  typed `TtsError` whose messages never contain the text, and text preparation (control
  characters removed, whitespace collapsed, length bounded at a sentence boundary).
- `fish_s2.rs`: Fish Audio S2 Pro through the `s2.cpp` HTTP server, supervised exactly like
  the whisper server (loopback ephemeral port, listener ownership check, restart, deadline
  kill, start backoff, job object). The text is posted to `/generate` and the returned WAV
  is decoded in memory.
- `playback.rs`: resamples to the output device rate, limits the peak, fades the edges, and
  plays through `cpal` until the audio ends or a stop flag is set.
- `mod.rs`: the coordinator and Tauri commands (`tts_status`, `speak_text`,
  `speak_last_transcript`, `stop_speaking`). A newer request or a stop supersedes the one in
  progress. The engine is unloaded after five minutes without use and on app exit. Phase
  changes are emitted on `localflow://tts` without the text.

The engine and model live in `.localflow-runtime/fish` and `.localflow-runtime/models/fish`
and are installed by `scripts/Build-FishS2.ps1` and `scripts/Get-FishModel.ps1`. They are not
bundled. To add another voice engine, implement `TextToSpeechProvider` and return it from
`resolve_engine`.
