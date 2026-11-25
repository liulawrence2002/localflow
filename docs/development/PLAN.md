# LocalFlow Plan

## Wispr-class upgrade (spec-driven)

Phase 0 (evidence-based audit): completed. See `../archive/REPO_AUDIT.md` and
`../archive/PARITY_MATRIX.md` (historical snapshots) for current-state, discrepancies, risk-ranked debt, target
architecture, and the ASR/refinement benchmark plan. Baseline: 77 frontend tests,
14 Rust tests, all lint/format/type checks green.

Phase 1 (unify the production pipeline): in progress.

- **Slice 1 — session identity & cancellation (done).** Native `SessionRegistry` in
  `native_dictation.rs`; transcribe/refine/insert tail moved to a worker thread that
  revalidates the session before every side effect. A superseded or cancelled session
  never inserts (spec §4.4). +3 Rust tests.
- **Slice 2 — target-window revalidation (done).** Capture the foreground window
  (`TargetWindow { hwnd, pid }`) at record start; before pasting, skip insertion (fail
  closed) unless the same window is still focused (spec §6.1/§6.2). Addresses the top
  safety risk (wrong-window insertion). +2 Rust tests (19 total).
- **Slice 3 — copy/last-transcript recovery (done).** The last finalized transcript is
  kept in volatile memory and recoverable via a tray item and a Home > Recovery card
  (`get_last_transcript`/`copy_last_transcript`), so a skipped/failed insertion is never
  lost. Reveal-preview is opt-in; nothing is persisted. +3 frontend tests.
- **Slice 4 — deterministic smart formatting (done).** New authoritative Rust `transcript`
  module: spoken punctuation, explicit self-corrections, filler/stutter cleanup, sentence
  capitalization, URL/email/decimal protection. Applied before the LLM (seeds the cleanup
  prompt) and used as the safe fallback when the model is unavailable. +10 Rust tests.
- **Slice 5 — personalization + model config on the native path (done).** The native path
  now reads the user's settings: exact replacements + snippets are applied deterministically,
  dictionary terms bias whisper via its initial prompt, and the Ollama model is configurable
  (no longer hardcoded). +3 Rust tests.
- **Slice 6 — Escape-to-cancel (done).** Escape is registered only while recording and
  cancels the active dictation (routed to the existing `cancel` command); nothing inserts.
- **Slice 7 — honest diagnostics + labeled test controls (done).** Diagnostics describe the
  real native pipeline; the Home simulated-test panel is clearly labeled as a test, not
  production dictation.
- **Slice 8 — streaming ASR foundation (done, not yet on the default path).** New
  `src-tauri/src/asr` submodules: a typed `AsrEvent` contract + `StreamingAsrProvider` trait
  (spec §3.2/§4.3), an overlap-dedup `TranscriptStabilizer` and `StreamingSession` coordinator
  that never duplicate committed words, a rolling-window planner, and a `word_error_rate`
  metric for the benchmark harness. All unit-tested (+15 Rust tests, 47 total). This is the
  Phase 3 foundation; wiring a persistent whisper provider onto it + the benchmark harness is
  next (requires local models to verify).
- **Slice 9 — snappier, more Wispr Flow-like feel (done).** Tightened VAD/overlay timings
  (end-of-speech 760→550 ms, no-speech idle 6 s→2.5 s, overlay hold 1200→700 ms, paste
  settle 700→400 ms); silent idle close (no more "too short" error flash when you tap without
  speaking); changed the default cleanup model from the 12B `gemma4:12b-it-qat` to the fast
  `llama3.2:3b` (configurable; researched — the same Llama family Wispr Flow uses for cleanup)
  with the cleanup timeout bounded to 20 s; wired `low_resource_mode` as an instant-text mode
  that skips the LLM. +1 Rust test (48 total).
- **Slice 10 -- cleanup prompt hardening (done).** Added an explicit cleanup rule requiring
  the local model to preserve deterministicText capitalization, punctuation, line breaks, and
  technical casing unless the raw transcript clearly proves they are wrong. The native prompt
  and TypeScript preview prompt now share this contract, with focused regression coverage.
  +1 Rust test (49 total), +1 frontend test (81 total).
- **Slice 11 -- self-contained desktop launch + settings persistence (done).** Normal usage now
  points at the packaged release app through `Start-LocalFlow.ps1` / `npm start`, which stops
  the known Vite dev server and starts only the hidden tray app plus local Ollama when available.
  Settings are loaded from and saved to the SQLite `settings.v1` JSON row, and production builds
  hide simulated pipeline controls. +1 Rust test (50 total).
- **Slice 12 -- Wispr-like waveform pill polish (done).** Replaced the dark neon ribbon overlay
  with a compact frosted pill, centered professional waveform, pitch-reactive warm/cool harmonics,
  and a tiny phase dot for listening/processing/inserted/error feedback. Native overlay behavior
  and audio event contracts are unchanged.
- **Slice 13 -- taskbar-safe overlay placement (done).** Shrunk the overlay window and pill, then
  positioned the native overlay against the Windows monitor work area instead of full monitor
  bounds so it sits centered above the taskbar with a consistent bottom gap. +3 Rust tests.
- **Slice 14 -- flush overlay window cleanup (done).** Matched the native overlay window to the
  visible pill footprint, disabled the undecorated Tauri window shadow, and moved remaining visual
  depth inside the pill so no outer transparent rectangle or outline should show.
