# LocalFlow Agent Notes

## Commands

- Install JavaScript dependencies: `npm install`
- Start frontend dev server: `npm run dev`
- Start Tauri desktop dev mode: `npm run tauri:dev`
- Build frontend: `npm run build`
- Run tests: `npm run test`
- Run lint: `npm run lint`
- Run format check: `npm run format`
- Run all available checks: `npm run check`
- Build installer: `npm run tauri:build`

Rust, Cargo, WebView2, and MSVC Build Tools are required before native Tauri commands can run. On Windows, `.\scripts\Install-Prereqs.ps1 -Install` checks and installs the common prerequisites.

## Coding Conventions

- Keep domain logic testable outside Tauri commands.
- Keep platform-specific code behind narrow modules in `src-tauri/src/platform`, `hotkeys`, `context`, and `insertion`.
- Use explicit state transitions for dictation workflow changes.
- Prefer typed provider traits and mock providers over direct dependencies in workflow code.
- Keep dictated content out of logs unless a user explicitly exports it.
- Keep native runtime assets in `.localflow-runtime/` or packaged Tauri resources; do not commit downloaded models or sidecar binaries.

## Privacy Requirements

- No cloud ASR, LLM, telemetry, analytics, or silent network fallback.
- One owner-approved exception (2026-11-23): a voice command may start a coding agent the user subscribes to (Claude Code now), only after a spoken read-back and "yes". Only the spoken question goes to it, never page text or other context; LocalFlow itself makes no network connection, and never passes flags that skip the agent's permission prompts. See `docs/PRIVACY.md`.
- Ordinary dictation must work offline once local models are installed.
- Logs redact transcript content by default.
- Do not collect protected-field or password context.
- Store only the retention level selected by the user.
- Do not silently fall back from local Whisper/Ollama to a remote service.

## Definition of Done

- Code builds in the available environment, or blockers are documented with exact missing prerequisites.
- Formatting, linting, type checks, and tests pass where the local toolchain permits.
- New workflow logic has focused unit tests.
- `docs/development/PLAN.md` reflects completed work, known limitations, and next risks.
- User-facing claims match what was actually run.
