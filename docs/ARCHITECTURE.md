# Architecture

LocalFlow is split into a Tauri native layer and a React settings/history interface.

## Native Layer

- `src-tauri/src/workflow`: explicit dictation state machine. **Note:** this reducer is
  currently exercised only by the mock-session Tauri commands, not by the live hotkey path.
  See `REPO_AUDIT.md` §2 for the three current session representations and the plan to
  unify them.
- `src-tauri/src/audio`: microphone capture interface and mock capture, plus the audio code on the live path: `capture.rs` (the `cpal` input stream, single-channel downmix with a locked channel choice, capture statistics), `vad.rs` (`EndOfSpeechDetector`: loudness-based voice activity, end-of-speech, no-speech and maximum-length timeouts), `resample.rs` (band-limited windowed-sinc resampling to 16 kHz), `wav.rs` (in-memory 16-bit PCM WAV encoding), and `spectrum.rs` (the overlay's real-time analyser: a Hann-windowed FFT pooled into 32 mel-spaced bands from 70 Hz to 8 kHz, mapped to 0..1 against an adaptive noise floor, plus pitch from autocorrelation and brightness from the spectral centroid).
- `src-tauri/src/asr`: the speech-to-text provider boundary used by the live path. `provider.rs` defines `SpeechToTextProvider`, the typed `AsrError`, the per-utterance deadline, and `FallbackProvider`. Engines: `whisper_server.rs` (persistent `whisper-server.exe`, model kept loaded, loopback only), `whisper_cli.rs` (one process per utterance; fallback and low-resource engine), and `parakeet_cli.rs` (opt-in Parakeet TDT accuracy engine). `sidecar.rs` holds hidden-process launch, the Windows kill-on-close job object, and memory measurement. The older `AsrProvider` mock trait and the streaming foundation remain for the mock sessions and future streaming work.
- `src-tauri/src/llm`: the local language model. `ollama.rs` is the loopback-only Ollama client (no proxy, no redirects, cloud-hosted model tags refused, one fixed context size so the model is never reloaded between calls) and the warm-up; `cleanup.rs` is dictation cleanup (prompt, strict JSON contract, one repair attempt).
- `src-tauri/src/refinement`: refinement provider trait and mock provider used by the mock sessions.
- `src-tauri/src/context`: local context snapshot interface.
- `src-tauri/src/insertion`: `target.rs` (the window focused when dictation started, and the fail-closed check before pasting), `clipboard_paste.rs` (transient clipboard write, Ctrl+V, and the tidy-up afterwards: restore the earlier text, clear the dictated text when there is nothing to restore, and leave the clipboard alone if the user copied something else meanwhile), plus the mock inserter.
- `src-tauri/src/hotkeys`: global hotkey registration through Tauri's global-shortcut plugin.
- `src-tauri/src/overlay.rs`: the floating voice overlay: phase and audio-feature events on `localflow://native-dictation`, show/hide with the auto-hide timer, and placement above the taskbar.
- `src-tauri/src/runtime_paths.rs`: where local binaries and models are looked for (packaged resources, then `.localflow-runtime/`).
- `src-tauri/src/native_dictation`: the live push-to-talk coordinator: the recorder loop, hotkey tap/hold handling, the level meter, and the transcribe/format/paste worker. It composes `audio`, `asr`, `llm`, `insertion` and `overlay`; `session.rs` holds `SessionRegistry` and `latency.rs` the per-dictation timing snapshots. Carries a native `SessionRegistry` (session identity + supersede/cancel guard); the transcribe/refine/insert tail runs on a worker thread and revalidates the session before every side effect so a superseded or cancelled session never inserts.
- `src-tauri/src/transcript`: **authoritative** deterministic formatting for the native path — spoken punctuation, explicit self-corrections, filler/stutter cleanup, sentence capitalization, user replacements/snippets, and URL/email/decimal protection. Runs before the LLM and is the fallback when the LLM is unavailable.
- `src-tauri/src/storage`: SQLite schema initialization and the settings row. A row that cannot be parsed is copied to `settings.v1.unreadable` before defaults replace it.
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
- `src/components/VoiceOverlay.tsx`: compact overlay rendered in the Tauri `overlay` window. Subscribes to `localflow://native-dictation`. Phase events flow through React state; audio-feature events (about 36 a second, carrying level, pitch, brightness and 32 spectrum bands) only update a ref polled by the renderer, so React never re-renders at microphone-event frequency and a late feature event can never change the phase.
- `packages/localflow-sdk/src/react/topology/`: the renderer behind `LocalFlowOverlay`, a mirrored ridgeline spectrogram. The front ridge is the live spectrum (low frequencies at the centre, highs toward both ends) and each ridge behind it is the spectrum a little earlier, so sound recedes into the distance. `spectrumMath.ts` (pure, deterministic: band conditioning, attack/release smoothing, Catmull-Rom interpolation, a synthetic spectrum for consumers without real bands, layout and colour helpers), `audioFeatureSmoothing.ts` (frame-rate-independent smoothing and the `SpectrumHistory` ring), `overlayStateVisuals.ts` (dictation phase to visual parameters, including freezing the captured spectrum while processing, and reduced-motion variants), and `topologyRenderer.ts` (single-rAF Canvas 2D loop with preallocated typed arrays, hidden-line occlusion, DPI handling, visibility pause, and full listener cleanup). The renderer contains no dictation business logic. `?view=overlay-demo` previews it in a browser with generated speech-like input.
- `src/App.tsx`: settings, status, history, diagnostics, and mock workflow controls.

## Conversation Mode

A second mode beside dictation: a hotkey (`Ctrl+Alt+T`, or `Ctrl+Alt+Shift+T` when that is
taken) switches on a hands-free spoken conversation with the local language model. The same
hotkey interrupts a reply, or ends the conversation when the assistant is not talking.
Dictation is ignored while a conversation is on, so the two never share the microphone.

```
microphone (kept open) -> voice detector -> turn segmenter -> Parakeet
    -> command matcher --(goodbye, repeat, quiet)--> fixed action
                       --(anything else)----------> Ollama chat, streaming
    -> sentence chunker -> Phonon voice -> speaker queue
```

- `src-tauri/src/conversation/state.rs`: the turn-taking state machine, a pure function
  from (state, event) to (state, effects). Phases: off, starting, listening, user speaking,
  thinking, speaking. A conversation counter and a turn counter make late results from a
  cancelled turn or a closed conversation harmless.
- `conversation/engine.rs`: the threads that run a conversation, written against traits so
  the whole lifecycle is tested with scripted audio and fake models. The loop applies
  events; the ear reads the microphone, finds turn boundaries and enforces the limits; a
  worker per turn transcribes, routes and answers. The model streams into the voice and the
  voice into the speaker, so the first sentence is heard while later ones are being written.
- `conversation/router.rs`: fixed phrases handled without the model. The model is given no
  tools; nothing it reads or writes can trigger an action.
- `conversation/chunker.rs`: cuts the streamed reply into speakable pieces (first piece at a
  clause, later ones at sentence ends) and removes markup a voice would read aloud.
- `conversation/memory.rs`: the history sent to the model. In memory only, bounded to fit the
  fixed context size, cleared when the conversation ends. An interrupted reply is remembered
  only as far as it was heard.
- `conversation/mod.rs`: opens the real devices and models, and connects the engine to the
  hotkey, the tray, the overlay and the settings window.
- `audio/devices.rs`: `MicStream` and `SpeakerStream`, long-lived `cpal` streams whose
  callbacks neither allocate nor lock (`audio/ring.rs`). The speaker can be told to stop: it
  fades out over 6 ms and discards what was queued.
- `audio/speech_detector.rs` and `audio/turn_segmenter.rs`: Silero VAD (through the speech
  library already linked), with loudness against an adaptive noise floor as the fallback,
  and a frame-counted segmenter: 96 ms of speech starts a turn, 700 ms of silence ends it.
- `llm/chat.rs`: `ChatProvider` and the streaming Ollama client. Loopback only, cancellable
  by closing the connection, same context size as dictation cleanup.
- `tts/stream.rs` and `tts/phonon.rs`: `StreamingTextToSpeech` and the Phonon 31m voice,
  in-process on at most four CPU threads.

Rules the design enforces:

- **The assistant cannot hear itself.** The microphone is only acted on while listening or
  while the user is speaking; during thinking and speaking its audio is discarded, and the
  first 300 ms after a reply is skipped. Interrupting is by hotkey for now.
- **Limits.** 45 s of silence ends a conversation, as does 10 minutes in total; one turn is
  cut at 60 s. A microphone that reports an error or delivers nothing for 3 s ends it.
- **Nothing said is logged or stored.** Notices from the engine carry phases, fixed
  messages, counts and timings; a test checks that no transcript or reply text appears in
  them.

Measurements are in `benchmarks/CONVERSATION_BENCHMARK.md`.

## Web UI Boundary

The web UI is confined in two ways (`src-tauri/tauri.conf.json`, `src-tauri/capabilities/`):

- A content security policy allows scripts and styles from the app itself only, and
  network connections only to the app's own IPC. The Models screen asks the native layer
  which Ollama models are installed (`list_ollama_models`), so the packaged web UI needs no
  network access at all. Development builds use a looser policy for Vite's hot reload and
  for asking Ollama directly from a browser.
- Every command is declared in `src-tauri/build.rs` and granted per window. The `main`
  window may call them; the `overlay` window may only listen for events, so it cannot
  read transcripts or change settings. A new command must be added to `build.rs` and to
  `capabilities/main.json`, or the web UI cannot call it.

## Provider Boundary

Providers are intentionally narrow:

- ASR accepts mono 16 kHz samples, a language, and an optional vocabulary prompt, and returns text plus the engine that produced it. Every engine enforces a deadline and reports a typed error; a failure falls back to another local engine, never to a remote one.
- Refinement accepts compact dictation-cleanup inputs and returns strict JSON.
- Context providers never return password/protected-field content.
- Inserters must verify the intended target before writing text.

## Persistence

Settings structs are `#[serde(default)]`, so stored settings written before a field existed
still load and keep the user's dictionary, replacements and snippets. SQLite is initialized under the Tauri app data directory. Milestone 1 creates tables for settings, dictation history, and dictionary entries. Later milestones will add migrations and retention jobs.

## Speech-to-Text Engines

`plan_asr_engine` in `asr/engine_select.rs` decides the engine chain per dictation from the
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
in `asr/engine_select.rs`. Capture, formatting, and insertion do not change. The loaded
engine lives in the `AsrEngines` state, so any feature that transcribes speech shares it. Measurements
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
