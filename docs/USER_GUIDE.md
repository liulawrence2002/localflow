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

LocalFlow is built from source on the computer that runs it. The models it uses are read
from the `.localflow-runtime` folder of that checkout, so keep the checkout where it is.

### What you need

- Windows 11, with the WebView2 Runtime (Windows 11 includes it).
- Node.js 22 or newer and npm 10 or newer.
- Rust stable with Cargo.
- Microsoft C++ Build Tools with the VCTools workload. Its CMake must be on `PATH` for the
  first build, which compiles the speech engine from C++ source.
- Ollama.
- The whisper.cpp program files in `.localflow-runtime\whisper\Release` (the build bundles
  them). No script fetches these; see [DEVELOPMENT.md](DEVELOPMENT.md#runtime-assets).

```powershell
.\scripts\Install-Prereqs.ps1 -Install
npm install
```

`Install-Prereqs.ps1` checks Node, npm, Git, Rust and the C++ Build Tools, and with
`-Install` installs the missing ones through winget. It does not install Ollama, WebView2 or
the Vulkan SDK, and it does not put CMake on `PATH`. [DEVELOPMENT.md](DEVELOPMENT.md) covers
those steps.

### Models

Every download is a script you run. LocalFlow never downloads anything by itself. The files
go to `.localflow-runtime\models`, which Git ignores. Phonon, Silero and Smart Turn come from
a pinned revision and are checked against a SHA-256 hash; the speech models are checked by
size.

| Script                                      | Model                                           | Size        | Used for                                                                            |
| ------------------------------------------- | ----------------------------------------------- | ----------- | ----------------------------------------------------------------------------------- |
| `.\scripts\Get-AsrModel.ps1`                | Parakeet TDT 0.6B v3, speech recognition        | 549 MB      | Dictation and conversation. The default when installed.                             |
| `.\scripts\Get-AsrModel.ps1 -Model tiny.en` | Whisper tiny.en, small speech model             | 32 MB       | The fallback. The installer bundles it, so the build needs it.                      |
| `.\scripts\Get-PhononModel.ps1`             | Phonon 31m, the voice                           | about 65 MB | Conversation and reading aloud. Required for both.                                  |
| `.\scripts\Get-VadModel.ps1`                | Silero VAD v6.2, tells speech from noise        | 1.2 MB      | Conversation. Without it a loudness rule is used, which any loud sound can trigger. |
| `.\scripts\Get-SmartTurnModel.ps1`          | Smart Turn v3.2, hears whether you had finished | 32 MB       | Conversation. Without it the words alone decide when you have finished.             |

"Read last transcript aloud" uses a separate, optional voice, Fish Audio S2 Pro (3.6 GB,
personal non-commercial use only). See [MODEL_SETUP.md](MODEL_SETUP.md#read-aloud-voice-fish-audio-s2-pro).

### Ollama

Install Ollama, then pull the default model and check it:

```powershell
ollama pull llama3.2:3b
.\scripts\Check-Ollama.ps1
```

The same model answers in conversations and does the background clean-up after dictation.
You can pick any installed model in Settings > Models. Models with a cloud tag (`:cloud`,
`-cloud`) are refused, because they run on Ollama's servers. A model that does not fit in
your GPU's memory answers too slowly for conversation.

### Build and start

```powershell
npm start
```

This runs `.\scripts\Start-LocalFlow.ps1`. The first time, it builds the release app with
`npm run tauri:build`, which takes a while. It starts Ollama if nothing is listening on port
11434, then starts LocalFlow hidden in the tray. Open the settings window with "Show
LocalFlow" in the tray menu.

- `.\scripts\Start-LocalFlow.ps1 -Restart` restarts a copy that is already running, for
  example after a new build.
- `-SkipOllama` leaves Ollama alone.
- A desktop shortcut can point at `scripts\Start-LocalFlow.vbs`, which runs the same launcher
  without a terminal window.

### Optional: speech recognition on the GPU

The default build runs speech recognition on the CPU. If you have a discrete GPU, a GPU build
runs it there through Vulkan. In the same session on the development laptop (RTX 4070 Laptop),
the median time from the end of your speech to the start of the reply was 568 ms on the GPU
against 993 ms on the CPU
([benchmark](benchmarks/CONVERSATION_BENCHMARK.md#after-the-upgrades-gpu-recognition-and-smart-turn)).

```powershell
npm run tauri:build:gpu
.\scripts\Start-LocalFlow.ps1 -Restart
```

- Building it needs the Vulkan SDK ([DEVELOPMENT.md](DEVELOPMENT.md#gpu-builds-vulkan-sdk)).
- The GPU build will not start on a computer without a Vulkan loader (`vulkan-1.dll`). That
  is why it is not the default.
- It uses the discrete GPU, preferring one whose name contains "NVIDIA", and never an
  integrated GPU. With no discrete GPU, or if loading or a run fails, it uses the CPU.
- It shares GPU memory with Ollama, using about 0.8 GB more. There is no check for free
  memory before loading.
- Both builds write `src-tauri\target\release\localflow.exe`, so `Start-LocalFlow.ps1` runs
  whichever you built last.

## Hotkeys

| Hotkey           | What it does                                                                                                        | If another program holds it                            |
| ---------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| `Ctrl+Alt+Space` | Dictation: tap to start, then pause when you have finished. Press again to stop early, or hold and release.         | `Ctrl+Alt+Shift+Space` works too; both are registered. |
| `Escape`         | Cancels a dictation while it is recording. Only registered then.                                                    |                                                        |
| `Ctrl+Alt+T`     | Starts a conversation. While the assistant is thinking or speaking, interrupts it. Otherwise ends the conversation. | `Ctrl+Alt+Shift+T`                                     |
| `Ctrl+Alt+R`     | Reads the focused window aloud. Press again to stop.                                                                | `Ctrl+Alt+Shift+R`                                     |

- The hotkeys cannot be changed yet. Settings > Hotkeys shows them but does not change them.
- The Home screen shows which conversation and reading hotkeys were registered.
- One thing uses the microphone at a time. The dictation hotkey is ignored during a
  conversation, the conversation hotkey during a dictation, and the reading hotkey during a
  conversation (say "read this page" instead).

The tray menu has Show LocalFlow, Start or stop conversation, Copy last transcript, Read last
transcript aloud, Stop reading, and Quit.

## Dictation

1. Click into a text field.
2. Tap `Ctrl+Alt+Space`. A small waveform appears above the taskbar.
3. Speak, then pause. About 550 ms of silence ends the dictation. You can also press the
   hotkey again, or hold it while you speak and release it (a press of 700 ms or more counts
   as a hold).
4. The text is pasted where you started.

What to expect:

- If you say nothing for 2.5 s the waveform closes quietly. A dictation stops at 120 s.
- `Escape` cancels while you are speaking. Nothing is pasted.
- The text is pasted only into the window that had the focus when you started. If the focus
  moved, nothing is pasted and the overlay shows a red tint. "Copy last transcript", in the
  tray or on the Home screen under Recovery, gets the text back.
- Before pasting, LocalFlow applies spoken punctuation ("comma", "new line", "bullet
  point"), explicit corrections ("meet Tuesday, actually Wednesday"), filler and stutter
  removal, and sentence capitals. URLs, email addresses and decimals are left alone.
- Your Dictionary terms help the recogniser, and your Replacements and Snippets are applied.
- After the paste, Ollama cleans up the text in the background. That does not change what
  was pasted; when it finishes, "Copy last transcript" returns the cleaned version.
  Low-resource mode skips it.
- Pasting goes through the clipboard and `Ctrl+V`. Your earlier clipboard text is put back
  afterwards; images and other rich formats are not. The dictated text is kept out of
  clipboard history (`Win+V`) and cloud clipboard sync.

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
   "Check again" looks again after you install something.

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
  answer. The answer is only spoken; it is never acted on.
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

The settings window has these screens. Some settings are shown but not yet used by the
native app; they are marked below.

| Screen        | What it holds                                                                                                                                                                                                                                                                                                                                              |
| ------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Home          | The app's state; the conversation's state in words, its Start / End / Interrupt button, and which models it will use; whether reading aloud is available; Recovery (Copy last transcript, Reveal preview, Read aloud, Stop).                                                                                                                               |
| Models        | Whisper model path: leave it empty for Parakeet when installed, otherwise tiny.en; a `.gguf` model runs in-process, a ggml `.bin` model in the whisper.cpp program. Ollama model: used for both conversation and dictation clean-up. Low-resource mode (below). Threads: not used yet; see `LOCALFLOW_ASR_THREADS` in [Troubleshooting](#troubleshooting). |
| Microphone    | Device, voice activity detection, end-of-speech time and maximum length. Not used yet: LocalFlow uses the Windows default microphone and fixed timings.                                                                                                                                                                                                    |
| Hotkeys       | Shows the dictation hotkeys. The activation mode buttons are not used yet; tap and hold both always work.                                                                                                                                                                                                                                                  |
| Dictionary    | Words and names to help the recogniser in dictation.                                                                                                                                                                                                                                                                                                       |
| Replacements  | Exact replacements applied to dictated text.                                                                                                                                                                                                                                                                                                               |
| Snippets      | Phrases that expand into longer text when dictated.                                                                                                                                                                                                                                                                                                        |
| Styles        | Style profiles. Saved, but not yet applied to the clean-up.                                                                                                                                                                                                                                                                                                |
| Coding agents | Projects and their spoken names, the agents found, and example commands.                                                                                                                                                                                                                                                                                   |
| Privacy       | Context and retention controls. Not used yet by the native app; the history setting is disabled.                                                                                                                                                                                                                                                           |
| History       | Empty: dictations are not stored.                                                                                                                                                                                                                                                                                                                          |
| Diagnostics   | The speech engine in use, timings for the last dictation and the last conversation turn, and a diagnostics export that leaves out dictated text and model paths.                                                                                                                                                                                           |
| About         | Version and build details.                                                                                                                                                                                                                                                                                                                                 |

**Low-resource mode** skips the Ollama clean-up after dictation and, when tiny.en is
installed, uses the one-shot Whisper program instead of keeping Parakeet loaded. That also
applies to conversations, which then recognise speech more slowly and less accurately.

## Privacy

In brief; [PRIVACY.md](PRIVACY.md) is the full statement and [SECURITY.md](../SECURITY.md)
lists the controls.

- No account, analytics, telemetry or cloud sync. Speech recognition, the language model
  (Ollama on `127.0.0.1`) and the voices all run on this computer, and nothing falls back to
  a remote service.
- Dictation: the text goes only to the window you started in. Nothing is stored in a
  history.
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
- Logs contain phases, counts and timings, never what was said, read or answered.

## Troubleshooting

See also [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for dictation, the Whisper fallback and the
Fish read-aloud voice.

### No GPU, or the GPU build will not start

The GPU build needs a Vulkan loader (`vulkan-1.dll`). On a computer without one it does not
start at all; rebuild the default CPU version and restart:

```powershell
npm run tauri:build
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
`127.0.0.1:11434`. Start Ollama (`ollama serve`, or `Start-LocalFlow.ps1`, which starts it)
and run `.\scripts\Check-Ollama.ps1`. "The language model isn't installed yet." means the
model chosen in Settings > Models is missing: `ollama pull llama3.2:3b`, or choose another.
Dictation still pastes without Ollama; only the background clean-up is skipped.

### Conversation will not start

If the voice is missing, the Home screen says "The conversation voice is not installed. Run
scripts\Get-PhononModel.ps1." and the Start button is disabled.

The conversation hotkey does nothing while a dictation is recording; finish the dictation
first. Other failures show a red tint on the overlay, but the reason is not shown on screen
yet. The usual ones are no default microphone and a speech model that cannot load (check
the Whisper model path in Settings > Models).

### The microphone

LocalFlow uses the Windows default input device. With a multi-channel microphone it uses the
loudest channel. Check Windows Settings > System > Sound, and that desktop apps may use the
microphone (Privacy & security > Microphone). A conversation ends, with a red tint on the
overlay, if the microphone reports an error or delivers nothing for 3 s. A headset keeps the
assistant from hearing itself.

### It answers too early, or waits too long

Install Smart Turn (`Get-SmartTurnModel.ps1`) and Silero (`Get-VadModel.ps1`). The Home
screen lists which of them the next conversation will use. Without Silero, any loud sound
can start a turn.

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

### A hotkey does nothing

Another program may hold it; LocalFlow then registers the `Shift` variant (see
[Hotkeys](#hotkeys)), and the Home screen shows which one. During a conversation the
dictation and reading hotkeys are ignored on purpose.
