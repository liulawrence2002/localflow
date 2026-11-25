# Changelog

All notable changes to LocalFlow are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). The detailed record of each
change, with its known limitations, is [docs/development/PLAN.md](docs/development/PLAN.md).

## [Unreleased]

Everything since the project started in July 2026. No version has been released yet.

### Added

- **Dictation** (July 2026). A tray app: tap `Ctrl+Alt+Space`, speak, pause, and the text is
  pasted where you started. A waveform overlay shows the microphone while listening.
  Deterministic formatting runs before any model (spoken punctuation, explicit
  self-corrections, filler and stutter removal, sentence capitals, with URLs, email
  addresses and decimals protected). Dictionary, replacements and snippets from Settings are
  applied. A local Ollama model (default `llama3.2:3b`) tidies the text in the background
  after the paste; low-resource mode skips it.
- **Safe insertion.** Each dictation has a session id; a superseded or cancelled one never
  pastes, `Escape` cancels while recording, and the text is pasted only into the window that
  had the focus at the start. "Copy last transcript" recovers a skipped paste. Dictated text
  is kept out of clipboard history and cloud clipboard sync.
- **Speech recognition.** NVIDIA Parakeet TDT 0.6B v3 kept loaded in-process through
  transcribe.cpp is the default (0.93 % word error rate on the bench corpus against 3.31 % for
  Whisper tiny.en), with a persistent whisper.cpp server and the one-shot CLI with tiny.en as
  local fallbacks. A reproducible speech-to-text benchmark harness.
- **Milestone 1, foundations.** Settings that survive new fields, the dictation code split
  into shared modules, a content security policy, per-window command permissions and a CI
  workflow.
- **Milestone 2, conversation.** `Ctrl+Alt+T` starts a hands-free spoken conversation with the
  local Ollama model: Silero voice detection, Parakeet, a streaming chat client and the Phonon
  voice, with hard time limits and an end-to-end latency benchmark.
- **Milestone 3, reading a window aloud.** `Ctrl+Alt+R`, or "read this page" in a
  conversation, reads the focused window's main content word for word through Windows UI
  Automation, behind a capture policy that refuses private windows, password managers,
  credential prompts and focused password fields.
- **Milestone 4, natural turn-taking.** Phrases are transcribed while you speak, the wait
  depends on how finished you sound, and you can talk over a reply: echo cancellation
  (WebRTC AEC3 through `sonora`), "mm-hmm" lets the reply carry on and real words stop it.
- **Upgrades from the open-source review.** The Smart Turn v3.2 end-of-turn model, the
  microphone kept on while the model thinks, and an optional GPU build (`gpu-vulkan`) that
  runs speech recognition on a discrete GPU (median response 568 ms against 993 ms on the CPU
  in the same session).
- **Milestone 5, voice commands for coding agents.** In a conversation, "open Claude Code in
  localflow", "ask Claude ..." and "open localflow in Cursor", each read back and run only
  after a spoken "yes". LocalFlow never passes options that skip the agent's permission
  prompts.
- **Optional read-aloud voice.** Fish Audio S2 Pro through s2.cpp, built and downloaded by
  explicit scripts and never bundled (non-commercial licence).
- **Release readiness.**
  - Licensed under MIT OR Apache-2.0, with `THIRD_PARTY_NOTICES.md` (generated crate and
    npm lists plus hand-written credits for ported code, the bundled whisper.cpp programs
    and every downloadable model), installed next to the app.
  - A readiness checklist command (`readiness_status`) that reports which models and
    services are installed and how to fix what is missing, without naming any local path.
  - Installed copies read downloaded models from `%LOCALAPPDATA%\LocalFlow`.
  - `scripts/Get-WhisperRuntime.ps1` fetches the pinned whisper.cpp v1.9.5 programs and
    checks every file's SHA-256.
  - A release workflow that builds the CPU installers (MSI and NSIS) from a version tag,
    ships the Visual C++ runtime next to the programs that need it, checks every shipped
    program's DLL imports, and attaches the installers with `SHA256SUMS` to a draft GitHub
    release.
  - An app icon, installer metadata, the WebView2 bootstrapper embedded in the installers,
    contribution guidelines, a code of conduct, issue forms, a pull request template and
    Dependabot.
- **Settings window, rebuilt** (November 2026). The screens are Home, Models, Dictation,
  Hotkeys, Dictionary, Replacements, Snippets, Coding agents, Privacy, Diagnostics and About,
  in the Windows light or dark theme.
  - Home: a Setup checklist (from `readiness_status`) while something required is missing; a
    dismissible banner after a dictation or reading problem, with the time and Copy last
    transcript or a hint, and a dot on Home in the navigation while it is up; how the last
    conversation ended.
  - The top bar says "Saving...", "All changes saved" or "Not saved", with Try again after a
    failed save.
  - Dictation: the End-of-speech wait (300 to 3000 ms, default 900 ms), which the native app
    uses.
  - Hotkeys: every hotkey and whether it is registered.
  - Diagnostics: OK / Check / Not available / Problem labels, "Settings storage" and "Last
    crash" rows, and Prepare export, which refreshes first and replaces the user folder and
    microphone names.
  - Privacy states what is kept and sent; About shows the version from the build, the licence
    and the copyright.
- **Log file.** `%LOCALAPPDATA%\app.localflow.desktop\logs\localflow.log`, plus
  `localflow.log.1`, at most 1 MB each. A panic hook records the thread and source location.