- **Slice 15 -- latency diagnostics + fast insert (done).** Added native per-stage timing snapshots
  to Diagnostics and changed the native path to paste deterministic text immediately after Whisper,
  then run configured local Ollama cleanup in the background for recovery/diagnostics. +5 Rust tests
  (58 total).
- **Slice 16 -- speech-to-text provider layer, persistent engine, and benchmark (done).**
  The native path now transcribes through `SpeechToTextProvider`. Default engine: a supervised
  persistent `whisper-server.exe` on loopback (in-memory audio, warm-up at record start,
  restart on exit, deadline kill, start-failure backoff, listener ownership check, kill-on-close
  job object, stop on app exit) with the one-shot CLI as the local fallback. Opt-in engines:
  base.en (model path) and Parakeet TDT through `parakeet-cli.exe`. Also: polyphase
  band-limited resampler, locked capture channel, watchdog timeouts when the microphone stops
  delivering audio, loopback HTTP clients without proxies or redirects, refusal of cloud-tagged
  Ollama models, bounded Ollama context, clipboard history exclusion, path-free error
  messages, health file written on phase changes only, and three bundled sidecar executables
  instead of 37. Added `examples/asr_bench.rs` and measured a baseline. +54 Rust tests (112
  total, 2 ignored). See `../research/VOICE_PIPELINE_REVIEW.md` and
  `../benchmarks/ASR_BENCHMARK.md`.
  Known limitations: measured on the speech-to-text stage with clean read speech, not end to
  end with a microphone; the packaged installer was not rebuilt or smoke-tested in this slice;
  the sidecar stays loaded (about 130 MB) until the app exits; a timeout on the server can be
  followed by a second wait on the CLI fallback; rich clipboard formats are still not restored.
  Next risks: antivirus or application control blocking the sidecar's loopback listener
  (falls back to the CLI), and clipboard exclusion flags interfering with remote-desktop
  clipboard redirection.
- **Slice 17 -- Parakeet as the default speech model, read-aloud wiring, storage cleanup
  (done, with one part unverified).** Speech-to-text now defaults to Parakeet TDT 0.6B v3
  (Q5_K_M GGUF) kept loaded in-process through the `transcribe-cpp` crate, with the Whisper
  CLI and tiny.en as the local fallback; engine choice is a pure, tested function
  (`plan_asr_engine`). Measured 0.93 % WER against 3.31 % for tiny.en at a similar median.
  The per-utterance Parakeet CLI engine and its bundled executable were removed. Added
  `src-tauri/src/tts`: `TextToSpeechProvider`, a supervised Fish Audio S2 Pro engine through
  the s2.cpp server, playback, commands, tray items, and Home-screen buttons, plus
  `scripts/Build-FishS2.ps1` and `scripts/Get-FishModel.ps1`. Storage: removed base.en, the
  old Parakeet file, the VAD model, and the 13 GB Rust debug build cache. +30 Rust tests (142
  total, 3 ignored).
  Follow-up the same day: the engine was built for CPU, Vulkan, and CUDA and measured. CUDA
  is installed (about 2.8 times slower than real time, 35 s start-up, 3.3 GB VRAM); CPU is
  about 11 times slower and Vulkan about 17 times. The packaged app was rebuilt and launched.
  Known limitations: read-aloud was exercised through the engine's server endpoint but not by
  a person from inside the running app, and voice quality was not judged; S2 Pro is slower
  than real time on every backend tried; the S2 Pro weights are research and personal
  non-commercial use only and are not bundled; the Parakeet model holds about 1.2 GB while
  loaded and is not unloaded when idle; Parakeet is slower than tiny.en on dictations over
  about ten seconds; the Parakeet model is read from the dev runtime folder and is not in the
  installer; the installer was not rebuilt or smoke-tested.
  Next risks: an in-process native crash would take the app down (the Whisper engines run as
  separate processes); the first build now needs CMake on `PATH`.
- **Slice 18 -- real spectrum overlay (done; not yet judged with a real microphone).** The
  overlay now shows what the microphone hears. Native: `audio/spectrum.rs` analyses the newest
  40 ms about 36 times a second (FFT, 32 mel bands, adaptive noise floor, pitch, brightness;
  92 microseconds per frame measured) and the event carries `bands`. Voice-activity detection
  still runs on 55 ms chunks. Feature events no longer touch the overlay window, hide timer,
  or health file. Web: the renderer is a mirrored ridgeline spectrogram with hidden-line
  depth, coloured by frequency while listening and by the phase accent otherwise; consumers
  without bands get a synthesised spectrum. Removed the synthetic terrain math. Added a
  browser preview at `?view=overlay-demo`. +20 Rust tests, +13 web tests.
  Known limitations: the look was checked with generated speech-like input in a browser, not
  with a live microphone in the packaged overlay; the mirrored layout is decorative rather
  than a calibrated analyser.
