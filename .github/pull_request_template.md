## What this changes

<!-- What and why, in a few sentences. Link the issue if there is one. -->

## How it was checked

<!-- What you actually ran, and on what. Only claim what was run. -->

## Definition of done (AGENTS.md)

- [ ] It builds in my environment, or the blocker is written down with the exact missing
      prerequisite.
- [ ] `npm run check` passes (format, lint, tests, build).
- [ ] In `src-tauri`: `cargo fmt --check` and `cargo test --release --lib` pass.
- [ ] New workflow logic has focused unit tests.
- [ ] `docs/development/PLAN.md` says what was done, its known limitations and next risks.
- [ ] User-facing claims (README, docs, messages in the app) match what was actually run.

## Privacy

- [ ] No cloud speech recognition, language model, telemetry, analytics or silent network
      fallback was added.
- [ ] Dictated, spoken and page text stays out of logs and events, and no password or
      protected-field content is collected.
- [ ] No models, sidecar binaries or personal paths are committed; nothing in this pull
      request contains a transcript.
- [ ] New dependencies: their licences are compatible, and `THIRD_PARTY_NOTICES.md` was
      regenerated with `scripts/Generate-ThirdPartyNotices.ps1`.