- **Single instance.** Starting LocalFlow while it runs shows the settings window.

### Changed

- Release builds are built without the `dev-runtime` feature: they no longer carry the build
  checkout's path, never look in it for models, and ignore `LOCALFLOW_WHISPER_CLI`. Programs
  are started only from the app's own folder and `%LOCALAPPDATA%\LocalFlow`.
- "Missing model" messages name the file and the script to run, never a folder.
- Every model script takes `-Destination`, defaulting to the checkout's `.localflow-runtime`
  inside a Git checkout and to `%LOCALAPPDATA%\LocalFlow` otherwise. `Get-AsrModel.ps1` and
  `Get-FishModel.ps1` download from a pinned revision and check SHA-256 instead of size.
- `Build-FishS2.ps1` builds in a fresh folder in the user profile (never at the drive root)
  and installs only `s2.exe` and the ggml DLLs the backend needs.
- The installers carry only the whisper.cpp files LocalFlow runs, not the other programs and
  libraries of the whisper.cpp release.
- `Start-LocalFlow.ps1` builds only the executable (`--no-bundle`) and finds CMake itself.
- **Dictation: tap or hold.** A tap is hands-free and stops after the End-of-speech wait
  (it was a fixed 550 ms). Holding the hotkey records until you let go and is never stopped
  by a pause. The 2-minute limit still applies to both. A press within 700 ms of an automatic
  stop is ignored.
- The status message while a dictation is transcribed is "Transcribing" instead of naming
  whisper.cpp, whichever engine runs.
- Closing the settings window hides it; "Show LocalFlow" in the tray brings it back.
- The Microphone screen is now Dictation; the Replacements columns are Heard / Write / On.
- Lists save a change when you leave the field. Emptying a field shows an error and never
  deletes the row.
- Settings load leniently: an unknown value falls back to its default and an out-of-range
  number is clamped, instead of the whole row being reset.
- A conversation looks for coding agents only when a project is set up, and reuses what it
  found for 30 minutes.
- Reading a transcript aloud stops a page reading. The page-reading voice stays loaded
  between readings and is freed after 5 minutes without one.
- `src/App.tsx` is split into `src/app`, `src/screens`, `src/components/ui`, `src/domain`
  and `src/styles`.
- The mock dictation commands (`begin_mock_session`, `finish_mock_session`) exist only in
  debug builds.

### Removed

- `scripts/Set-WhisperModelPath.ps1`, which wrote a file nothing read.
- `docs/TROUBLESHOOTING.md` (merged into the user guide) and `docs/ROADMAP.md` (folded into
  the voice assistant plan).
- Settings the native app did not use: the Styles and History screens, Threads, the
  microphone device, level meter, voice-detection switch and maximum length, Push/Toggle and
  the command hotkey, dictionary hints and categories, the snippet Cleanup switch, and the
  Privacy switches.

### Fixed

- The paste waits up to 1 s for `Ctrl`, `Alt`, `Shift` and `Win` to be released, and
  otherwise skips the paste and says so; a held `Alt` turned `Ctrl+V` into `Ctrl+Alt+V`.
- A dictation superseded while it was still being transcribed is kept for Copy last
  transcript, with a notice, instead of being lost.
- A missing speech engine is reported when the hotkey is pressed, before anything is
  recorded.
- Pasting into an administrator window gives a message instead of a false "Inserted".
- The earlier clipboard is put back once the target has read the dictated text (delayed
  rendering), never sooner than 400 ms and later in a Remote Desktop session, so a slow
  target no longer pastes the earlier clipboard.
- Quitting no longer hangs: shutting down waits at most 2 s.
- The Fish S2 engine is unloaded after 5 minutes without use even after a stop, and stopping
  a reading that is still being prepared stops the engine's work on it.
- Temporary audio files left by a crash are removed at the next start, and `whisper-cli`
  runs in a kill-on-close job.
- A damaged settings database is moved aside as `localflow.sqlite3.corrupt-<stamp>` and
  replaced, and a message box says so. A start-up failure shows a message box instead of
  nothing.
- A lost microphone ends a conversation with "Conversation ended: the microphone stopped
  working."
- A failed save is reported, never shown as saved, and if settings cannot be loaded nothing
  can be edited, so the saved settings are not replaced by defaults.

### Security

- The paste is also blocked when the keyboard focus moved to another control of the same
  window (`GUITHREADINFO::hwndFocus`; fields inside a web page share one such control).
- Clipboard content another program marked private, such as a password manager's, is
  cleared instead of restored, and a restore never adds a clipboard-history or cloud
  clipboard entry.
- An Ollama model that runs on Ollama's servers is refused whatever its name (its `/api/show`
  description names a `remote_host` or `remote_model`) and is left out of the model list.
- Web-side Ollama calls, used only in browser development, allow only
  `http://localhost:11434` (or `127.0.0.1`, `[::1]`) and refuse redirects. The
  `?view=overlay-demo` and `?view=mobile-sdk-example` pages exist only in development builds.
- LocalFlow's own background `claude` runs set `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`
  and `DISABLE_AUTOUPDATER=1`. The Claude Code tab you open is left as it is.
- The log file and `desktop-health.json` replace user folders with `<user folder>`; the
  recording-start entry no longer keeps the microphone's name; a panic is recorded without its
  message.

[Unreleased]: https://github.com/liulawrence2002/localflow/commits/master
