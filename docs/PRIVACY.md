# Privacy

LocalFlow is local-first.

Default behavior:

- No account.
- No analytics.
- No telemetry.
- No cloud sync.
- No remote ASR or LLM calls.
- No screenshot capture.
- No permanent audio storage.
- No update checks.

The default speech model runs in-process and writes no file. Only the one-shot `whisper.cpp`
CLI fallback (low-resource mode, or after the in-process engine fails) writes a temporary
WAV file, in `%TEMP%\localflow`, and deletes the WAV and JSON output after transcription.
Files that a crash left behind are deleted the next time LocalFlow starts.

The floating overlay receives listening/processing/refining/inserted/error state and microphone level values. It does not receive or display transcript text.

The settings window's Privacy screen states the short version: dictations are not stored,
audio is deleted right after transcription, and nothing leaves this computer except a
coding-agent question you confirm by voice.

## Dictation and the Clipboard

- The last transcript is held in memory, for "Copy last transcript", until the next
  dictation or until LocalFlow closes. It is not stored; the only file a transcript passes
  through is the one-shot Whisper fallback's temporary JSON output described above.
- The text is pasted only into the window that had the focus when you started: the same
  window and process, with the same control holding the keyboard focus. Inside a web page,
  and in programs that draw their own controls, every field is the same control to Windows,
  so a move between them is not seen.
- The dictated text goes on the clipboard with the Windows formats that keep it out of
  clipboard history (`Win+V`) and cloud clipboard sync. The same goes for the earlier text
  LocalFlow puts back afterwards and for "Copy last transcript", so none of them adds a
  clipboard-history or cloud entry.
