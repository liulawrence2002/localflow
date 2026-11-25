# LocalFlow

LocalFlow is a local-first Windows voice app built with Tauri 2, Rust, React and TypeScript.
It runs quietly in the tray and does four things:

- **Dictation.** Tap `Ctrl+Alt+Space`, speak, pause, and the text is pasted into the field
  where you started; or hold the hotkey while you speak and let go. Speech recognition is
  NVIDIA Parakeet TDT 0.6B v3, kept loaded in-process, with Whisper tiny.en as the fallback.
- **Conversation.** `Ctrl+Alt+T` starts a hands-free spoken conversation with a language model
  running in Ollama on this computer. You can talk over a reply: "mm-hmm" lets it carry on,
  real words stop it.
- **Reading aloud.** `Ctrl+Alt+R` over a web page or document reads its main content aloud,
  word for word, through Windows UI Automation.
- **Voice commands for coding agents.** In a conversation, "open Claude Code in localflow",
  "ask Claude what the tests in localflow check" or "open localflow in Cursor". Every command
  is read back and runs only after a spoken "yes".

## Privacy

Speech recognition, the language model, the voices, turn-taking and page reading all run on
this computer. There is no account, analytics, telemetry or cloud sync, and nothing falls
back to a remote service. Nothing said in a conversation is stored or logged, and page text
goes only to the voice. LocalFlow never checks for updates and never downloads anything by
itself: models come from scripts you run, and new versions from
[GitHub Releases](https://github.com/liulawrence2002/localflow/releases).

The one exception is the coding-agent commands, and only after your spoken "yes" to a
read-back. "Ask Claude ..." sends your question to Claude Code, which sends it to Anthropic
under your own Claude subscription. "Open Claude Code ..." opens a terminal tab and sends
nothing by itself; what you then dictate into Claude Code goes to Anthropic, as in any Claude
Code session. LocalFlow itself still makes no network connection. See
[docs/PRIVACY.md](docs/PRIVACY.md) for exactly what is read, kept and sent.

## Status

This is pre-release software. What has and has not been tried:

- Dictation is the oldest and most complete path. The manual checklist in
  [TESTING.md](docs/TESTING.md) (Notepad, browser fields, VS Code and others) has not been
  worked through yet. The November 2026 changes (tap or hold, the clipboard restore once the
  target has read the text, the focus and administrator-window checks) are unit-tested, and
  an opt-in test runs the clipboard restore on the real Windows clipboard.
- Conversation has been measured end to end with real models and scripted audio: a median
  of 568 ms from the end of speech to the first audio of the reply with speech recognition
  on the GPU, and 993 ms on the CPU, in the same session
  ([benchmark](docs/benchmarks/CONVERSATION_BENCHMARK.md)). Nobody has yet talked over the
  assistant at a real microphone; if interrupting by voice misbehaves, the hotkey always
  works.
- Page reading has been checked on test pages in Edge and Chrome, not yet on real websites by
  a person.
- The coding-agent commands are built and tested with a stand-in agent. The Windows Terminal
  launch was checked with a stand-in program; Claude Code and Cursor have not yet been
  launched by voice from a real conversation.
- The release workflow and its installers have not run yet, and no installer has been tried
  on a clean computer.

## Install

You need Windows 11 (x64) and, for conversation and the clean-up after dictation,
[Ollama](https://ollama.com).

1. Download the installer from
   [GitHub Releases](https://github.com/liulawrence2002/localflow/releases):
   `LocalFlow_<version>_x64-setup.exe` installs for you only, into
   `%LOCALAPPDATA%\LocalFlow`; `LocalFlow_<version>_x64_en-US.msi` installs for all users.
   Check it against `SHA256SUMS` on the same page. The installers are not code-signed yet, so
   SmartScreen may warn: choose **More info**, then **Run anyway**.
2. Download the models. The installer includes only the small Whisper fallback; the scripts
   are in the install folder and put the models in `%LOCALAPPDATA%\LocalFlow`, checking each
   file against a pinned SHA-256. With the `.msi`, use `$env:ProgramFiles` instead of
   `$env:LOCALAPPDATA` in the first line:

   ```powershell
   $scripts = "$env:LOCALAPPDATA\LocalFlow\scripts"
   powershell -ExecutionPolicy Bypass -File "$scripts\Get-AsrModel.ps1"        # speech recognition, 549 MB
   powershell -ExecutionPolicy Bypass -File "$scripts\Get-PhononModel.ps1"     # the voice, about 65 MB
   powershell -ExecutionPolicy Bypass -File "$scripts\Get-VadModel.ps1"        # voice detection, 1.2 MB
   powershell -ExecutionPolicy Bypass -File "$scripts\Get-SmartTurnModel.ps1"  # end-of-turn model, 32 MB
   ```

3. Install Ollama and pull the default model: `ollama pull llama3.2:3b`.
4. Start LocalFlow from the Start menu. It lives in the tray; "Show LocalFlow" opens the
   settings window, whose Home screen lists anything still missing.

The [user guide](docs/USER_GUIDE.md#install-and-first-run) has the details, and
[MODEL_SETUP.md](docs/MODEL_SETUP.md) every model, its source and where it goes.

## Build from source

You need Windows 11, Node.js 22+, Rust 1.96 or newer, the
Microsoft C++ Build Tools (with CMake on `PATH`) and Ollama. The full walk-through is in the
[user guide](docs/USER_GUIDE.md#build-from-source) and
[DEVELOPMENT.md](docs/DEVELOPMENT.md).

```powershell
.\scripts\Install-Prereqs.ps1 -Install    # Node, npm, Git, Rust, C++ Build Tools
npm install

.\scripts\Get-WhisperRuntime.ps1           # whisper.cpp fallback programs (the build bundles them)
.\scripts\Get-AsrModel.ps1 -Model tiny.en  # Whisper fallback, 32 MB (the build bundles it)
.\scripts\Get-AsrModel.ps1                 # Parakeet speech recognition, 549 MB
.\scripts\Get-PhononModel.ps1              # the voice, about 65 MB
.\scripts\Get-VadModel.ps1                 # voice detection, 1.2 MB
.\scripts\Get-SmartTurnModel.ps1           # end-of-turn model, 32 MB

ollama pull llama3.2:3b

npm start                                  # builds once, then starts the tray app
```

In a checkout the scripts put everything in `.localflow-runtime`, which Git ignores, and a
build from source reads it from there. `npm start` runs `scripts\Start-LocalFlow.ps1`: it
builds the release executable if there is none, starts Ollama if it is not running, and
starts LocalFlow hidden in the tray.

## CPU and GPU builds

| Build | How                                      | Speech recognition                                                                        |
| ----- | ---------------------------------------- | ----------------------------------------------------------------------------------------- |
| CPU   | The installers, or `npm run tauri:build` | On the CPU. Runs anywhere. The default.                                                   |
| GPU   | From source: `npm run tauri:build:gpu`   | On a discrete GPU through Vulkan, falling back to the CPU. Needs the Vulkan SDK to build. |

The GPU build does not start on a computer without a Vulkan loader (`vulkan-1.dll`), which is
why the installers are CPU builds. It never uses an integrated GPU, and
`LOCALFLOW_ASR_DEVICE=cpu` switches it back to the CPU without a rebuild. Both write
`src-tauri\target\release\localflow.exe`; restart with `.\scripts\Start-LocalFlow.ps1 -Restart`
after building.

## Hotkeys

| Hotkey           | Action                                                         | Fallback               |
| ---------------- | -------------------------------------------------------------- | ---------------------- |
| `Ctrl+Alt+Space` | Dictation: tap, speak, pause; or hold. `Escape` cancels.       | `Ctrl+Alt+Shift+Space` |
| `Ctrl+Alt+T`     | Start a conversation; interrupt a reply; end the conversation. | `Ctrl+Alt+Shift+T`     |
| `Ctrl+Alt+R`     | Read the focused window aloud; press again to stop.            | `Ctrl+Alt+Shift+R`     |

## Dictation details

- **Formats deterministically before any model**: spoken punctuation ("comma", "new line",
  "bullet point"), explicit self-corrections ("meet Tuesday, actually Wednesday"), filler and
  stutter removal, and sentence capitals, while protecting URLs, email addresses and decimals.
- **Personalises from your settings**: exact replacements and snippets are applied, and your
  dictionary biases recognition.
- **Inserts safely**: each dictation has a session id; it never pastes into a different window
  than the one you started in, nor after the keyboard focus moved to another control that
  Windows tracks separately; a superseded dictation never pastes; and `Escape` cancels. It waits for you to let go of `Ctrl` and
  `Alt` before pasting, and says so instead of failing silently in an administrator window.
- **Never loses a transcript**: if the paste is skipped, "Copy last transcript" (tray or Home
  screen) gets it back. A local Ollama clean-up runs after the paste in the background
  (default `llama3.2:3b`; low-resource mode skips it).
- **Leaves the clipboard as it was**: the earlier text is put back once the target has read
  the dictation; nothing enters clipboard history, and a password manager's secret is never
  put back.

## Current limitations

- Windows only. Conversation, the voice, the end-of-turn judges and every spoken command are
  English only.
- The installers carry only the Whisper tiny.en fallback. Parakeet, the voice and the
  conversation models are downloaded by scripts you run, into `%LOCALAPPDATA%\LocalFlow`.
- The installers are not code-signed, and LocalFlow does not update itself.
- The microphone is the Windows default input device, and the hotkeys cannot be changed.
- Dictation pastes through the clipboard; only the earlier clipboard text is restored, not
  images or other rich formats. UI Automation insertion and start at login are not done.
- Conversation history is trimmed, not summarised. A resumed reply continues mid-word.
- Page reading cannot read Google Docs without its screen-reader mode, scanned PDFs, canvas
  apps or windows running as administrator, and very large pages time out.
- Codex and Cursor Agent cannot take commands yet; their command-line tools are not installed
  on the development machine.
- Summarising and explaining what is on screen are planned, not built
  ([roadmap](docs/VOICE_ASSISTANT_PLAN.md#roadmap)).

## Documentation

- [User guide](docs/USER_GUIDE.md): installing, the hotkeys, dictation, conversation, reading
  aloud, coding-agent commands, settings and
  [troubleshooting](docs/USER_GUIDE.md#troubleshooting).
- [Model setup](docs/MODEL_SETUP.md): every model, the scripts, where they go.
- [Development](docs/DEVELOPMENT.md): setup, building, tests, benchmarks, releases, and how to
  extend it.
- [Architecture](docs/ARCHITECTURE.md): modules, the conversation engine, the coding-agent
  path, and the Tauri boundary.
- [Privacy](docs/PRIVACY.md), [network policy](docs/network-policy.md) and
  [security](SECURITY.md).
- [Testing](docs/TESTING.md), and the [changelog](CHANGELOG.md).
- [Voice assistant plan](docs/VOICE_ASSISTANT_PLAN.md): decisions, measured baselines and
  milestones; [development/PLAN.md](docs/development/PLAN.md): what each slice built, with
  known limitations.
- Benchmarks: [conversation](docs/benchmarks/CONVERSATION_BENCHMARK.md),
  [speech recognition speed](docs/benchmarks/ASR_SPEED_BENCHMARK.md),
  [turn judges](docs/benchmarks/TURN_JUDGE_BENCHMARK.md),
  [speech recognition engines](docs/benchmarks/ASR_BENCHMARK.md).
- [All documentation](docs/README.md).

## Releases and packaging

Public installers are built by the [release workflow](.github/workflows/release.yml) from a
version tag: the CPU build, without the development runtime path, with the whisper.cpp
fallback, tiny.en and the Visual C++ runtime, published as a draft release with
`SHA256SUMS`. See [DEVELOPMENT.md](docs/DEVELOPMENT.md#releases).

To build installers locally for testing:

```powershell
.\scripts\Build-Installer.ps1
```

This writes `src-tauri\target\release\bundle\msi\LocalFlow_<version>_x64_en-US.msi` and
`src-tauri\target\release\bundle\nsis\LocalFlow_<version>_x64-setup.exe`. A local build keeps
the `dev-runtime` feature, so it also reads models from the checkout, and it does not ship the
Visual C++ runtime; use the release workflow for anything you hand to someone else.

## Contributing and security

Contributions are welcome: read [CONTRIBUTING.md](CONTRIBUTING.md) and the rules in
[AGENTS.md](AGENTS.md) first. Everyone taking part follows the
[code of conduct](CODE_OF_CONDUCT.md). Report security problems privately, as described in
[SECURITY.md](SECURITY.md).

## Licence

LocalFlow is licensed under either of

- the [MIT License](LICENSE-MIT), or
- the [Apache License, Version 2.0](LICENSE-APACHE),

at your option. Copyright (c) LocalFlow contributors.

LocalFlow includes and works with third-party code and models under their own licences,
listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). In particular, the Parakeet
speech model is NVIDIA's, under CC BY 4.0, and the optional Fish Audio S2 Pro voice is for
research and personal non-commercial use only. Neither is in the installer.
