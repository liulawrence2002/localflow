# Security

## Principles

- Keep dictated content local.
- Redact transcript text from logs by default.
- Treat selected text and cursor context as sensitive.
- Never execute arbitrary operating-system commands from command mode.
- Validate all local model responses before insertion.
- Protect against stale sessions inserting into the wrong window.

## Current Controls

- Native dictation assigns each recording a session id and runs the
  transcribe/refine/insert tail on a worker thread; a superseded or cancelled session
  is revalidated before every side effect and **never pastes** (`native_dictation.rs`
  `SessionRegistry`, `process_session`). Starting a new recording or a cancel command
  supersedes any in-flight worker.
- The mock/TS state machine prevents overlapping mock sessions (main-window verification
  path only; see `docs/REPO_AUDIT.md` §3 for which controls are native vs off-path).
- Mock pipeline never contacts a remote service.
- Shared network policy blocks remote model-provider URLs during ordinary dictation.
- Shared Ollama provider blocks remote URLs before fetch and validates JSON output before cleanup is accepted.
- Native cleanup calls only local Ollama at `127.0.0.1` with the configured local model (default `llama3.2:3b`); it never switches to a remote model.
- Native speech-to-text runs through local sidecars only. The persistent `whisper-server.exe` binds `127.0.0.1` on an ephemeral port and receives audio in memory; its output streams are discarded. Every engine has a deadline after which the process is killed, and a kill-on-close job object stops the sidecar if LocalFlow crashes.
- Loopback HTTP clients (whisper server and Ollama) ignore proxies and never follow redirects. Cloud-tagged Ollama models are refused.
- Dictionary terms reach whisper as one argument or one form field, so they cannot inject flags.
- The default speech model runs in-process through transcribe.cpp with its native logging disabled; it opens no socket and writes no file.
- Read-aloud text is treated like dictated text: it goes only to the supervised `s2.exe` sidecar on loopback, is never logged or emitted in events, and error messages report status codes and file names only. No voice cloning is used. The S2 Pro weights are research and personal non-commercial use only and are never bundled.
- Shared context policy blocks active-app, selected-text, and cursor-context collection for password or protected fields.
- Command-mode planning rejects operating-system command execution phrases.
- Native insertion revalidates the target window before pasting: LocalFlow captures the
  foreground window (HWND + owning process id) when dictation starts and, immediately
  before `Ctrl+V`, aborts if the current foreground window is not the same one. It **fails
  closed** — if the original or current target cannot be confirmed, it does not paste
  (`native_dictation.rs` `TargetWindow`/`target_matches`).
- Insertion planning also rejects protected fields and changed targets in the shared TS
  `insertionPlan` layer (used by the mock/verification path).
- Duplicate insertion guard prevents repeating the same generated text for one session
  (shared TS layer; the native path now supersedes stale sessions via `SessionRegistry`).
- The web UI runs under a content security policy: scripts and styles from the app only, connections only to the app's IPC. It has no other network access; the native layer asks local Ollama which models are installed.
- Conversation mode gives the language model no tools. Fixed phrases are matched in code before the model is asked, and the model's output is only ever spoken, so nothing it reads or writes can trigger an action.
- A window is read aloud only on the user's own request at that moment, only the focused window, and only after a capture policy has refused private browsing windows, password managers, credential prompts, LocalFlow's own windows and windows with a password field focused. Password and edit fields are never entered.
- Text read from a window goes only to the voice. It cannot reach the language model, the conversation memory, a log or an event: the type that carries it can only be created from page text, hides its content in debug output, and tests check that no page text appears in engine notices or in the next model request.
- The assistant's own voice must not be taken for the user's. While it speaks, the microphone passes through echo cancellation, speech has to pass a stricter and longer test than an ordinary start of turn, and an apparent interruption whose transcript is a stretch of the reply being spoken is discarded. While it is thinking, microphone audio is discarded.
- Tauri capabilities are per window. The settings window may call the app's declared commands; the overlay window may only listen for events. The opener and global-shortcut web permissions are not granted.
- Logs use redaction helpers for dictated content.
- Diagnostics export excludes dictated content and local model paths by default.
- Native clipboard paste fallback restores prior text clipboard content after a short delay, clears the dictated text when there was no text to restore, and leaves the clipboard alone if something else was copied in between. "Copy last transcript" uses the same history and cloud-sync exclusion as the paste. The dictated text is placed on the clipboard with the Windows flags that exclude it from clipboard history (Win+V) and cloud clipboard sync.
- A microphone that stops delivering audio no longer leaves a session listening: the end-of-speech, no-speech, and maximum-length timeouts run on silence too.
- The waveform overlay receives phase and audio-level metadata, not transcript text.

## Planned Controls

- Native UI Automation insertion.
- Native rich clipboard preservation, and stronger target verification (process start time
  to defeat PID reuse; UIA element identity; protected/read-only field detection).
- Paste-last-into-focus recovery via a dedicated hotkey (copy-last recovery already exists;
  the transcript is held in volatile memory only and never written to disk).
- Verify the identity of the process listening on the Ollama port, and close the short window in which another local process could take the whisper sidecar's reserved port.
- Restrict release builds from honouring `LOCALFLOW_WHISPER_*` overrides and the developer runtime path.
- Diagnostics export with transcript exclusion by default.
- Retention enforcement jobs.