- Before pasting, LocalFlow checks the markers other programs put on what they copy. If a
  password manager or another program marked its content as private
  (`ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, or a
  clipboard-history or cloud-sync format saying no), LocalFlow does not read that content and
  does not put it back: the clipboard is left empty after the paste. Images and other
  formats are not put back either.
- If you copy something else after the paste, LocalFlow leaves it alone.

## Conversation Mode

Conversation mode is off until you switch it on with its hotkey, the tray, or the Home
screen, and it says when the microphone is open: the overlay stays on screen for the whole
conversation and the Home screen states the microphone's state in words.

While it is on:

- The microphone stays open, and audio is processed only on this computer: voice detection,
  speech recognition, the language model (Ollama on `127.0.0.1`) and the voice all run
  locally. Nothing is sent anywhere else, and there is no fallback that could.
- Audio is held in memory only for the turn being spoken, plus 0.4 s before it so the
  first word is not clipped. It is discarded once transcribed. No audio file is written.
- What you and the assistant said is held in memory for the length of the conversation so
  the assistant can follow it, and is cleared when the conversation ends. It is not written
  to the database, to a file, or to a log, and it is not sent to the overlay or the
  settings window.
- While the assistant is thinking, the microphone's audio is discarded. While it is
  speaking, the microphone is listened to only to notice you talking over it; that audio is
  kept for less than a second unless you do, in which case it becomes the start of your
  next turn.

It ends when you press the hotkey while the assistant is not talking, say goodbye, quit
the app, after 45 seconds without speech, after 10 minutes in total, or if the microphone
stops working. The microphone is closed in every case.

## Reading a Window Aloud

LocalFlow can read the window you are looking at aloud, word for word. It does this only
when you ask, at that moment: by pressing the reading hotkey over the window, or by saying
"read this page" (or "next paragraph", "go back") during a conversation. It never reads
a window in the background, on a timer, or because of something a program or a web page did.

How the window is read:

- Through Windows UI Automation, the interface screen readers use. No screenshot is taken.
- Only the window that has the focus when you ask.
- The text goes straight to the local voice. No language model sees it, it is not added to
  a conversation's memory, and it is not written to the database, a file, a log, or sent to
  the overlay or the settings window. Logs record counts and timings only.
- What LocalFlow remembers afterwards is where you stopped: an identifier computed from
  the page and the paragraph, for up to 16 pages, in memory until the app closes. It cannot
  be turned back into the page's address or text.

What it refuses to read, saying so aloud:

- A window while a password field has the keyboard focus. Password fields are never read
  in any window, and neither is anything typed into a text box.
- Private browsing windows (InPrivate, Incognito, Private Browsing).
- Password managers and authenticator apps, and Windows credential prompts.
- LocalFlow's own windows, and any window whose program it cannot identify.

Limits you should know about:

- Private windows are recognised by the English words in their title. A browser in another
  language, or one not on the list, is not recognised as private.
- Password managers are recognised by program name. A password vault open in an ordinary
  browser tab is read like any other page if you ask for it.
- Asking a browser for its accessibility information switches that feature on in the
  browser, which has a small ongoing cost there, and a browser that reports usage statistics
  to its maker may include the fact that an accessibility client connected.
- A page can contain text meant only for screen readers. Most text hidden by clipping is
  filtered out, but not every way of hiding text can be detected, so hidden text may
  occasionally be read aloud. Since nothing but your ears receives it, the harm is limited
  to hearing it.

## Voice Commands for Coding Agents

This is the one place where something you say can leave the computer, and only on your
spoken "yes" to a read-back.

- During a conversation you can say "open Claude Code in <project>", "ask Claude <question>"
  or "open <project> in Cursor". Only folders you added under Settings, Coding agents, can be
  named. Commands are matched by fixed rules on your own transcript; page text, model replies
  and the assistant's own voice cannot produce or confirm one.
- Every command is read back and runs only after a "yes" said once the read-back has
  finished. Anything else, or 15 s of silence, declines it.
- "Open Claude Code" opens a Windows Terminal tab running Claude Code in that folder;
  LocalFlow passes it nothing: no task, no transcript. What you then type or dictate into
  Claude Code goes to Anthropic under your own Claude subscription, as with any Claude Code
  session. Claude Code starts in its normal permission mode and asks you before it edits
  files or runs commands; LocalFlow never passes options that skip those prompts.
- "Ask Claude" gives Claude Code, on your subscription, the question you spoke plus a fixed
  instruction to answer in a few short spoken sentences; no page text, transcript history or
  other context is added. Claude Code sends that to Anthropic and, to answer, may read and
  search files in that project folder (it has read-only tools only), so the contents of files
  it reads are sent to Anthropic too, as in any Claude Code session. It runs without the
  folder's own hooks, tool servers or settings, and without saving the session. The answer is
  spoken and is never acted on. "Stop that" ends it.
- LocalFlow itself still makes no network connection for this; the agent does. Logs record
  which agent and which project name was used, never the question or the answer.
- The question run, and the `claude --version` and `--help` runs that find Claude Code, are
  started with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and `DISABLE_AUTOUPDATER=1`,
  which tell Claude Code to skip its update checks, error reports and other traffic it does
  not need to answer. The tab you open is your own session and gets neither.
- A conversation looks for the agents only when you have added a project. The Coding agents
  screen looks when you open it.
- "Open in Cursor" only opens the folder in the Cursor editor.

## Stored Fields

The settings are stored in `localflow.sqlite3` in `%APPDATA%\app.localflow.desktop`. If
that file is damaged, LocalFlow renames it to `localflow.sqlite3.corrupt-<date and time>` and
starts a new one; the damaged copy, with your old settings in it, stays in that folder until
you delete it. A settings row LocalFlow could only partly read is also kept, as
`settings.v1.unreadable` in the database.

Milestone 1 initializes SQLite tables for:

- `settings`: key, JSON value, update timestamp.
- `dictation_history`: id, creation timestamp, target application, raw transcript, final text, cleanup level.
- `dictionary_entries`: phrase, pronunciation hint, category, capitalization setting.

History storage is controlled by retention settings. Later milestones must enforce:

- Do not store history.
- Store transcript only.
- Store original and cleaned transcript.
- Delete after 24 hours.
- Delete after 7 days.
- Keep until manually deleted.

The shared domain layer includes build-tested retention helpers for disabled history, transcript-only history, original-and-cleaned history, and delete-after windows. Native SQLite writes still need to use those helpers before production dictation is enabled.

In the desktop app the settings window saves settings only through the native layer; a
failed save is reported and never falls back to browser storage. Only during browser-only
development (`npm run dev` outside the desktop app) are settings kept in the browser's
local storage.

## Context

Context awareness is optional and narrowly scoped. LocalFlow may read:

- Active app name.
- Window title.
- App category.
- Selected text.
- Limited text before and after the cursor.
- Sentence-start and code-field signals.

LocalFlow must never collect context from password or protected fields.

The shared context policy enforces separate gates for active-app metadata, accessibility text around the cursor, and selected text. It also trims before-cursor, after-cursor, and selected-text context to bounded lengths before those values can be sent to cleanup logic.

## Network Connections

Allowed:

- Localhost calls to Ollama, on `127.0.0.1:11434`.
- Localhost or file/process IPC for `whisper.cpp` sidecars.

Ollama can also run "cloud" models on Ollama's servers. LocalFlow refuses them: a model whose
tag is `cloud` or ends in `-cloud`, and, before any text is sent, any model whose description
from Ollama (`/api/show`) names a remote host or a remote model, whatever the model is
called. A model Ollama cannot describe is refused too. Such models are also left out of the
list in Settings > Models.

Not allowed during ordinary dictation:

- Cloud ASR.
- Cloud LLMs.
- Telemetry.
- Analytics.
- Silent remote fallback.

The single exception is a coding agent you start by voice and confirm (see "Voice Commands
for Coding Agents"): the agent, not LocalFlow, connects to its own service.

In the desktop app the settings window makes no network request at all: its content
security policy allows only the app's own IPC, and it asks the native layer which Ollama
models are installed. Only in a browser during development does the web code call Ollama
itself, and then the shared policy (`evaluateDictationNetworkUrl`) allows only plain HTTP to
`localhost`, `127.0.0.1` or `[::1]` on port 11434, with no user name or password in the URL,
and every request refuses redirects. Anything else is rejected before `fetch` is called.
Automated checks cover the policy and the Ollama provider.

Native dictation cleanup uses a configurable local Ollama model (default `llama3.2:3b`) at
`http://127.0.0.1:11434/api/generate`, and only ever a model Ollama runs on this computer.
Every native HTTP client ignores proxy settings and never follows a redirect.

## Diagnostics

The shared diagnostics export excludes dictated transcript text and local model paths by default. A transcript-inclusive export path exists only as an explicit option for user-approved troubleshooting.

Prepare export in Settings > Diagnostics also replaces your user folder (`C:\Users\<name>`)
with `<user folder>` and any microphone name with `<microphone>`, and it is only shown to you:
you copy it, and decide where it goes.

Messages about missing models, and the readiness checklist in the settings window, name
files and the script to run, never a folder, so a screenshot of them does not show your user
name.

## Logs and the Health File

LocalFlow writes two files about itself. Neither leaves this computer.

- **The log**, `%LOCALAPPDATA%\app.localflow.desktop\logs\localflow.log`. When it reaches
  1 MB it is renamed `localflow.log.1`, replacing the previous one, so there are at most two
  files of 1 MB each. It records phases, counts, timings, fixed messages and errors: which
  engine ran and how long it took, how many characters were pasted, which agent and project
  name a command used. It never records what was said, read aloud, asked or answered. Every
  line has your user folder replaced by `<user folder>` before it is written. It records the
  configured Ollama model's name, but not your microphone's name. If part of LocalFlow fails (a panic), the log gets the thread's name and the source
  file and line, never the panic's message, which could quote the text being worked on.
- **`desktop-health.json`**, in `%APPDATA%\app.localflow.desktop`: when LocalFlow started,
  which hotkeys registered, the last hotkey press, the last overlay message, the time,
  sample rate and channel count of the last recording, the last dictation error, and where
  the last panic happened. It does not keep the microphone's name, and user folders are
  replaced as in the log.

Settings > Diagnostics shows what the health file holds.

## Installation, Updates and Models

- **No update checks.** LocalFlow never checks for updates and never downloads anything by
  itself. New versions come from
  [GitHub Releases](https://github.com/liulawrence2002/localflow/releases), when you choose to
  download them.
- **WebView2.** The installers embed Microsoft's WebView2 bootstrapper
  (`embedBootstrapper`). If the WebView2 Runtime is missing (Windows 11 includes it), the
  bootstrapper downloads and installs it from Microsoft while LocalFlow is being installed.
  That download is Microsoft's installer, under Microsoft's terms; LocalFlow itself is not
  running yet.
- **Models.** The models are downloaded only when you run one of the `Get-*.ps1` scripts.
  Each script contacts Hugging Face or GitHub for one pinned file, checks its SHA-256, and
  sends nothing about you or your computer beyond what any download sends (your IP address
  and the request). An installed LocalFlow reads the models from
  `%LOCALAPPDATA%\LocalFlow`; uninstalling LocalFlow leaves that folder, so delete it by hand
  to remove the models.
- **Crash reports.** LocalFlow sends none. Windows Error Reporting is part of Windows and
  follows your Windows settings; see [SECURITY.md](../SECURITY.md#known-limitations) for how
  to exclude LocalFlow from it.
