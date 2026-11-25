# Network Policy

LocalFlow itself makes no connection beyond this computer. Speech recognition, the language
model, the voices, turn-taking and page reading all run locally. There is no telemetry, no
analytics, no update check and no silent fallback to a remote service.

Allowed:

- `localhost`, `127.0.0.1` and `[::1]`, for the local services listed below.
- Local sidecar process IPC.

Blocked:

- Cloud speech recognition endpoints.
- Cloud language model endpoints, including Ollama's cloud-hosted models.
- Analytics and telemetry endpoints, and update checks.
- Silent fallback to a remote model provider.

## The Settings Window

In the desktop app the web UI makes no network request: its content security policy
(`tauri.conf.json`) allows connections only to the app's IPC, and the Models screen asks the
native layer which Ollama models are installed (`list_ollama_models`).

Only when the web UI runs in a browser during development (`npm run dev`) does it call
Ollama itself, through the shared Ollama provider in `src/domain/ollama.ts`. Before every
request the shared `evaluateDictationNetworkUrl` helper allows only:

- plain `http:`,
- to `localhost`, `127.0.0.1` or `[::1]`,
- on port 11434,
- with no user name or password in the URL.

Everything else is refused before `fetch` is called. Every request is sent with
`redirect: "error"`, so a redirect fails instead of being followed. Future local providers
in the web UI must call the same helper.

## Native Enforcement

The native app talks to at most three local HTTP endpoints, all on `127.0.0.1`:

- The `whisper-server.exe` sidecar that LocalFlow starts itself on an ephemeral loopback
  port, when a Whisper model is selected. The default Parakeet model runs in-process and
  uses no socket at all.
- The local Ollama server on port 11434.
- The `s2.exe` read-aloud sidecar on an ephemeral loopback port, only while read-aloud is in
  use.

Every HTTP client is built with system and environment proxies disabled and redirects
disabled, so dictated audio and text cannot be routed off the machine by an `HTTP_PROXY` /
`ALL_PROXY` setting or by a redirect response.

Ollama can serve "cloud" models, which it forwards to Ollama's servers, and a model can be
copied or created under any name. So, before any request that carries text:

- a model name with a cloud tag (for example `gemma4:cloud` or `*-cloud`) is refused without
  a request;
- otherwise Ollama is asked about the model (`POST /api/show`), and a model whose description
  has a non-empty `remote_host` or `remote_model` is refused, whatever it is called;
- a reply that is not a model description, or an error, also refuses it: when in doubt,
  nothing is sent.

A clear answer is remembered for 60 s. The installed-model list (`list_ollama_models`) and
the readiness checklist leave out every model with a cloud tag or a remote host or model in
`/api/tags`.

## The One Exception: Coding Agents Started by Voice

Recorded by the project owner on 2026-11-23 (see `AGENTS.md` and [PRIVACY.md](PRIVACY.md)).
During a conversation, a voice command may start a coding agent the user subscribes to
(Claude Code today), and only after LocalFlow has read the command back and heard a spoken
"yes":

- "Ask Claude ..." passes only the spoken question, plus a fixed instruction to answer
  briefly, to Claude Code. Claude Code sends it to Anthropic under the user's own
  subscription and may read files in the named project folder to answer. No page text,
  transcript history or other context is added.
- "Open Claude Code in ..." opens a Windows Terminal tab running Claude Code in that folder
  and passes it nothing. What the user then types or dictates into Claude Code goes to
  Anthropic, as in any Claude Code session.
- LocalFlow itself still makes no network connection: the agent does. LocalFlow never passes
  options that skip the agent's permission prompts, and refuses any argument list that could.
- LocalFlow's own background runs of Claude Code (detection with `--version` and `--help`,
  and the question) are started with `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` and
  `DISABLE_AUTOUPDATER=1`, which tell Claude Code to skip its update checks, error reports
  and other traffic it does not need to answer. The visible tab is the user's own session and gets
  neither. A conversation runs detection only when a project is set up.

## Downloads and Installation

- Model downloads and engine builds are never performed by the app. The scripts
  (`Get-AsrModel.ps1`, `Get-PhononModel.ps1`, `Get-VadModel.ps1`, `Get-SmartTurnModel.ps1`,
  `Get-FishModel.ps1`, `Get-WhisperRuntime.ps1` and `Build-FishS2.ps1`) are explicit,
  user-run helpers. Each downloads a pinned revision over HTTPS from Hugging Face or GitHub
  and checks its SHA-256.
- The installers embed Microsoft's WebView2 bootstrapper. If the WebView2 Runtime is missing
  (Windows 11 includes it), the bootstrapper downloads it from Microsoft during
  installation. That is the installer, not LocalFlow.
- LocalFlow never checks for updates. New versions come from
  [GitHub Releases](https://github.com/liulawrence2002/localflow/releases).
