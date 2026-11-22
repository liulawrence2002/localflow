# Local Dictation Network Policy

Ordinary dictation must not contact remote services.

Allowed during ordinary dictation:

- `localhost`
- `127.0.0.1`
- `[::1]`
- Local sidecar process IPC

Blocked during ordinary dictation:

- Cloud ASR endpoints.
- Cloud LLM endpoints.
- Analytics and telemetry endpoints.
- Silent fallback to a remote model provider.

The shared `evaluateDictationNetworkUrl` helper enforces the URL portion of this policy for HTTP-based local model providers. The shared Ollama provider calls it before model discovery and cleanup requests. Native integrations and future local server providers must call the same policy before any production dictation path can contact a model server.

## Native Enforcement

The native app talks to at most three local HTTP endpoints, all on `127.0.0.1`:

- The `whisper-server.exe` sidecar that LocalFlow starts itself on an ephemeral loopback
  port, when a Whisper model is selected. The default Parakeet model runs in-process and
  uses no socket at all.
- The local Ollama server on port 11434.
- The `s2.exe` read-aloud sidecar on an ephemeral loopback port, only while read-aloud is in
  use.

Both HTTP clients are built with system and environment proxies disabled and redirects
disabled, so dictated audio and text cannot be routed off the machine by an `HTTP_PROXY` /
`ALL_PROXY` setting or by a redirect response. Ollama model names with a cloud tag (for
example `gemma4:cloud` or `*-cloud`) are refused before any request is made, because those
models run on Ollama's servers.

Model downloads and engine builds are never performed by the app. `scripts/Get-AsrModel.ps1`,
`scripts/Get-FishModel.ps1`, and `scripts/Build-FishS2.ps1` are explicit, user-run helpers.
