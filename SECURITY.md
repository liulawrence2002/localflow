# Security

## Reporting a vulnerability

Please report security problems privately through GitHub:
[open a private vulnerability report](https://github.com/liulawrence2002/localflow/security/advisories/new)
(the repository's **Security** tab, then **Report a vulnerability**). Do not open a public
issue, and do not include transcripts, dictated or page text, or paths with your user name.

Say which version or commit you used, what you did, and what an attacker could do. The
maintainers will acknowledge the report on GitHub, keep you informed while it is fixed, and
credit you in the advisory if you would like. Only the latest release and the `master` branch
receive fixes.

LocalFlow never checks for updates and never downloads anything by itself: new versions,
including security fixes, come from
[GitHub Releases](https://github.com/liulawrence2002/localflow/releases).

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
  is revalidated before every side effect and **never pastes**
  (`native_dictation/session.rs` `SessionRegistry`, `process_session` in
  `native_dictation/mod.rs`). Starting a new recording or a cancel command supersedes any
  in-flight worker. A superseded transcript is kept in memory for "Copy last transcript".
- The mock/TS state machine prevents overlapping mock sessions (main-window verification
  path only; see [the archived repository audit](docs/archive/REPO_AUDIT.md) §3 for which
  controls are native and which are off the production path). The native mock dictation
  commands exist only in debug builds; a release build neither has them nor can grant them.
- Mock pipeline never contacts a remote service.
- Shared network policy blocks remote model-provider URLs during ordinary dictation.
- Shared Ollama provider blocks remote URLs before fetch and validates JSON output before cleanup is accepted.
- Native cleanup calls only local Ollama at `127.0.0.1` with the configured local model (default `llama3.2:3b`); it never switches to a remote model.
- Native speech-to-text runs through local sidecars only. The persistent `whisper-server.exe` binds `127.0.0.1` on an ephemeral port and receives audio in memory; its output streams are discarded. Every engine has a deadline after which the process is killed, and a kill-on-close job object stops the sidecar if LocalFlow crashes.
- Loopback HTTP clients (whisper server, Ollama and the `s2.exe` voice) ignore proxies and never follow redirects. Ollama models that run on Ollama's servers are refused: by their cloud tag, and before any text is sent by Ollama's own description of the model (`/api/show`: a `remote_host` or `remote_model` refuses it whatever its name, and an unclear answer refuses it too).
- The web UI's own Ollama calls, made only in browser development, allow only `http://localhost:11434` (or `127.0.0.1`, `[::1]`) without credentials, and treat a redirect as an error.
- Dictionary terms reach whisper as one argument or one form field, so they cannot inject flags.
- The default speech model runs in-process through transcribe.cpp with its native logging disabled; it opens no socket and writes no file.
- Read-aloud text is treated like dictated text: it goes only to the supervised `s2.exe` sidecar on loopback, is never logged or emitted in events, and error messages report status codes and file names only. No voice cloning is used. The S2 Pro weights are research and personal non-commercial use only and are never bundled.
- Shared context policy blocks active-app, selected-text, and cursor-context collection for password or protected fields.
- Command-mode planning rejects operating-system command execution phrases.
- Native insertion revalidates the target before pasting: LocalFlow captures the
  foreground window (HWND, owning process id, and the control holding the keyboard focus)
  when dictation starts and, immediately before `Ctrl+V`, aborts if any of them changed. It
  **fails closed** — if the original or current target cannot be confirmed, it does not
  paste (`insertion/target.rs` `TargetWindow`/`target_matches`). Controls that are not
  windows of their own, such as the fields of a web page, cannot be told apart.
- The paste waits up to 1 s for `Ctrl`, `Alt`, `Shift` and `Win` to be released, so a held
  modifier cannot turn `Ctrl+V` into another shortcut, and does not paste if one stays down.
- A target running at a higher integrity level (an administrator window) is detected and
  not pasted into, with a message, instead of the paste being silently dropped.
- Insertion planning also rejects protected fields and changed targets in the shared TS
  `insertionPlan` layer (used by the mock/verification path).
- Duplicate insertion guard prevents repeating the same generated text for one session
  (shared TS layer; the native path now supersedes stale sessions via `SessionRegistry`).
- The web UI runs under a content security policy: scripts and styles from the app only, connections only to the app's IPC. It has no other network access; the native layer asks local Ollama which models are installed. The overlay demo and SDK example pages exist only in development builds.
- Conversation mode gives the language model no tools. Fixed phrases are matched in code before the model is asked, and the model's output is only ever spoken, so nothing it reads or writes can trigger an action.
- A window is read aloud only on the user's own request at that moment, only the focused window, and only after a capture policy has refused private browsing windows, password managers, credential prompts, LocalFlow's own windows and windows with a password field focused. Password and edit fields are never entered.
- Text read from a window goes only to the voice. It cannot reach the language model, the conversation memory, a log or an event: the type that carries it can only be created from page text, hides its content in debug output, and tests check that no page text appears in engine notices or in the next model request.
- The assistant's own voice must not be taken for the user's. While it speaks, the microphone passes through echo cancellation, speech has to pass a stricter and longer test than an ordinary start of turn, and an apparent interruption whose transcript is a stretch of the reply being spoken is discarded. While it is thinking, microphone audio is discarded.
- Tauri capabilities are per window. The settings window may call the app's declared commands; the overlay window may only listen for events. The opener and global-shortcut web permissions are not granted.
- Logs use redaction helpers for dictated content. The log file
  (`%LOCALAPPDATA%\app.localflow.desktop\logs\localflow.log`, two files of at most 1 MB) has
  user folders replaced by `<user folder>` in every line, and its filter is fixed, so an
  environment variable cannot turn on a dependency's request logging. The panic hook records
  only the thread and the source location, never the panic's message.
- The diagnostics export in the settings window leaves out dictated text and local model
  paths by default, and replaces user folders and microphone names; including transcript
  text is an explicit option for troubleshooting.
- The clipboard paste publishes the dictated text with delayed rendering and puts the earlier
  text back once the target has read it (never sooner than 400 ms), clears the dictated text
  when there was no text to restore, and leaves the clipboard alone if something else was
  copied in between. Content another program marked private (a password manager's
  `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, or a
  history/cloud format set to no) is never read and never restored: the clipboard is cleared
  instead. The dictated text, the restored text and "Copy last transcript" all carry the
  Windows formats that exclude them from clipboard history (Win+V) and cloud clipboard sync.
- A microphone that stops delivering audio no longer leaves a session listening: the end-of-speech, no-speech, and maximum-length timeouts run on silence too.
- The waveform overlay receives phase and audio-level metadata, not transcript text.
- **Where programs are started from.** Release builds are compiled without the `dev-runtime`
  feature. They start the whisper.cpp programs and the Fish engine only from the app's own
  folder (`localflow-runtime` next to `localflow.exe`) and from the per-user folder
  `%LOCALAPPDATA%\LocalFlow`, ignore `LOCALFLOW_WHISPER_CLI`, and carry no path of the
  computer they were built on. Both folders are writable only by the user (and
  administrators), so these programs run with the same trust as the user's own files.
  `LOCALFLOW_WHISPER_MODEL` is still honoured: it names a model file, like the path in
  Settings, not a program.
- "Missing" messages and the readiness checklist name files and scripts, never a folder,
  so they cannot leak the user's name into a screenshot or a bug report.
- **Downloads.** Every model and the whisper.cpp programs come from a pinned revision and are
  checked against a SHA-256 before they replace anything, over HTTPS only. The scripts are
  run by the user; LocalFlow never downloads anything itself.
- **Release builds.** Installers are built by the release workflow on GitHub Actions from a
  version tag, with no dependency caches, and published with `SHA256SUMS`. Third-party
  actions are pinned to commits. Every program in the installer is checked to load only
  Windows DLLs or DLLs shipped beside it.

## Known Limitations

- **The whisper server is unauthenticated.** When a Whisper model is selected, LocalFlow keeps
  `whisper-server.exe` running on `127.0.0.1` on an ephemeral port. Any program on the same
  computer, running as any user, can connect to that port while it runs and have its own
  audio transcribed (using CPU time). It cannot read LocalFlow's requests or results, which
  travel on LocalFlow's own connections. The default Parakeet model runs in-process and
  starts no server. Planned: replace the server with the in-process engine or protect it
  with a per-launch secret.
- **Windows Error Reporting.** If LocalFlow or one of its helper programs crashes, Windows
  Error Reporting may write a crash report, and depending on your Windows diagnostic data
  settings may send it to Microsoft. A crash report can contain process memory, and so
  recent dictated or spoken text. LocalFlow does not change these settings. To exclude it,
  add a `DWORD` value named `localflow.exe` set to `1` under
  `HKEY_CURRENT_USER\Software\Microsoft\Windows\Windows Error Reporting\ExcludedApplications`,
  or turn off optional diagnostic data in Windows Settings.
- **Unsigned installers.** The installers are not code-signed yet, so Windows SmartScreen
  warns before running them. Check the download against `SHA256SUMS` on the release page.
- **App-local Visual C++ runtime.** The installers ship the Visual C++ runtime DLLs next to
  the programs that need them, so Windows Update does not service those copies; they are
  refreshed with each LocalFlow release.

## Planned Controls

- Native UI Automation insertion.
- Native rich clipboard preservation, and stronger target verification (process start time
  to defeat PID reuse; UIA element identity; protected/read-only field detection).
- Paste-last-into-focus recovery via a dedicated hotkey (copy-last recovery already exists;
  the transcript is held in volatile memory only and never written to disk).
- Verify the identity of the process listening on the Ollama port, and close the short window in which another local process could take the whisper sidecar's reserved port.
- Check the integrity of bundled programs before starting them.
- Code-sign the installers and the executable.
- Retention enforcement jobs.