- **Slice 19 -- foundations for conversation mode (done; packaged app not launched).** No
  intended behaviour change to dictation. See `../VOICE_ASSISTANT_PLAN.md` for the audit and
  roadmap this starts.
  - `native_dictation.rs` (3,122 lines) split into `audio/capture.rs`, `audio/vad.rs`,
    `asr/engine_select.rs`, `llm/ollama.rs`, `llm/cleanup.rs`, `insertion/target.rs`,
    `insertion/clipboard_paste.rs`, `overlay.rs`, `runtime_paths.rs`, and
    `native_dictation/{mod,session,latency}.rs`. The loaded speech engine and the overlay
    epoch moved into their own Tauri states (`AsrEngines`, `OverlayRuntime`). Code and tests
    were moved, not rewritten.
  - Settings structs are `#[serde(default)]`, so adding a field no longer makes stored
    settings fail to load. A row that still cannot be parsed is copied to
    `settings.v1.unreadable` before defaults overwrite it.
  - Web UI confinement: a content security policy, per-window capabilities (the overlay can
    only listen for events), commands declared in `build.rs`, the unused opener plugin and
    web global-shortcut permissions removed.
  - Clipboard: dictated text is cleared when there was no earlier text to restore, a newer
    copy by the user is not overwritten (clipboard sequence number), and "Copy last
    transcript" is excluded from clipboard history and cloud sync.
  - Removed unused `thiserror` and `tauri-plugin-opener` crates and two unused npm packages;
    fixed diagnostics text that still described whisper.cpp as the engine; added
    `.github/workflows/ci.yml`.
  - Measured local model latency for conversation (in `../VOICE_ASSISTANT_PLAN.md`).
    +8 Rust tests (175 total, 4 ignored); 152 web tests unchanged.
    Known limitations: the packaged app was not rebuilt or launched, so the security policy,
    the per-window command permissions and the clipboard changes have not been exercised in
    the real window. The policy was checked by loading the built web UI in headless Edge with
    the same header (main, overlay and overlay-demo views rendered with no violations), which
    does not cover Tauri IPC. The CI workflow has not run on GitHub. `cargo check` in the
    debug profile was not run (tests ran in release).
    Next risks: a command added without its `build.rs` and `capabilities/main.json` entries
    fails only at run time in the web UI; clearing the clipboard after a paste removes an
    image or file that was there before, which the paste had already replaced.
- **Slice 20 -- conversation mode, first version (done; not yet tried by a person).** A
  second mode beside dictation, toggled by `Ctrl+Alt+T` (fallback `Ctrl+Alt+Shift+T`), the
  tray, or the Home screen. See `../ARCHITECTURE.md` (Conversation Mode) and
  `../benchmarks/CONVERSATION_BENCHMARK.md`.
  - New: `conversation/{state,engine,router,chunker,memory,mod}.rs`, `llm/chat.rs`,
    `tts/{stream,phonon}.rs`, `audio/{devices,ring,speech_detector,turn_segmenter}.rs`,
    `cancel.rs`, a stateful `StreamResampler`, `scripts/Get-PhononModel.ps1` and
    `Get-VadModel.ps1` (pinned revisions, SHA-256 verified), and three benchmark examples.
  - Models: Phonon 31m voice on at most four CPU threads; Silero VAD v6.2 with a loudness
    fallback; the configured Ollama model with streaming, cancellation and no tools.
  - Safety: the microphone is acted on only while listening; 45 s idle, 10 min total and
    60 s per turn limits; device loss ends the conversation; dictation is ignored while a
    conversation is on; nothing said is logged, stored or sent in an event.
  - Also: the Models screen asks the native layer for installed Ollama models (the web
    check never worked in the packaged app because Ollama refuses the window's origin), and
    the packaged web UI's policy no longer allows any network connection.
  - Measured: median 1,090 ms from end of speech to first reply audio (slowest 1,859 ms),
    down from 1,860 ms before the model was kept awake during the user's turn.
    +98 Rust tests (273 total, 6 ignored), +7 web tests (159 total).
    Known limitations: no person has yet spoken to it through a real microphone and speaker,
    and nobody has judged the voice; the benchmark's questions are synthetic speech delivered
    digitally. Interrupting is by hotkey only, because the microphone is ignored while the
    assistant speaks. English only. The 700 ms end-of-turn wait and whole-utterance
    transcription dominate the delay. The overlay reuses dictation's phases ("listening",
    "processing", "refining") rather than having its own, and the tray icon does not change.
    History is trimmed, not summarised. The voice and Silero models are read from the dev
    runtime folder and are not in the installer. `Ctrl+Alt+T` may clash with other software.
    Next risks: the in-process voice and voice detector add native code to the main process;
    the keep-warm requests keep the GPU awake while the user speaks; a model that does not
    fit in GPU memory will answer too slowly to be usable.
- **Slice 21 -- read a window aloud (done; exercised on test pages, not yet on real sites by
  a person).** `Ctrl+Alt+R` (fallback `Ctrl+Alt+Shift+R`) reads the focused window word for
  word and stops when pressed again; in a conversation "read this page", "next paragraph",
  "go back", "read that again" and "read from the top" do the same. No language model is
  involved. See `../ARCHITECTURE.md` (Reading a Window Aloud) and `../PRIVACY.md`.
  - New: `platform/uia/` (UI Automation reader on a dedicated thread with a deadline),
    `context/page/{raw,types,policy,extract,cursor,speech}.rs`, `page_reader.rs`,
    `conversation/speaking.rs` (shared by replies and reading), `examples/page_probe.rs`.
    The reader and the extraction were written by two agents against a fixed data contract
    and joined afterwards.
  - Checked on real Microsoft Edge with a test article: 126 nodes read in 105-232 ms,
    extraction in under 0.4 ms; the article's 18 blocks came out in order with navigation,
    sidebar, search box, footer and cookie dialog excluded, and two pieces of deliberately
    hidden text (a zero-size box and a screen-reader-only span) left out after a fix. The agent's own checks covered
    Edge and Chrome: password fields flagged and never read, typed values absent.
  - Checked in the running app by sending the real hotkey over test pages: the article was
    read aloud and stopped on the second press; an InPrivate window and a page with its
    password field focused were both refused with the reason shown and spoken.
    +147 Rust tests (420 total, 6 ignored), +4 web tests (163 total).
    Known limitations:
  - Not tried on real websites by a person. Extraction on pages that do not mark their main
    content is a heuristic: comments, "read more" links and small sidebars can be included.
  - Hidden text: a block clipped in place, or laid one word per line over two other blocks, is
    filtered. Not detected: hidden text inside a real paragraph, off-screen indents,
    same-colour or transparent text, and clipped boxes that keep a normal width. The same rule
    can skip real text in collapsed sections and non-current carousel slides.
  - Private windows are detected from English window titles only.
  - Pages above roughly 30,000 accessibility nodes time out (estimated from 15,000 working
    in 1.8-3.3 s and 50,000 failing); the first read of a long page can also time out while
    the browser builds its tree, and works when asked again.
  - Google Docs without its screen-reader mode, scanned PDFs, canvas apps and elevated
    windows expose nothing. Firefox, PDFs, frames and Electron apps were not tested.
  - The reading hotkey is ignored during a conversation (say "read this page" instead).
    Spoken reading commands are English phrases matched exactly.
  - The page does not scroll or highlight as it is read.
    Next risks: asking a browser for its accessibility tree switches that feature on in the
    browser; `Ctrl+Alt+R` may clash with other software.
