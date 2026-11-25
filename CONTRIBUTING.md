# Contributing to LocalFlow

Thank you for helping. LocalFlow is a local-first Windows voice app: dictation, a spoken
conversation with a local language model, reading the focused window aloud, and voice
commands for coding agents. Everything except the confirmed coding-agent commands runs on
the user's own computer, and contributions have to keep it that way.

## Before you start

- For anything larger than a small fix, open an issue first so the approach can be agreed
  before you spend time on it.
- Never put transcripts, dictated or page text, or paths containing your user name in an
  issue, a pull request, a test or a log.
- Security problems go through [private vulnerability reporting](SECURITY.md#reporting-a-vulnerability),
  not public issues.
- Everyone taking part follows the [code of conduct](CODE_OF_CONDUCT.md).

## The rules

[AGENTS.md](AGENTS.md) is binding for every contributor, human or coding agent. In short:

- No cloud speech recognition, language model, telemetry or analytics, and no silent network
  fallback. Ordinary dictation must work offline once the models are installed. The one
  recorded exception is a coding agent started by voice after a spoken read-back and "yes"
  ([PRIVACY.md](docs/PRIVACY.md)).
- Keep dictated, spoken and page content out of logs and events. Never collect password or
  protected-field content.
- Keep domain logic testable outside Tauri commands, platform code behind the narrow modules,
  workflow changes as explicit state transitions, and providers behind typed traits.
- Never commit models or sidecar binaries. They live in `.localflow-runtime/`, which Git
  ignores, or come from the scripts.

## Setting up and checking your change

[docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) covers the tools, the runtime files, CPU and GPU
builds, the tests and the benchmarks. Before opening a pull request, run:

```powershell
npm run check                  # prettier, eslint, vitest, tsc and vite build
cd src-tauri
cargo fmt --check
cargo test --release --lib     # always --release: the debug cache grows very large
```

Run `npx prettier --write` on any Markdown you change, including files under `docs/`.

## What a pull request needs

The [pull request template](.github/pull_request_template.md) mirrors the definition of done
in AGENTS.md:

- It builds, or the blocker is written down with the exact missing prerequisite.
- Formatting, linting, type checks and tests pass.
- New workflow logic has focused unit tests.
- [docs/development/PLAN.md](docs/development/PLAN.md) says what was done, its known
  limitations and the next risks.
- User-facing claims match what was actually run. Say what you tested, on what, and what you
  did not.

## Dependencies

- Prefer crates and packages under MIT, Apache-2.0, BSD or similarly permissive licences.
  Ask in the issue before adding anything copyleft or with usage restrictions.
- A new dependency must not add network access, telemetry or a download at run time.
- After changing `Cargo.lock` or `package-lock.json`, regenerate the notices and format them:

  ```powershell
  .\scripts\Generate-ThirdPartyNotices.ps1
  npx prettier --write THIRD_PARTY_NOTICES.md
  ```

- New model downloads go in a `scripts\Get-*.ps1` script that pins a revision and checks each
  file's SHA-256, with a credit in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Licence of contributions

LocalFlow is licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at
your option. Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in LocalFlow by you, as defined in the Apache-2.0 licence, shall be dual
licensed as above, without any additional terms or conditions.
