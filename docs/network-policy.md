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