- **Slice 22 -- natural turn-taking (done; interruption by voice not yet tried by a person).**
  See `../ARCHITECTURE.md` (Conversation Mode) and `../benchmarks/CONVERSATION_BENCHMARK.md`.
  - Transcribing during the turn: `conversation/assembler.rs` sends each phrase to the
    recogniser at a 250 ms pause and ends the turn once everything said is transcribed and
    the silence has lasted 400 ms, 700 ms or 1.4 s depending on how finished it reads
    (`conversation/turn_completion.rs`). The turn worker no longer transcribes anything.
  - Talking over the assistant: `audio/echo.rs` (the `sonora` port of the WebRTC canceller,
    pure Rust, pinned), the speaker reporting exactly what it played, a stricter speech gate
    while a reply is spoken, a `BargeIn` transition in the state machine, and
    `conversation/echo_guard.rs` to discard an interruption that is the assistant's own
    words. With no working canceller the previous behaviour applies (hotkey only).
  - Three agents built the echo canceller, the completeness judge and a Smart Turn audio
    model in isolated harnesses while the engine was restructured.
  - Measured: median 663 ms from end of speech to first reply audio over twelve turns
    (slowest 786 ms), against 1,090 ms and 1,859 ms before. On this laptop the audio driver
    already removes the speaker from the microphone (46-56 dB loss in three of four runs).
    +76 Rust tests (496 total, 7 ignored); 163 web tests unchanged.
    Known limitations:
  - Nobody has talked over the assistant at a real microphone. The tests use scripted
    audio, and the echo canceller's final settings were not run against the real devices.
    One measurement run had the microphone 27 dB louder than the others for unknown
    reasons, so behaviour may vary from day to day.
  - The completeness judge is English, rule-based, and tested on written utterances. It
    treats "hold on" and "wait" as hesitation, never calls an unpunctuated statement
    complete, and can be fooled by a full stop the recogniser put after a verb that wanted
    more. Its precision for "incomplete" on the held-out set was 0.85, below the 0.9 aim,
    which shows up as waiting longer than needed, not as cutting the user off.
  - The turn ends when the last phrase's transcript arrives, about 350-500 ms after the
    pause, so the 400 ms wait is rarely the limit; a faster recogniser would be.
  - Smart Turn (`audio/smart_turn.rs`, `scripts/Get-SmartTurnModel.ps1`) is written and
    validated against the reference implementation but NOT compiled into the app: it would
    add about 27 MB and 40 crates, takes 62-80 ms per decision (150-210 ms under load), and
    in a rough check called 18 of 60 cut-off clips complete. The file is not in the module
    tree. Decide after real use whether to wire it in or delete it. (Decided in slice 24 by
    a benchmark on Smart Turn's public test set: wired in.)
  - A real interruption that repeats most of a sentence the assistant just said is
    discarded as echo. While the assistant is thinking, speech is still ignored (fixed in
    slice 24).
    Next risks: an echo that slips through all three layers makes the assistant cut itself
    off; the canceller adds 28 ms of delay to the microphone path.
- **Slice 23 -- code review fixes (done).** A review of the branch against `master` found
  ten issues; seven were fixed, three are recorded below.
  - Fixed: a conversation now stops a page reading when it starts, and read-aloud refuses to
    start during a conversation, so neither can be heard by the conversation's microphone and
    sent to the model. Starting a dictation stops read-aloud and page reading, which it would
    otherwise record and paste. The conversation hotkey is ignored during a dictation and the
    reading hotkey during a conversation, instead of replacing and then hiding the overlay.
    A page reading cut off part-way through a paragraph's last sentence now reads that
    paragraph again (`Spoken::finished_pieces`). A Stop pressed just as read-aloud synthesis
    finished could be lost; it now always stops playback. Ollama error bodies, which can echo
    the dictated text, no longer reach the log.
  - Verified live in the app: read-aloud refused during a conversation; the reading hotkey
    ignored without touching the overlay; a dictation stopping a page reading; read-aloud
    working again afterwards. +1 Rust test (497 total, 7 ignored, plus all 7 real-model tests
    passing).
  - Not fixed, all present on `master` before this branch: the SDK's Tauri adapter calls
    `mobile_start_dictation`/`mobile_stop_dictation`, which the app does not have; changing the
    speech model while a transcription runs can shut that engine down mid-request; Win32 calls
    in `asr/sidecar.rs` and `overlay.rs` sit outside `platform/` as AGENTS.md asks.
- **Slice 24 -- turn-taking upgrades from the open-source review (done; not yet tried by a
  person).** The review is summarised in `../VOICE_ASSISTANT_PLAN.md` ("Lessons from other
  projects").
  - Talking over a reply pauses it instead of cancelling it (LiveKit Agents' "false
    interruption"). `conversation/barge_in.rs` judges what was said: a stop word, a control
    phrase or two real words cancel the reply; blank, filler, backchannel ("mm-hmm", "okay",
    "yeah") or echo resume it; 1.5 s of quiet with nothing decided resumes it; a hold over 6 s
    cancels it. The word minimum applies only while the reply is audible (Pipecat), so "yeah"
    after a reply has finished still goes to the model. `AudioSink` gained `pause`/`resume`; the
    speaker fades out over 6 ms and stops reading the queue, so held audio is never counted as
    heard.
  - The microphone stays on while the model thinks. Speech then holds the reply before any
    of it plays, and real words join the earlier turn into one message (Pipecat's "turn
    resumed"); "read this page" followed by "from the top" is understood as one command.
    Speech that starts before a reply begins is carried into the next turn instead of lost.
  - The barge-in gate uses two thresholds (start at 0.65, stay above 0.5, Silero-style), so
    one quiet frame no longer resets it; it still needs 288 ms of speech.
  - Smart Turn is wired in (`audio/smart_turn.rs`, tract 0.23.10, the fp32 model
    `smart-turn-v3.2-gpu.onnx`; the 8-bit `-cpu` file gives wrong answers in tract). Scored
    first on 2,000 English clips from `pipecat-ai/smart-turn-data-v3.2-test`
    (`../benchmarks/TURN_JUDGE_BENCHMARK.md`): on 1,600 human clips the text judge alone ended
    28.4 % of unfinished turns too early, Smart Turn with the text judge 4.5 %, with the wait
    on finished turns 415 ms against 431 ms. At each pause it hears the last 8 s of the turn
    on its own thread while Parakeet transcribes. Above 0.5 means ready (400 ms), otherwise not
    ready (1.4 s); words that trail off pull ready down to unsure (700 ms), and words that read
    finished lift not ready up to unsure. While its answer is still out, finished-sounding
    words wait the unsure time rather than the short one; a late answer still applies, and a
    missing or failed model leaves the words to decide alone. Results for an earlier pause
    are discarded.
  - Races fixed with turn numbers: a cancelled reply can no longer flush or write over a newer
    reply (`SpeakerGate` in `speaking.rs`); memory entries are kept in turn order, so a late
    reply cannot land after the next question; finished turns are handed over by number
    rather than through one slot.
  - Speech recognition on the GPU: a `gpu-vulkan` Cargo feature (`npm run tauri:build:gpu`)
    builds transcribe.cpp with Vulkan, picks the discrete GPU by name (never the integrated
    one), and falls back to the CPU if the GPU is missing or a load or run fails. Each
    conversation starts with a warm-up (2 s on the CPU; 2, 4 and 10 s on a GPU).
    `LOCALFLOW_ASR_DEVICE=cpu` and `LOCALFLOW_ASR_THREADS` override it. Measured
    (`../benchmarks/ASR_SPEED_BENCHMARK.md`, `../benchmarks/CONVERSATION_BENCHMARK.md`): the
    same word error rate (0.85 % against 0.93 %), 135-199 ms against 383-734 ms for a 1-3 s
    clip, and a median response of 568 ms against 993 ms on the CPU in the same session.
    The CPU thread count stays at the library's default (8 here); every logical CPU (22)
    never finished the corpus.
    Known limitations:
  - The GPU build imports `vulkan-1.dll` and will not start on a computer without a Vulkan
    loader, so the feature is off by default; it shares the GPU's memory with Ollama (about
    0.8 GB more) and there is no free-memory check before loading.
  - Silero VAD v6.3 (released 2026-11-23) has no GGUF conversion yet; v6.2 stays.
  - A resumed reply comes back after about 0.6-1.2 s of silence and continues mid-word.
  - Whether Parakeet writes "mm-hmm" as one of the listed forms is unverified on real audio.
  - A one-word stop word always wins over the echo check, so a reply that leaks back as just
    "no" or "stop" stops itself (not seen on this laptop, whose driver removes the echo).
  - No loudness floor on the barge-in gate yet; it needs real-room recordings to tune.
- **Slice 25 -- voice commands for coding agents (milestone 5; built, not yet tried live).**
  `coding_agents/` (detection, spoken names, grammar, read-back dialog, argument building, the
  background runner), `platform/process.rs` (a process tree inside a kill-on-close job object),
  `conversation/agents.rs` (the conversation side), and a "Coding agents" Settings screen.
  - "Open Claude Code in <project>" opens a Windows Terminal tab running the real `claude.exe`
    (never the npm `.cmd` shim) with `--permission-mode default`, then ends the conversation
    so the task is dictated into Claude's own prompt. No spoken text goes on the terminal's
    command line. "Ask Claude ..." runs `claude.exe -p` with read-only tools, `dontAsk`, the
    folder's own hooks, tool servers and settings switched off, no saved session, and the
    question on standard input; "stop that" or the hotkey kills its whole process tree.
    "Open <project> in Cursor" opens the folder in Cursor.
  - Every command is read back and needs a "yes" said after the read-back has finished. The
    read-backs contain no stretch of words that itself counts as a yes (tested). Anything
    else declines; 15 s of silence declines.
  - Spoken names follow Talon community's rules plus a spaces-removed comparison; a name
    matching no project or several is asked about. A guard refuses any argument list that
    could skip permission checks, wherever it appears.
  - Folders containing `;`, `%` or `"` are refused, because Windows Terminal splits at `;` even
    inside quotes and expands `%NAME%`.
    Verified live: the real Windows Terminal opens the tab in a folder whose path has spaces
    and hands the program exactly `--permission-mode default`
    (`live_windows_terminal_passes_the_folder_and_arguments_through`, with `examples/wt_probe.rs`
    standing in for `claude.exe`; opt-in with `LOCALFLOW_LIVE_WT=1`). A review of PR #2 found
    and fixed: a confirmed question lost when the user talked over "Asking Claude"; "stop that"
    not stopping an answer that arrived during a hold; folders whose path contains
    "dangerously" or "acceptEdits" refused at launch; `//?/` paths accepted then failing; the
    warm-up holding the model lock for dictation. Known limitations: Claude Code and Cursor themselves have not been launched
    by voice yet; a real `claude -p` answer and its sign-in failure wording are unverified; an
    answer begun within 150 ms of the read-back ending is ignored, and the user hears the
    time-out line; "say that again" while a read-back waits declines it; Windows may not bring
    the new tab to the front, so dictation could land elsewhere (the user sees where before
    pressing Enter); Codex and Cursor Agent have no command-line tools installed, so they say
    they are not set up.
- **Slice 26 -- professional hardening (done; the installer has not been tried on a clean
  machine).** Four read-only audits (security and privacy, native reliability, frontend and
  UX, engineering and release) produced about 100 findings, none critical. Fixed in four
  parallel batches:
  - Dictation: holding the hotkey records until release (a pause no longer ends it); a tap
    stops after the End-of-speech setting (was a fixed 550 ms); a press within 700 ms of an
    automatic stop is ignored; a dictation superseded while transcribing is kept for Copy last
    transcript; the paste waits for Ctrl/Alt/Shift/Win to be released; the clipboard is
    restored after the target reads it (delayed rendering) and content another app marked
    private is cleared, never restored, and never re-enters clipboard history; focus moving
    within the target window and administrator windows block the paste with a clear message;
    a missing engine is reported at the hotkey; whisper-cli runs in a kill-on-close job and
    stale temp audio is swept; the recorder is supervised.
  - App lifecycle: closing the settings window hides it; single instance; a damaged settings
    database is moved aside and replaced, settings load leniently, and fatal start errors
    show a message box; a size-capped log file (`app_log_dir\localflow.log`, user folders
    redacted) and a panic hook that records only the thread and location; exit work capped
    at 2 s off the main thread; slow commands off the main thread; atomic desktop-health
    writes; Ollama cloud models refused by `/api/show`, not only by name; coding agents not
    probed without projects; Fish S2 requests cancellable; page-reading status races fixed.
  - Settings window: the layout regression fixed; native errors no longer replaced by mock
    data; validated number fields; editing a row never deletes it; about 20 controls that did
    nothing hidden; dictation, conversation and read-aloud errors shown on Home; a Setup
    checklist (`readiness_status`); true privacy facts; light and dark themes; accessibility
    fixes; `App.tsx` split by screen; diagnostics export redacts user folders and microphone
    names.
  - Release and repository: `MIT OR Apache-2.0` licence and third-party notices; models in a
    per-user folder (`%LOCALAPPDATA%\LocalFlow`) with the checkout path only under the
    default `dev-runtime` feature; every download pinned and hash-checked, including a pinned
    whisper.cpp runtime; app-local VC++ runtime; a tag-triggered release workflow that strips
    build paths and writes checksums; CI pins actions by SHA and runs clippy with
    `-D warnings`, `npm audit` and the examples build; Dependabot; contributor, conduct,
    security-reporting and issue files; an original icon; stale docs archived.
  - Tests: 850 Rust (23 ignored), 257 frontend.
    Known limitations: the release workflow and the installer have never run; the delayed-
    rendering clipboard path is unit-tested but not tried on the real clipboard; the
    installers are unsigned (SmartScreen will warn); the bundled whisper-server is an
    unauthenticated loopback server (documented in SECURITY.md); speech from a video or call
    could still trigger a coding-agent command; `rust-toolchain.toml` waits for disk space.
- Follow-ups (roadmap): unload the speech model when idle; streaming partials; UI Automation
  insertion; native context capture; paste-last-into-focus hotkey; code signing; a first
  release dry run with a release-candidate tag.

## Milestone 1: Foundation

Status: completed.

Completed:

- Created a Tauri 2 + React + TypeScript project.
- Added root documentation, agent instructions, prompt contract, and Windows scripts.
- Added explicit dictation workflow state machine in TypeScript and Rust.
- Added mock ASR, mock refinement, mock insertion, and provider traits.
- Added settings UI screens for home, models, microphone, hotkeys, dictionary, replacements, snippets, styles, privacy, history, diagnostics, and about.
- Added SQLite schema initialization for settings, history, and dictionary data.
- Added Tauri tray setup and global-hotkey plugin registration for `Ctrl+Alt+Space`.
- Added unit tests for state transitions, transcript stabilization, deterministic personalization, insertion spacing, and LLM JSON validation.
- Added session-id validation for result-bearing workflow events to reject stale ASR/refinement/insertion results.
- Added build-tested audio helpers for bounded PCM ring buffers, RMS calculation, mono downmixing, linear resampling, and VAD/end-of-speech detection.
- Added rolling ASR window planning for overlap-based incremental transcription.
- Added shared `whisper.cpp` sidecar command planning, vocabulary prompt construction, and JSON transcript parsing.
- Added local cleanup JSON contract runner with one repair attempt and deterministic fallback that preserves the raw transcript.
- Added privacy retention helpers for disabled history, transcript-only history, delete-after windows, context retention, and redacted diagnostics.
- Added diagnostics export generation that excludes dictated content and local model paths by default.
- Added shared context snapshot policy with protected-field blocking, context limits, cleanup mapping, and application category classification.
- Added shared performance recorder and diagnostics formatter for latency and peak-memory metrics without invented values.
- Added timeout guard for local providers.
- Added editable dictionary, replacement, snippet, and style-profile controls in the settings UI.
- Added settings mutation helpers for personalization CRUD and browser fallback persistence.
- Extended style profiles with greeting, sign-off, filler-removal, and sentence-fragment behavior fields.
- Added command-mode planning with selected-text requirement, OS-command rejection, preview decisions, strict JSON parsing, and undo text.
- Added insertion planning helpers for target validation, ordered accessibility/keyboard/clipboard fallback, delayed clipboard restoration, and duplicate insertion rejection.
- Added “Undo AI cleanup” UI action for the mock workflow output.
- Added localhost-only dictation network policy checks.
- Added a tested Ollama refinement provider that discovers local models, blocks remote URLs, sends non-streaming JSON generate requests, and reports unavailable or missing-model errors clearly.
- Added Models screen Ollama discovery and local model selection for the browser/dev UI path.
- Installed Rust stable through rustup and Microsoft Visual Studio C++ Build Tools for native Windows builds on this workstation.
- Added a first native push-to-talk path behind the Tauri global hotkey: default microphone capture, local `whisper.cpp` transcription, and clipboard paste insertion.
- Added a fallback global hotkey, `Ctrl+Alt+Shift+Space`, when `Ctrl+Alt+Space` is already registered.
- Added bundled-resource lookup for the local Whisper CLI, DLLs, and tiny English model, with `.localflow-runtime/` as the dev fallback.
- Added native local Ollama cleanup pinned to model `gemma4:12b-it-qat`, with strict JSON parsing, one repair request, and raw-transcript fallback if cleanup fails.
- Added a hidden-on-start settings window and a compact always-on-top waveform overlay for active dictation.
- Added native microphone level events so the overlay reacts to speech volume while listening.
- Added native end-of-speech auto-stop so dictation starts processing after a short post-speech pause, while still allowing hotkey release to finish immediately.
- Added hybrid tap/hold hotkey handling: quick releases now keep recording open for tap-to-start dictation, while a second tap or longer hold release can stop manually.
- Added no-speech timeout after tap-to-start so the overlay does not stay open forever when no voice is detected.
- Added native `gemma4:12b-it-qat` warmup on recording start and longer Ollama keep-alive to reduce cleanup wait on repeated dictations.
- Refined the floating overlay into a wider polished waveform with live bars, generated wave paths, processing sheen, and success/error state colors.
- Hid the `whisper-cli.exe` sidecar process window on Windows so hotkey dictation only surfaces the waveform overlay.
- Added live pitch and brightness extraction from microphone chunks and replaced the overlay with a dark, layered audio-ribbon canvas: higher pitch lifts warmer upper strands and lower pitch deepens cooler lower strands.

Verified:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test` with 77 passing tests.
- `npm run build`
- `cd src-tauri; cargo fmt --check`
- `cd src-tauri; cargo test` with 14 passing tests.
- `cd src-tauri; cargo check`
- `npm run tauri:build`, producing:
  - `src-tauri\target\release\localflow.exe`
  - `src-tauri\target\release\bundle\msi\LocalFlow_0.1.0_x64_en-US.msi`
  - `src-tauri\target\release\bundle\nsis\LocalFlow_0.1.0_x64-setup.exe`
- Vite dev server at `http://127.0.0.1:1420/`
- Live dev-server smoke check: page status 200, root element present, transformed `App.tsx` contains LocalFlow Home, Privacy, Diagnostics, mock transcript UI markers, editable personalization UI markers, Undo cleanup marker, Ollama check markers, and command-mode module task marker.
- `npm run tauri -- info` after prerequisite installation.
- `.\scripts\Check-Ollama.ps1`, confirming local model `gemma4:12b-it-qat`.
- Direct local Ollama generate smoke test with `gemma4:12b-it-qat`, returning strict cleanup JSON.

Known native limitations:

- Manual microphone dictation and text insertion must still be exercised by a human in Notepad, a browser field, and VS Code.
- Native dictation now inserts deterministic personalized text first, then runs the configured local Ollama cleanup model in the background for recovery/diagnostics.
- Clipboard fallback restores prior text clipboard content, but not rich clipboard formats.
- UI Automation insertion, target-window verification, and startup-at-login are still pending.

Re-run:

- `npm install`
- `npm run format`
- `npm run lint`
- `npm run test`
- `npm run build`
- `cd src-tauri; cargo fmt --check; cargo test; cargo check`
- `npm run tauri:dev`

Risks:

- Rust toolchain and Windows C++ build tools may be missing on other developer machines; `scripts/Install-Prereqs.ps1 -Install` now covers rustup and MSVC Build Tools.
- Tauri plugin API drift can break native compilation; keep `npm run tauri -- info` and Rust checks in the verification loop.
- The first native path proves local recording/transcription/paste plumbing, but not final latency, cleanup quality, target-window safety, or broad app compatibility.

Acceptance criteria for this milestone:

- Frontend builds and tests pass.
- Native code is structured behind small interfaces and ready for compilation once prerequisites are installed.
- UI can exercise a mock dictation session end to end without remote calls.

## Milestone 2: Audio and Local ASR

Status: partial, shared logic verified and first native local-ASR path runnable.

Completed:

- Bounded audio ring buffer.
- Mono downmixing and sample-rate normalization helper.
- RMS-based VAD/end-of-speech detector.
- Rolling-window planner with overlap.
- Shared `whisper.cpp` sidecar invocation planner using model, audio, thread, language, prompt, JSON-output, and CPU/GPU flags.
- Shared `whisper.cpp` JSON transcript parser with segment timestamps.
- Initial prompt builder for dictionary terms and pronunciation hints.
- Timeout guard for local provider calls.
- Shared performance recorder for hotkey-to-recording, ASR partial, release-to-final, LLM, insertion, model-load, and peak-memory metrics.
- Native `cpal` default-microphone capture on a dedicated recorder thread.
- Multi-channel input downmixing selects the loudest active channel to avoid phase-cancellation silence.
- Native end-of-speech detection stops recording after speech is heard and a short silence follows.
- Quick hotkey taps now start dictation without closing the overlay on key release.
- No-speech timeout ends abandoned tap-to-start sessions.
- Native recording has a maximum-duration cap for the current hotkey path.
- Capture diagnostics compute duration, peak, RMS, and nonzero sample ratio before Whisper runs.
- Temporary mono 16 kHz WAV writing for the current sidecar path, with cleanup after transcription.
- Local `whisper-cli.exe` launch against `ggml-tiny.en-q5_1.bin` with JSON output parsing, no timestamps, and adaptive CPU thread count.
- Hidden Windows sidecar launch prevents terminal flashes during transcription.
- Near-silence captures and blank Whisper markers are rejected with clearer errors.
- Clear missing-runtime and missing-model errors for the native path.
- Live microphone level, pitch, and brightness sampling for the floating waveform overlay.
- Polished floating overlay shows pitch-reactive colored ribbon waves and distinct processing, inserted, and error states without opening the full settings UI.

Not yet completed:

- Replace the temporary WAV path with a longer-term sidecar process manager and narrower IPC boundary.
- Add cancellation, timeout enforcement, sidecar crash recovery, and native latency metric feeds.
- Wire rolling partial transcription into real ASR events.
- Drive overlay motion from stabilized partial transcript timing once live partial ASR is wired; the current overlay reacts to live acoustic features.
- Add microphone selection and device-disconnect recovery.

## Milestone 3: Insertion and Cleanup

Status: partial, shared cleanup contract verified and native clipboard insertion started.

Completed:

- Strict local-cleanup JSON validation.
- One repair attempt for invalid cleanup responses.
- Deterministic fallback that preserves raw transcript.
- Timeout guard for provider calls.
- Undo AI cleanup helper and UI action for restoring deterministic/raw text.
- Insertion target validation, method ordering, clipboard restoration plan, and duplicate insertion guard.
- Local Ollama model discovery through `/api/tags`.
- Local Ollama cleanup requests through `/api/generate` with `stream: false` and strict JSON-format output.
- Native Ollama cleanup requests use the configured local model, defaulting to `llama3.2:3b`.
- Native recording start warms pinned `gemma4:12b-it-qat` in the background and keeps it loaded longer for repeated dictation.
- Remote Ollama URLs blocked before fetch.
- Clear shared errors for unavailable Ollama, no selected model, and missing local model.
- Native clipboard paste fallback using Win32 `SendInput` for `Ctrl+V`.
- Delayed restoration of the previous text clipboard after paste.

Not yet completed:

- Add Windows target tracking.
- Add UI Automation insertion where safe.
- Preserve rich clipboard formats during native fallback.
- Wire deterministic personalization into the production native dictation workflow.

## Milestone 4: Personalization

Status: partial, editable UI and shared mutations verified.

Completed:

- Editable dictionary entries.
- Editable replacements.
- Editable exact snippets.
- Editable style profiles with conciseness, formality, contractions, emoji, paragraph length, bullet preference, greeting/sign-off behavior, filler removal, and sentence-fragment controls.
- Shared settings mutation helpers with tests.
- Shared context snapshot policy for optional active-app, accessibility text, and selected-text collection.
- Shared application-category classifier for messaging, email, documents, code editors, terminals, search fields, and generic text fields.

Not yet completed:

- Persist dictionary, replacements, snippets, and style profiles through native SQLite across desktop restarts.
- Inject relevant vocabulary into ASR prompts.
- Wire native Windows context capture into the shared context policy.

## Milestone 5: Advanced Workflow

Status: partial, command-mode planning verified.

Completed:

- Command mode requires selected text.
- Command mode rejects operating-system command execution.
- Command mode classifies common editing intents and chooses preview for large selected text.
- Command mode preserves undo text.

Not yet completed:

- Add optional live insertion.
- Wire command mode to native selected-text capture, preview UI, local model execution, and insertion.
- Add native diagnostics file export if needed, broader privacy controls, and retention enforcement.

## Milestone 6: Packaging and Hardening

- Tauri bundle config now includes the Whisper CLI, required DLLs, and tiny English model as app resources.
- Build installer.
- Add startup-at-login.
- Add crash recovery, benchmarks, security review, and full manual acceptance testing.
