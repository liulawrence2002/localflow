# LocalFlow

LocalFlow is a local-first Windows voice app built with Tauri 2, Rust, React and TypeScript.
It runs quietly in the tray and does four things:

- **Dictation.** Tap `Ctrl+Alt+Space`, speak, pause, and the text is pasted into the field
  where you started. Speech recognition is NVIDIA Parakeet TDT 0.6B v3, kept loaded
  in-process, with Whisper tiny.en as the fallback.
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
goes only to the voice.

The one exception is the coding-agent commands, and only after your spoken "yes" to a
read-back. "Ask Claude ..." sends your question to Claude Code, which sends it to Anthropic
under your own Claude subscription. "Open Claude Code ..." opens a terminal tab and sends
nothing by itself; what you then dictate into Claude Code goes to Anthropic, as in any Claude
Code session. LocalFlow itself still makes no network connection. See
[docs/PRIVACY.md](docs/PRIVACY.md) for exactly what is read, kept and sent.

## Status

This is a development build, not a release. What has and has not been tried:

- Dictation is the oldest and most complete path. The manual checklist in
  [TESTING.md](docs/TESTING.md) (Notepad, browser fields, VS Code and others) has not been
  worked through yet.
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

## Quick start

You need Windows 11, Node.js 22+, Rust, the Microsoft C++ Build Tools (with CMake on
`PATH`) and Ollama. The full walk-through is in the [user guide](docs/USER_GUIDE.md#install-and-first-run).

```powershell
.\scripts\Install-Prereqs.ps1 -Install    # Node, npm, Git, Rust, C++ Build Tools
npm install

.\scripts\Get-AsrModel.ps1                 # Parakeet speech recognition, 549 MB
.\scripts\Get-AsrModel.ps1 -Model tiny.en  # Whisper fallback, 32 MB (bundled by the build)
.\scripts\Get-PhononModel.ps1              # the voice, about 65 MB
.\scripts\Get-VadModel.ps1                 # voice detection, 1.2 MB
.\scripts\Get-SmartTurnModel.ps1           # end-of-turn model, 32 MB

ollama pull llama3.2:3b

npm start                                  # builds once, then starts the tray app
```

Every model download is a script you run; LocalFlow never downloads anything itself. Models
go to `.localflow-runtime\models`, which Git ignores. The build also needs the whisper.cpp
program files in `.localflow-runtime\whisper\Release`; see
[DEVELOPMENT.md](docs/DEVELOPMENT.md#runtime-assets).

`npm start` runs `scripts\Start-LocalFlow.ps1`: it builds the release app if there is none,
starts Ollama if it is not running, and starts LocalFlow hidden in the tray. Open the
settings window from the tray icon.

## CPU and GPU builds

| Build | Command                   | Speech recognition                                                                        |
| ----- | ------------------------- | ----------------------------------------------------------------------------------------- |
| CPU   | `npm run tauri:build`     | On the CPU. Runs anywhere. The default.                                                   |
| GPU   | `npm run tauri:build:gpu` | On a discrete GPU through Vulkan, falling back to the CPU. Needs the Vulkan SDK to build. |

The GPU build does not start on a computer without a Vulkan loader (`vulkan-1.dll`), which is
why it is not the default. It never uses an integrated GPU, and `LOCALFLOW_ASR_DEVICE=cpu`
switches it back to the CPU without a rebuild. Both write
`src-tauri\target\release\localflow.exe`; restart with `.\scripts\Start-LocalFlow.ps1 -Restart`
after building.

## Hotkeys

| Hotkey           | Action                                                          | Fallback               |
| ---------------- | --------------------------------------------------------------- | ---------------------- |
| `Ctrl+Alt+Space` | Dictation: tap, speak, pause. `Escape` cancels while recording. | `Ctrl+Alt+Shift+Space` |
| `Ctrl+Alt+T`     | Start a conversation; interrupt a reply; end the conversation.  | `Ctrl+Alt+Shift+T`     |
| `Ctrl+Alt+R`     | Read the focused window aloud; press again to stop.             | `Ctrl+Alt+Shift+R`     |

## Dictation details

- **Formats deterministically before any model**: spoken punctuation ("comma", "new line",
  "bullet point"), explicit self-corrections ("meet Tuesday, actually Wednesday"), filler and
  stutter removal, and sentence capitals, while protecting URLs, email addresses and decimals.
- **Personalises from your settings**: exact replacements and snippets are applied, and your
  dictionary biases recognition.
- **Inserts safely**: each dictation has a session id; it never pastes into a different window
  than the one you started in, a superseded dictation never pastes, and `Escape` cancels.
- **Never loses a transcript**: if the paste is skipped, "Copy last transcript" (tray or Home
  screen) gets it back. A local Ollama clean-up runs after the paste in the background
  (default `llama3.2:3b`; low-resource mode skips it).

## Current limitations

- Windows only. Conversation, the voice, the end-of-turn judges and every spoken command are
  English only.
- LocalFlow is built from source on the computer that runs it. Parakeet, the voice and the
  conversation models are read from the checkout's `.localflow-runtime` folder and are not in
  the installer.
- The microphone is the Windows default input device. The Microphone, Hotkeys and Privacy
  settings screens are not read by the native app yet, and hotkeys cannot be changed.
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
  aloud, coding-agent commands, settings and troubleshooting.
- [Development](docs/DEVELOPMENT.md): setup, building, tests, benchmarks, and how to extend it.
- [Architecture](docs/ARCHITECTURE.md): modules, the conversation engine, the coding-agent
  path, and the Tauri boundary.
- [Privacy](docs/PRIVACY.md) and [security](SECURITY.md).
- [Model setup](docs/MODEL_SETUP.md), [testing](docs/TESTING.md),
  [troubleshooting](docs/TROUBLESHOOTING.md).
- [Voice assistant plan](docs/VOICE_ASSISTANT_PLAN.md): decisions, measured baselines and
  milestones; [development/PLAN.md](docs/development/PLAN.md): what each slice built, with
  known limitations.
- Benchmarks: [conversation](docs/benchmarks/CONVERSATION_BENCHMARK.md),
  [speech recognition speed](docs/benchmarks/ASR_SPEED_BENCHMARK.md),
  [turn judges](docs/benchmarks/TURN_JUDGE_BENCHMARK.md),
  [speech recognition engines](docs/benchmarks/ASR_BENCHMARK.md).
- [All documentation](docs/README.md).

## Packaging

```powershell
.\scripts\Build-Installer.ps1
```

A successful build creates:

- `src-tauri\target\release\localflow.exe`
- `src-tauri\target\release\bundle\msi\LocalFlow_0.1.0_x64_en-US.msi`
- `src-tauri\target\release\bundle\nsis\LocalFlow_0.1.0_x64-setup.exe`

The installer bundles the Whisper executables, their DLLs and the tiny English model. It has
not been smoke-tested on a clean computer.
