# LocalFlow User Guide

LocalFlow is a Windows tray app for talking to your computer. It does four things:

- **Dictation.** Speak into any text field and LocalFlow pastes what you said.
- **Conversation.** A hands-free spoken conversation with a language model running on this
  computer.
- **Reading aloud.** It reads the focused window (a web page, a document) word for word.
- **Voice commands for coding agents.** In a conversation, open Claude Code or Cursor in one of
  your project folders, or ask Claude a read-only question about one.

Everything except the coding-agent commands runs on this computer. See [Privacy](#privacy).

Some of this has not been tried by a person yet. Interrupting the assistant by voice has
been tested with scripted audio only, page reading has been checked on test pages rather
than real sites, and no coding agent has yet been launched by voice from a real conversation.
Each section says where that applies.

## Install and first run

There are two ways to get LocalFlow: the installer from GitHub Releases, or building it from
source. The installer is the CPU build; speech recognition on a GPU needs a build from source
([below](#optional-speech-recognition-on-the-gpu)).

The installers have not yet been tried on a clean computer. If something is missing after
installing, please [report it](https://github.com/liulawrence2002/localflow/issues).

### Install from GitHub Releases

1. Download one installer from the
   [Releases page](https://github.com/liulawrence2002/localflow/releases):
   - `LocalFlow_<version>_x64-setup.exe` installs LocalFlow for you only, into
     `%LOCALAPPDATA%\LocalFlow`, without administrator rights;
   - `LocalFlow_<version>_x64_en-US.msi` installs it for all users, into
     `%ProgramFiles%\LocalFlow`.
2. Optionally check the download against `SHA256SUMS` from the same release:

   ```powershell
   Get-FileHash .\LocalFlow_0.1.0_x64-setup.exe -Algorithm SHA256
   ```

3. Run the installer. It is not code-signed yet, so Windows SmartScreen may say "Windows
   protected your PC"; choose **More info**, then **Run anyway**, once you have checked the
   hash. The installer includes the app, the Whisper tiny.en fallback model, the whisper.cpp
   programs and the Visual C++ runtime they need. If WebView2 is missing (Windows 11 includes
   it), the installer downloads it from Microsoft.
4. Download the models ([below](#models)) and install Ollama ([below](#ollama)).
5. Start LocalFlow from the Start menu. It runs in the tray; open the settings window with
   "Show LocalFlow" in the tray menu, or by starting LocalFlow again. While something it needs
   is missing, the Home screen shows a Setup list with the script or command for each item.

LocalFlow never checks for updates. To update, download the new installer from the Releases
page and run it; your settings and models are kept.

### Models

Every download is a script you run. LocalFlow never downloads anything by itself. Each
script downloads one pinned revision and checks every file against its SHA-256 before it
replaces anything; running it again is safe.

For an installed copy the scripts are in the install folder and put the models in
`%LOCALAPPDATA%\LocalFlow`. Windows does not run PowerShell scripts by default, so pass
`-ExecutionPolicy Bypass` for each command. With the `.msi`, use `$env:ProgramFiles` instead
of `$env:LOCALAPPDATA` in the first line:

```powershell
$scripts = "$env:LOCALAPPDATA\LocalFlow\scripts"
powershell -ExecutionPolicy Bypass -File "$scripts\Get-AsrModel.ps1"
powershell -ExecutionPolicy Bypass -File "$scripts\Get-PhononModel.ps1"
powershell -ExecutionPolicy Bypass -File "$scripts\Get-VadModel.ps1"
powershell -ExecutionPolicy Bypass -File "$scripts\Get-SmartTurnModel.ps1"
```

In a source checkout, run `.\scripts\Get-AsrModel.ps1` and so on; the models then go to the
checkout's `.localflow-runtime` folder, which Git ignores. Every script takes
`-Destination <folder>` to put them somewhere else.

| Script                            | Model                                           | Size        | Used for                                                                            |
| --------------------------------- | ----------------------------------------------- | ----------- | ----------------------------------------------------------------------------------- |
| `Get-AsrModel.ps1`                | Parakeet TDT 0.6B v3, speech recognition        | 549 MB      | Dictation and conversation. The default when installed.                             |
| `Get-AsrModel.ps1 -Model tiny.en` | Whisper tiny.en, small speech model             | 32 MB       | The fallback. The installer includes it; a build from source needs it.              |
| `Get-PhononModel.ps1`             | Phonon 31m, the voice                           | about 65 MB | Conversation and reading aloud. Required for both.                                  |
| `Get-VadModel.ps1`                | Silero VAD v6.2, tells speech from noise        | 1.2 MB      | Conversation. Without it a loudness rule is used, which any loud sound can trigger. |
| `Get-SmartTurnModel.ps1`          | Smart Turn v3.2, hears whether you had finished | 32 MB       | Conversation. Without it the words alone decide when you have finished.             |

"Read last transcript aloud" uses a separate, optional voice, Fish Audio S2 Pro (3.6 GB,
personal non-commercial use only). See [MODEL_SETUP.md](MODEL_SETUP.md#read-aloud-voice-fish-audio-s2-pro).
Where models are looked for, and every pinned source, is in [MODEL_SETUP.md](MODEL_SETUP.md).

### Ollama

Install Ollama from [ollama.com](https://ollama.com), then pull the default model and check
it:

```powershell
ollama pull llama3.2:3b
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\LocalFlow\scripts\Check-Ollama.ps1"
```

(From a checkout: `.\scripts\Check-Ollama.ps1`.) The same model answers in conversations and
does the background clean-up after dictation. You can pick any installed model in Settings >
Models. Models that run on Ollama's servers are refused and left out of that list: those with
a cloud tag (`:cloud`, `-cloud`), and any other that Ollama describes as running remotely,
whatever it is called. A model that does not fit in your GPU's memory answers too slowly for
conversation.

### Build from source

You need:

- Windows 11, with the WebView2 Runtime (Windows 11 includes it).
- Node.js 22 or newer and npm 10 or newer.
- Rust 1.96 or newer with Cargo, installed through rustup.
- Microsoft C++ Build Tools with the VCTools workload. Its CMake must be on `PATH` for the
  first build, which compiles the speech engine from C++ source.
- Ollama.

```powershell
.\scripts\Install-Prereqs.ps1 -Install
npm install
.\scripts\Get-WhisperRuntime.ps1             # the whisper.cpp programs the build bundles
.\scripts\Get-AsrModel.ps1 -Model tiny.en    # the fallback model the build bundles
```

`Install-Prereqs.ps1` checks Node, npm, Git, Rust and the C++ Build Tools, and with
`-Install` installs the missing ones through winget. It does not install Ollama, WebView2 or
the Vulkan SDK, and it does not put CMake on `PATH`. [DEVELOPMENT.md](DEVELOPMENT.md) covers
those steps. Then get the other models as above, and build and start:

```powershell
npm start
```

This runs `.\scripts\Start-LocalFlow.ps1`. The first time, it builds the release app with
`npm run tauri:build -- --no-bundle`, which takes a while. It starts Ollama if nothing is
listening on port 11434, then starts LocalFlow hidden in the tray. Open the settings window
with "Show LocalFlow" in the tray menu.

- `.\scripts\Start-LocalFlow.ps1 -Restart` restarts a copy that is already running, for
  example after a new build.
- `-SkipOllama` leaves Ollama alone.
- A desktop shortcut can point at `scripts\Start-LocalFlow.vbs`, which runs the same launcher
  without a terminal window.

A build from source also reads models from the checkout's `.localflow-runtime`, so keep the
checkout where it is.

### Optional: speech recognition on the GPU

The default build runs speech recognition on the CPU. If you have a discrete GPU, a GPU build
from source runs it there through Vulkan. In the same session on the development laptop (RTX
4070 Laptop), the median time from the end of your speech to the start of the reply was
568 ms on the GPU against 993 ms on the CPU
([benchmark](benchmarks/CONVERSATION_BENCHMARK.md#after-the-upgrades-gpu-recognition-and-smart-turn)).

```powershell
npm run tauri:build:gpu -- --no-bundle
.\scripts\Start-LocalFlow.ps1 -Restart
```

- Building it needs the Vulkan SDK ([DEVELOPMENT.md](DEVELOPMENT.md#gpu-builds-vulkan-sdk)).
- The GPU build will not start on a computer without a Vulkan loader (`vulkan-1.dll`). That
  is why it is not the default, and why the release installers are CPU builds.
- It uses the discrete GPU, preferring one whose name contains "NVIDIA", and never an
  integrated GPU. With no discrete GPU, or if loading or a run fails, it uses the CPU.
- It shares GPU memory with Ollama, using about 0.8 GB more. There is no check for free
  memory before loading.
- Both builds write `src-tauri\target\release\localflow.exe`, so `Start-LocalFlow.ps1` runs
  whichever you built last.

## Hotkeys

| Hotkey           | What it does                                                                                                        | If another program holds it                            |
| ---------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `Ctrl+Alt+Space` | Dictation: tap it and speak, and it stops when you pause; or hold it while you speak and let go. See below.         | `Ctrl+Alt+Shift+Space` works too; both are registered. |
| `Escape`         | Cancels a dictation while it is recording. Only registered then.                                                    |                                                        |
| `Ctrl+Alt+T`     | Starts a conversation. While the assistant is thinking or speaking, interrupts it. Otherwise ends the conversation. | `Ctrl+Alt+Shift+T`                                     |
| `Ctrl+Alt+R`     | Reads the focused window aloud. Press again to stop.                                                                | `Ctrl+Alt+Shift+R`                                     |

- The hotkeys cannot be changed yet. Settings > Hotkeys lists them and says whether each one
  is registered.
- One thing uses the microphone at a time. The dictation hotkey is ignored during a
  conversation, the conversation hotkey during a dictation, and the reading hotkey during a
  conversation (say "read this page" instead).

The tray menu has Show LocalFlow, Start or stop conversation, Copy last transcript, Read last
transcript aloud, Stop reading, and Quit. Closing the settings window only hides it; Quit
ends LocalFlow. Only one copy runs: starting LocalFlow again from the Start menu opens the
settings window of the one already running.

## Dictation

1. Click into a text field.
2. Press `Ctrl+Alt+Space`. A small waveform appears above the taskbar.
3. Either way works:
   - **Tap** the hotkey (let go within 700 ms) and speak. The dictation stops by itself when
     you pause for the End-of-speech wait: 900 ms unless you change it in Settings >
     Dictation. To stop sooner, tap again, at least 700 ms after the first tap.
   - **Hold** the hotkey while you speak and let go when you have finished. A held hotkey is
     never stopped by a pause, however long.
4. The text is pasted where you started.

What to expect:

- After a tap, if you say nothing for 2.5 s the waveform closes quietly. Every dictation, held
  or not, stops at 2 minutes.
- A press within 700 ms of a dictation stopping by itself is ignored. It was almost certainly
  meant to stop the dictation that had just stopped, and a new recording would replace it.
- `Escape` cancels while you are speaking. Nothing is pasted.
- If no speech model is installed, LocalFlow says so when you press the hotkey, before it
  records anything.
- The text is pasted only into the window that had the focus when you started, and only if
  the same control inside it still has the keyboard focus. If the focus moved, nothing is
  pasted, the overlay shows a red tint and the Home screen shows what happened. "Copy last
  transcript", in the tray or on the Home screen, gets the text back. Windows can tell one
  control from another only when each is a window of its own, as in classic Windows dialogs;
  inside a web page, and in programs that draw their own controls, a move from one field to
  another is not seen.
- The paste waits up to 1 s for you to let go of `Ctrl`, `Alt`, `Shift` and the Windows key,
  because `Ctrl+V` sent while `Alt` is down arrives as `Ctrl+Alt+V`. If a key is still down
  after that, nothing is pasted and LocalFlow says "Keys were still held".
- Windows does not let LocalFlow type into a program running as administrator, unless
  LocalFlow runs as administrator too. LocalFlow then says "Can't paste into an administrator
  window" instead of pasting; use Copy last transcript.
- If you start a new dictation while the previous one is still being transcribed, the
  previous one is not pasted, because the focus may have moved for the new one. Its text is
  kept for Copy last transcript until the new dictation produces text of its own. If the new
  one ends without any (cancelled, or nothing said), LocalFlow says "Previous dictation was
  not pasted. Use Copy last transcript."
- Before pasting, LocalFlow applies spoken punctuation ("comma", "new line", "bullet
  point"), explicit corrections ("meet Tuesday, actually Wednesday"), filler and stutter
  removal, and sentence capitals. URLs, email addresses and decimals are left alone.
- Your Dictionary terms help the recogniser, and your Replacements and Snippets are applied.
- After the paste, Ollama cleans up the text in the background. That does not change what
  was pasted; when it finishes, "Copy last transcript" returns the cleaned version.
  Low-resource mode skips it.
- Pasting goes through the clipboard and `Ctrl+V`. Your earlier clipboard text is put back
  once the program you pasted into has read the dictated text: at least 0.4 s after the
  paste, and later when LocalFlow runs in a Remote Desktop session. Images and other rich
  formats are not put back. Neither is anything a password manager or another program marked
  as private: the clipboard is cleared instead. If you copy something else in the meantime,
  it is left alone. Neither the dictated text nor the restored text is added to clipboard
  history (`Win+V`) or cloud clipboard sync.

## Conversation

### Starting and stopping

Start a conversation with `Ctrl+Alt+T`, the tray item "Start or stop conversation", or the
button on the Home screen. LocalFlow loads the speech model, warms it up, loads the voice
and opens the microphone. Starting a conversation stops any reading aloud.

It ends when:

- you press the hotkey while the assistant is not thinking or speaking;
- you say "goodbye" or a similar phrase (see [Spoken phrases](#spoken-phrases));
- you quit the app;
- nobody speaks for 45 s while it waits for you;
- it has lasted 10 minutes;
- the microphone reports an error or delivers nothing for 3 s;
- you confirm "open Claude Code", so that you can dictate your task.

A single turn is cut off at 60 s.

### What the overlay shows

The overlay stays on screen for the whole conversation. It shows no words: it shows the
sound the microphone hears while it is listening, and a slow sweep while the assistant
thinks and speaks. A brief red tint means the conversation ended on an error.

The Home screen in the settings window says the state in words: "Listening. The microphone
is open.", "Thinking. The microphone is open: keep talking to add to it." or "Speaking. The
microphone is open." Its button says what the hotkey would do now: End conversation, or
Interrupt.

### Taking turns

Speak, then stop. How long the assistant waits before answering depends on how finished
you sounded:

- about 400 ms after something that sounds finished;
- about 700 ms when it is unsure;
- about 1.4 s after you trail off mid-thought ("I think we should, um").

Smart Turn listens to your voice to judge this and your words move its answer one step.
Without Smart Turn the words decide alone, and a full stop the recogniser puts where you
only took a breath makes it answer too early more often. Turn-taking, the voice and all
spoken phrases are English only.

The assistant answers in a few spoken sentences. It cannot browse, open apps or see the
screen, and it is given no tools: nothing it says can make LocalFlow do anything. It
remembers the conversation until the conversation ends.

### Talking over a reply

Start speaking while the assistant is talking and the reply pauses. It is not thrown away
yet: what you say decides what happens next.

- **It stops** if you say a stop word ("stop", "wait", "no", "hold on", "cancel", "quiet"),
  a control phrase ("never mind", "say that again", "goodbye", "next paragraph"), or at
  least two words of your own. What you are saying becomes your next turn, and only the part
  of the reply you heard is remembered.
- **It carries on** where it paused if what it heard was "mm-hmm", "yeah", "okay", "got it",
  "go on", a cough, or its own voice coming back through the microphone. It also carries on
  if you go quiet for 1.5 s without having said enough to decide.
- If you are still talking 6 s after the reply paused, it stops.

Speech has to be clear and last about 0.3 s before it pauses the reply, so a short noise
does not. The hotkey always interrupts, whatever is said.

Known limitations: nobody has yet talked over the assistant at a real microphone; the tests
use scripted audio. A resumed reply comes back after about 0.6 to 1.2 s of silence and
continues mid-word. An interruption that repeats most of a sentence the assistant just said
is taken for its own echo. If the voice-cancelling part cannot start, the microphone is
ignored while the assistant speaks and only the hotkey interrupts.

### Adding to what you said

The microphone stays on while the assistant is thinking. If you carry on talking before its
reply starts, the reply it was preparing is dropped unheard and your two parts go to the
model as one message. One word of substance is enough here. "Read this page", a pause, then
"from the top" is understood as one command.

### Spoken phrases

These are handled by fixed rules, without the model, when said as a turn of their own (up
to eight words). Polite words at either end ("please", "thanks", "okay", "hey") are ignored.
A longer sentence that merely contains one, such as "How do I stop a process in
PowerShell?", goes to the model as a question.

| To                                       | Say, for example                                                                                     |
| ---------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| End the conversation                     | "goodbye", "bye", "end the conversation", "stop listening", "exit", "that's all", "we're done"       |
| Say nothing and keep listening           | "stop", "stop that", "be quiet", "never mind", "cancel", "cancel that", "that's enough", "okay stop" |
| Hear the last reply again, word for word | "repeat that", "say that again", "could you repeat that", "come again", "pardon", "what did you say" |
| Read the focused window                  | see [Reading aloud](#reading-a-window-aloud)                                                         |

If there is nothing to repeat, it says "I haven't said anything yet."

If Ollama cannot be reached it says "I can't reach the language model. Check that Ollama is
running." If the model is not installed it says "The language model isn't installed yet."

## Reading a window aloud

Press `Ctrl+Alt+R` over a web page, a document or most other windows, and LocalFlow reads
its main content aloud, word for word, with the conversation voice. Press it again, or choose
"Stop reading" in the tray, to stop. It needs the Phonon voice (`Get-PhononModel.ps1`).

- It reads only the window that has the focus when you ask, and only then.
- It reads the page's own words through Windows accessibility (UI Automation). No language
  model is involved and nothing is stored.
- It skips navigation, banners, footers, cookie dialogs and controls where it can tell them
  apart from the article.
- It carries on from where you last stopped on that page. Positions are remembered for up to
  16 pages until LocalFlow closes.

In a conversation, say:

| To read                                      | Say, for example                                                                                                           |
| -------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------- |
| From where you stopped, or what is on screen | "read this page", "read this", "read it to me", "read the article", "read the screen", "keep reading", "continue", "go on" |
| From the top                                 | "read from the top", "read from the beginning", "start over", "from the top"                                               |
| The next paragraph                           | "next", "next paragraph", "next one", "skip"                                                                               |
| The paragraph before                         | "go back", "back", "previous paragraph", "the one before"                                                                  |
| The last paragraph again                     | "read that again", "read it again", "again", "one more time"                                                               |

While it reads, "next", "go back", "read that again" and "from the top" interrupt and do
that; "go on" and "keep reading" let it carry on; "stop" stops it. At the end it says
"That's the end of the page."

It refuses, and says why:

- a window with a password field focused (password fields are never read in any window,
  and neither is anything typed into a text box);
- private browsing windows (InPrivate, Incognito, Private Browsing);
- password managers, authenticator apps and Windows credential prompts;
- LocalFlow's own windows, and windows whose program it cannot identify.

Limits:

- Private windows are recognised by the English words in their title only.
- Google Docs (unless its screen-reader support is on), scanned PDFs, games and other canvas
  apps, and windows running as administrator expose little or nothing.
- Pages above roughly 30,000 accessibility nodes time out. The first read of a long page can
  also time out while the browser prepares; asking again usually works.
- The page does not scroll or highlight as it is read.
- On pages that do not mark their main content, comments or sidebars can be included, and
  occasionally text meant only for screen readers is read.
- It has been checked on test pages in Edge and Chrome, not yet on real websites by a person.
  Firefox, PDFs, frames and Electron apps have not been tried.

## Voice commands for coding agents

### Setting up

1. Install Claude Code (with npm or its native installer) and sign in once by running
   `claude` in a terminal. Opening it in a tab also needs Windows Terminal. Cursor is
   optional.
2. In Settings > Coding agents, under Projects, type a project folder's full path.
   LocalFlow suggests spoken names from the folder name ("LocalFlow" gives "local flow").
   Edit them if you like, separating several with commas, and choose Add.
3. Check "Agents found". It lists Claude Code (where it was found, its version, and whether
   it can open a tab and answer questions), Cursor, Codex, Cursor Agent and Windows Terminal.
   "Check again" looks again after you install something. A conversation looks for the
   agents as it starts only when you have added a project, and reuses what it found for 30
   minutes; "Check again" updates that at once.

About project names:

- Only folders you add here can be named in a command.
- Names are compared with spaces and punctuation removed, so "local flow", "localflow" and
  "local-flow" are the same. Numbers match as words: "v2" and "v two" are the same.
- "The localflow project", "my localflow folder" and similar are understood.
- A name that is also an agent's name ("cursor", "claude code") is refused. A name two
  projects share is allowed, and you are asked which one you mean.
- A project can have up to eight names.
- Folders whose path contains `;`, `%` or `"` cannot be added: Windows Terminal would read
  those characters as commands or variables.

LocalFlow uses the real `claude.exe`: one on `PATH`, the one inside the npm package next to
npm's `claude.cmd`, or `%USERPROFILE%\.local\bin\claude.exe`. It never runs the `.cmd` or
`.ps1` shims.

### The commands

Say these during a conversation.

| Say                                                       | What happens after you say yes                                                                                                                                      |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| "Open Claude Code in localflow"                           | A Windows Terminal tab opens in that folder running Claude Code, and the conversation ends so you can dictate your task.                                            |
| "Open Claude Code in localflow and fix the failing tests" | The same. The task is not passed on: the read-back tells you to dictate it in the new tab.                                                                          |
| "Ask Claude what the tests in localflow check"            | Claude Code answers in the background, read-only. LocalFlow says "Asking Claude. Say stop that to cancel.", then "Claude says: ..."                                 |
| "Open localflow in Cursor"                                | The folder opens in the Cursor editor. The conversation carries on.                                                                                                 |
| "Stop that", "cancel", "never mind"                       | Said while a question is running: stops it and everything it started ("Stopped."). Said while the answer is being spoken: stops the speaking. The hotkey does both. |

Other ways of saying them work too: "open", "open up", "start", "launch", "fire up" or "run";
"Claude" for Claude Code; "open localflow in Claude Code"; and openers such as "please",
"can you" or "I'd like you to". Common mishearings of the agent's name, such as "cloud
code", are corrected; nothing else you say is changed.

For a question, the project is the one named in it, preferably after "in", "about" or
"for". If none is named, you are asked "Which project should Claude look at?". "Ask Claude to
explain the overlay in localflow" asks "explain the overlay in localflow".

Talking while a question runs does not stop it. LocalFlow reminds you: "Claude is still
working. Say stop that to cancel." A question is stopped after 120 s. The answer is cut to
about 30 seconds of speech, with code left out.

Codex and Cursor Agent are not set up yet. Naming them gets "Codex isn't set up for voice
commands yet." Asking Cursor a question is treated as asking Cursor Agent.

### The read-back and "yes"

Every command is read back first, for example "Open Claude Code in localflow? Then dictate
your task in the new tab. Shall I?" or "Ask Claude, in localflow: what the tests in
localflow check? Shall I send it?". A question is read back word for word.

- Only a yes said after the read-back has finished runs the command: "yes", "yeah", "yep",
  "sure", "go ahead", "do it", "go for it", "yes please", "that's right".
- Anything else declines it ("OK, I won't."). That includes "goodbye", "say that again" and
  "yes, but in the other project".
- 15 s of silence after the read-back declines it: "I didn't hear a yes, so I've left it."
- A yes begun while the read-back was still playing, or within 150 ms of its end, does not
  count. Say it again.
- If you talk over the read-back and cut it off, the command is dropped without a word.
- Only your own speech counts. Page text, the model's replies and the assistant's own voice
  never reach the command rules, and the language model plays no part in deciding what runs.

If a project name matches no project, or several, LocalFlow asks: "I don't know a project
called foobar. Which project do you mean? You have localflow and pdf viewer." Answer with a
name, the folder it is in ("the one in personal"), or "the first one" / "the second one" /
"the last one". "Never mind" drops the command. Then it reads the command back as usual.

### What is sent, and to whom

- **Open Claude Code.** LocalFlow sends nothing anywhere. It starts Windows Terminal with a
  new tab in the folder, running Claude Code with `--permission-mode default` (or `manual`,
  in versions that renamed that mode). No task and no transcript is put on the command line.
  Claude Code starts in its normal mode and asks you before it edits files or runs
  commands. LocalFlow never passes options that skip those prompts, and refuses any list of
  arguments that could. What you then type or dictate into Claude Code goes to Anthropic
  under your own Claude subscription, as in any Claude Code session.
- **Ask Claude.** The question you spoke is passed to Claude Code, which sends it to
  Anthropic under your subscription. It runs in the project folder with only the tools that
  read and search files, refuses anything that would need permission, and runs without the
  folder's own hooks, tool servers or settings and without saving the session. To answer,
  Claude may read files in that folder, and what it reads goes to Anthropic with the question,
  as in any Claude Code session. LocalFlow adds a fixed instruction asking for a short spoken
  answer. The answer is only spoken; it is never acted on. This run, and the
  `claude --version` and `--help` runs that find Claude Code, are given
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and `DISABLE_AUTOUPDATER=1`, which tell
  Claude Code to skip its update checks, error reports and other traffic it does not need.
  The tab you open is your own session and is started without them.
- **Open in Cursor.** The folder is opened in Cursor. Nothing else is passed.
- LocalFlow itself makes no network connection for any of this. Its logs record which agent
  and which project name were used, never the question or the answer.

### Dictating the task into Claude Code

After you say yes to "open Claude Code", the conversation ends and LocalFlow says "Opened
Claude Code in localflow. Dictate your task in the new tab and press Enter."

1. Check that the new Windows Terminal tab has the focus. Windows does not always bring it to
   the front; click into it if it is not.
2. Press `Ctrl+Alt+Space`, say the task, and pause. The text is pasted into Claude Code's
   prompt.
3. Read it, correct it if needed, and press Enter yourself. LocalFlow never presses Enter and
   never answers Claude Code's prompts.

The first time Claude Code opens in a folder it shows its own trust prompt.

Not yet tried: Claude Code and Cursor have not been launched by voice from a real
conversation. The Windows Terminal launch itself was checked with a stand-in program, which
received exactly the folder and `--permission-mode default`. A real `claude -p` answer, and
the wording of its sign-in error, have not been seen yet.

## Settings

The settings window follows the Windows light or dark theme. A setting the native app does
not use is not shown.

| Screen        | What it holds                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Home          | Setup, while something required is missing: each part, whether it is ready, and the script or command that installs it. A banner after a problem (below). The conversation's state in words, its Start / End / Interrupt button, which models it will use, and how the last conversation ended, unless you ended it with the hotkey or "goodbye". How to dictate, and a warning if the dictation hotkey is not registered. Whether reading aloud is available. Recovery: Copy last transcript, Show transcript, Read aloud with Fish S2, Stop reading. |
| Models        | Speech model file: leave it empty for Parakeet when it is installed, otherwise Whisper tiny.en; a `.gguf` model runs inside LocalFlow, a ggml Whisper `.bin` model in the whisper.cpp program. Language model: the Ollama model used for conversation and for the dictation clean-up, with "List installed models". Low-resource mode (below).                                                                                                                                                                                                         |
| Dictation     | How tap and hold work, and the End-of-speech wait: how long a pause ends a tapped dictation, 300 to 3000 ms, 900 ms by default. Holding the hotkey ignores it. LocalFlow records from the Windows default microphone; change that in Windows Settings > System > Sound.                                                                                                                                                                                                                                                                                |
| Hotkeys       | `Ctrl+Alt+Space`, `Ctrl+Alt+Shift+Space`, `Ctrl+Alt+T`, `Ctrl+Alt+R` and `Escape`: what each does and whether it is registered. They cannot be changed yet.                                                                                                                                                                                                                                                                                                                                                                                            |
| Dictionary    | Words and names the recogniser should expect in dictation, one word or phrase per row.                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Replacements  | Heard, Write, On: when dictation hears the first phrase as whole words, in any capitalisation, it writes the second instead.                                                                                                                                                                                                                                                                                                                                                                                                                           |
| Snippets      | Trigger, Expansion, On: say the trigger while dictating and the expansion is written in its place.                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Coding agents | Projects and their spoken names, the agents found, and example commands.                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Privacy       | Three facts: "Dictations are not stored", "Audio is deleted right after transcription", and "Nothing leaves this computer, except a coding-agent question you confirm by voice". Where the full statement is, with Copy link. There is nothing to switch.                                                                                                                                                                                                                                                                                              |
| Diagnostics   | The speech engine in use, timings for the last dictation and the last conversation turn, hotkey registration, "Settings storage" and "Last crash", each marked OK, Check, Not available or Problem. Refresh, and Prepare export (below).                                                                                                                                                                                                                                                                                                               |
| About         | The version from the build, the licence and the copyright.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |

- **Saving.** A change is saved a moment after you make it. The top of the window says
  "Saving...", then "All changes saved", or "Not saved" with a Try again button when a save
  failed. If the settings could not be loaded at all, the screens that change them stay
  locked, so a save cannot replace your settings with defaults.
- **Lists.** In Dictionary, Replacements and Snippets an edit is saved when you leave the
  field. Emptying a field shows an error and keeps the row; the remove button deletes it.
- **The Home banner.** After a dictation or reading problem, Home shows what happened and at
  what time, with a Copy last transcript button when the text was not pasted, or a hint about
  what to try. A dot on Home in the list of screens shows while it is up. Dismiss it with its
  close button; the next successful dictation also clears it.
- **Prepare export** refreshes Diagnostics, then writes them out as text with dictated text
  and model paths left out, your user folder replaced by `<user folder>` and microphone names
  by `<microphone>`. Read it before you share it; Copy puts it on the clipboard.

**Low-resource mode** skips the Ollama clean-up after dictation and, when tiny.en is
installed, uses the one-shot Whisper program instead of keeping Parakeet loaded. That also
applies to conversations, which then recognise speech more slowly and less accurately.

## Privacy

In brief; [PRIVACY.md](PRIVACY.md) is the full statement and [SECURITY.md](../SECURITY.md)
lists the controls.

- No account, analytics, telemetry or cloud sync. Speech recognition, the language model
  (Ollama on `127.0.0.1`) and the voices all run on this computer, and nothing falls back to
  a remote service.
- Dictation: the text goes only to the window you started in. Dictations are not stored: the
  last transcript is held in memory for Copy last transcript until the next dictation or
  until LocalFlow closes. Audio is deleted right after transcription.
- The clipboard: the dictated text and the earlier text LocalFlow puts back are kept out of
  clipboard history and cloud clipboard sync, and anything a password manager marked as
  private is cleared rather than put back.
- Conversation: audio is held in memory only for the turn being spoken. What you and the
  assistant say is kept in memory for the conversation and cleared when it ends; it is never
  written to disk, a log or an event.
- Reading aloud: only the focused window, only when you ask, and the text goes straight to
  the voice. LocalFlow remembers only where you stopped, as an identifier that cannot be
  turned back into the page.
- The one exception is the coding-agent commands, and only after your spoken "yes": a
  question goes to Anthropic through Claude Code under your subscription, and whatever you
  type or dictate into a Claude Code tab goes there too. LocalFlow itself still makes no
  network connection.
- Logs contain phases, counts and timings, never what was said, read or answered. They are
  kept on this computer only ([Logs](#logs)).
- LocalFlow never checks for updates and never downloads anything itself; models come only
  from the scripts you run, and new versions from GitHub Releases.

Settings > Privacy gives the short version, and where the full statement is.

## Troubleshooting

### Installing

- **SmartScreen says "Windows protected your PC".** The installers are not code-signed yet.
  Check the file against `SHA256SUMS` on the release page, then choose **More info** and
  **Run anyway**.
- **"Running scripts is disabled on this system".** Windows blocks PowerShell scripts by
  default. Run each script with `powershell -ExecutionPolicy Bypass -File <script>`, as in
  [Models](#models); that allows the one command only.
- **Something is reported missing.** The Home screen's Setup list, and messages about a
  missing model, name the file and the script that installs it. Run that script from the
  installed `scripts` folder (or the checkout's), then try again; LocalFlow notices new
  models without a restart, and the Setup list is checked again when you come back to the
  window.

### The settings window disappeared

Closing it only hides it; LocalFlow keeps running in the tray. Choose "Show LocalFlow" in the
tray menu, or start LocalFlow again from the Start menu. To end LocalFlow, choose Quit.

### A hotkey does nothing

Another program may hold it; LocalFlow then registers the `Shift` variant (see
[Hotkeys](#hotkeys)), and Settings > Hotkeys shows which ones are registered. During a
conversation the dictation and reading hotkeys are ignored on purpose. Right after a
dictation stops by itself, the dictation hotkey is ignored for 700 ms. If neither variant
works, check that `localflow.exe` is running (its icon is in the tray).

### Nothing is pasted

The Home screen's banner says why, with the time it happened. Copy last transcript, in the
banner, in the tray or on the Home screen, gets the text back in every case below.

- Click into the text field before pressing the hotkey. LocalFlow pastes through the
  clipboard and `Ctrl+V`, so the focused program must accept a paste.
- "Focus changed before insertion": the text is pasted only into the window, and the control
  inside it, that had the focus when you started. Moving to another window, or to another
  control that Windows tracks separately, skips the paste.
- "Keys were still held": a modifier key was still down a second after the dictation was
  ready. Let go of `Ctrl`, `Alt`, `Shift` and the Windows key once you have finished
  speaking.
- "Can't paste into an administrator window": the program runs as administrator and
  LocalFlow does not, so Windows drops its keystrokes.
- "Previous dictation was not pasted": you started a new dictation before the previous one
  was transcribed.
- **Remote desktops and clipboard managers.** Dictated text is placed on the clipboard with
  the Windows flags that keep it out of clipboard history and cloud sync. Tools that mirror
  the clipboard, including some remote desktop and virtual machine clipboard redirectors,
  may honour those flags and skip the text. "Copy last transcript" sets the same flags, so it
  does not get around such a tool; it does leave the text on this computer's clipboard for
  you to paste by hand. When LocalFlow itself runs inside a Remote Desktop session, it waits
  longer before it puts the earlier clipboard back.

### Dictation ends too soon or too late

- If you tapped the hotkey, a pause as long as the End-of-speech wait ends the dictation.
  Raise it in Settings > Dictation (up to 3000 ms) if it stops while you think, or lower it
  (down to 300 ms) if it waits too long. If you say nothing for 2.5 s after a tap, the
  waveform closes quietly.
- To pause as long as you like, hold the hotkey while you speak: only letting go, or the
  2-minute limit, ends it.
- A press shorter than 700 ms counts as a tap. A second tap stops a dictation only if it
  comes at least 700 ms after the first.

### Silence, or the waveform does not move

LocalFlow uses the Windows default input device. With a multi-channel microphone it uses the
loudest channel, and it ignores near-silent recordings. Check Windows Settings > System >
Sound, the input level, and that desktop apps may use the microphone (Privacy & security >
Microphone). A conversation ends, with a red tint on the overlay, if the microphone reports
an error or delivers nothing for 3 s; the Home screen then says "Conversation ended: the
microphone stopped working." A headset keeps the assistant from hearing itself.

The waveform appears only while LocalFlow is listening or working. If it never appears,
check that `localflow.exe` is running and that the hotkey is registered (Settings >
Hotkeys).

### Dictation is slow, or Diagnostics shows another engine

The `ASR engine` row in Settings > Diagnostics names the engine and model actually used.

- Parakeet is used when `parakeet-tdt-0.6b-v3-Q5_K_M.gguf` is installed (`Get-AsrModel.ps1`).
  A model path set in Settings > Models wins over it; clear that field to return to the
  default.
- Low-resource mode uses the one-shot Whisper program on purpose, so that nothing stays in
  memory; every dictation then pays for starting it.
- `fallback engine used` means the main engine failed for that dictation and the Whisper
  program took over.
- With a Whisper model selected, LocalFlow keeps `whisper-server.exe` running on
  `127.0.0.1`. Antivirus or application-control software may block it or its loopback
  listener; allow it. After a failed start LocalFlow waits one minute before trying the
  server again and uses the one-shot program meanwhile.
- If the Whisper programs or tiny.en are reported missing in an installed copy, reinstall
  LocalFlow. In a build from source, run `.\scripts\Get-WhisperRuntime.ps1` and
  `.\scripts\Get-AsrModel.ps1 -Model tiny.en`.

### No GPU, or the GPU build will not start

The GPU build needs a Vulkan loader (`vulkan-1.dll`). On a computer without one it does not
start at all; rebuild the default CPU version and restart:

```powershell
npm run tauri:build -- --no-bundle
.\scripts\Start-LocalFlow.ps1 -Restart
```

With a working GPU build you can still choose the CPU without rebuilding. LocalFlow reads
`LOCALFLOW_ASR_DEVICE` (`cpu` or `gpu`) when it starts; `LOCALFLOW_ASR_THREADS` sets the CPU
thread count (`auto` for the library default: the number of logical CPUs, at most 8):

```powershell
$env:LOCALFLOW_ASR_DEVICE = "cpu"
.\scripts\Start-LocalFlow.ps1 -Restart
```

A GPU build falls back to the CPU by itself when there is no discrete GPU, or when loading
or a run fails. Integrated GPUs are never used.

### The assistant says it can't reach the language model

"I can't reach the language model. Check that Ollama is running." means nothing answered on
`127.0.0.1:11434`. Start Ollama (from the Start menu, `ollama serve`, or
`Start-LocalFlow.ps1` in a checkout, which starts it) and run `Check-Ollama.ps1`. "The
language model isn't installed yet." means the model chosen in Settings > Models is missing:
`ollama pull llama3.2:3b`, or choose another. Dictation still pastes without Ollama; only the
background clean-up is skipped. Low-resource mode skips the clean-up on purpose.

### Conversation will not start

If the voice is missing, the Home screen says "The conversation voice is not installed. Run
scripts\Get-PhononModel.ps1." and the Start button is disabled.

The conversation hotkey does nothing while a dictation is recording; finish the dictation
first. Other failures show a red tint on the overlay, and the Home screen's Conversation
panel says "The last conversation ended with a problem at" the time, with the reason. The
usual ones are no default microphone and a speech model that cannot load (check the speech
model file in Settings > Models).

### It answers too early, or waits too long

Install Smart Turn (`Get-SmartTurnModel.ps1`) and Silero (`Get-VadModel.ps1`). The Home
screen lists which of them the next conversation will use. Without Silero, any loud sound
can start a turn.

### Read aloud is greyed out or reports something missing

The Home screen shows what is missing. `Get-FishModel.ps1` downloads the model and
`Build-FishS2.ps1` builds the engine (it needs the Visual Studio Build Tools); put both in the
same folder. Speech always takes a while: the first request loads the model (about 35 s on
the CUDA build) and synthesis is slower than real time. If it is far slower than that, the
engine is the CPU build; rebuild it with `-Backend cuda` on an NVIDIA GPU. The Vulkan build
measured slower than CPU and is not recommended. A failed engine start is not retried for one
minute. Stopping a reading while it is still "Preparing speech" stops the engine as well, so
the next reading loads the model again; so does five minutes without a reading.

### It mis-hears a project name

- Look at the project's spoken names in Settings > Coding agents and add the form you
  actually say. Spacing and punctuation do not matter; the words do.
- Choose names that do not sound like each other or like an agent's name.
- If two projects share a name, answer the question with the folder it is in, or "the first
  one". The question lists your projects by name when you have five or fewer.
- If the agent's name is the problem, say "Claude Code" in full.

### Windows Terminal does not come to the front

Windows does not always bring a new tab forward. Click into the tab before you press the
dictation hotkey. Dictation pastes only into the window that had the focus when you pressed
it, so you see where the text went before you press Enter.

If Windows Terminal is not found at all, LocalFlow says "Opening Claude Code needs Windows
Terminal, and I couldn't find it." Install it, then choose "Check again" in Settings > Coding
agents.

### Claude Code does not answer

- "Claude Code needs you to sign in. Run claude in a terminal and log in, then ask again."
- "The installed Claude Code is missing an option LocalFlow needs. Updating it may help."
  Settings > Coding agents lists the missing options.
- "Claude took too long, so I stopped it." The limit is 120 s.

### LocalFlow says its settings were reset

The settings are in `localflow.sqlite3` in `%APPDATA%\app.localflow.desktop`. If that file
is damaged, LocalFlow moves it aside as `localflow.sqlite3.corrupt-<date and time>`, starts
with a new one and default settings, and says so in a message box and in the "Settings
storage" row of Settings > Diagnostics. Set your settings again; delete the damaged copy when
you no longer need it. If only some saved values cannot be read (written by a newer or older
version, say), only those are reset or, for numbers, moved into range; a copy of the saved
settings is kept and LocalFlow says so in the same two places.

If LocalFlow cannot start at all, a message box says why. The same reason is in the log.

### Logs

LocalFlow writes its log to `%LOCALAPPDATA%\app.localflow.desktop\logs\localflow.log`. When
the file reaches 1 MB it becomes `localflow.log.1`, replacing the previous one, so there are
never more than two. The log has phases, counts, timings and fixed messages, never what was
said, read or answered, and your user folder is replaced by `<user folder>` in every line.
It does not name your microphone. Read it before you attach it to a bug report all the same.

If part of LocalFlow hit an internal error, Settings > Diagnostics shows "Last crash": which
part (its thread) and the source file and line, without any other text.

### Building from source

- **Rust or the C++ Build Tools are missing.** Run `.\scripts\Install-Prereqs.ps1 -Install`,
  then open a new terminal so the new tools are on `PATH`.
- **The build fails looking for `.localflow-runtime` files.** The build bundles the
  whisper.cpp programs and tiny.en: run `.\scripts\Get-WhisperRuntime.ps1` and
  `.\scripts\Get-AsrModel.ps1 -Model tiny.en`.
- **Port 1420 is busy.** Only the development loop (`npm run tauri:dev`) uses it; the tray
  app started by `Start-LocalFlow.ps1` does not. Stop the program using the port, or change
  it in `src-tauri/tauri.conf.json` and `vite.config.ts` together.

More for developers is in [DEVELOPMENT.md](DEVELOPMENT.md#gotchas).
