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

The current native hotkey path writes a temporary WAV file under the OS temp directory for `whisper.cpp` CLI processing, then deletes the WAV and JSON output after transcription.

The floating overlay receives listening/processing/refining/inserted/error state and microphone level values. It does not receive or display transcript text.

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
- "Open in Cursor" only opens the folder in the Cursor editor.

## Stored Fields

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

During browser-only development, settings edits use local browser storage as a fallback when Tauri commands are unavailable. The production desktop path is expected to persist settings through the native local storage layer.

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

- Localhost calls to Ollama.
- Localhost or file/process IPC for `whisper.cpp` sidecars.

Not allowed during ordinary dictation:

- Cloud ASR.
- Cloud LLMs.
- Telemetry.
- Analytics.
- Silent remote fallback.

The single exception is a coding agent you start by voice and confirm (see "Voice Commands
for Coding Agents"): the agent, not LocalFlow, connects to its own service.

Automated checks cover the shared network policy and the shared Ollama provider. Remote Ollama URLs are rejected before `fetch` is called.

The shared domain layer includes an allowlist check that permits localhost provider URLs and rejects remote URLs for ordinary dictation. The Ollama provider calls this policy for model discovery and cleanup requests. Native providers and future local server providers must keep using the same policy before any production network-capable path is enabled.

Native dictation cleanup uses a configurable local Ollama model (default `llama3.2:3b`) at `http://127.0.0.1:11434/api/generate`, and only ever a localhost model.

## Diagnostics

The shared diagnostics export excludes dictated transcript text and local model paths by default. A transcript-inclusive export path exists only as an explicit option for user-approved troubleshooting.
