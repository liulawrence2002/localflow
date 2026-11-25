# Development

How to build, test and extend LocalFlow. The design is in [ARCHITECTURE.md](ARCHITECTURE.md),
what each change built is in [development/PLAN.md](development/PLAN.md), and the rules for
contributors (human or agent) are in [AGENTS.md](../AGENTS.md).

## Setup

### Tools

```powershell
.\scripts\Install-Prereqs.ps1            # check only
.\scripts\Install-Prereqs.ps1 -Install   # install what is missing, through winget
npm install
```

The script checks Node.js, npm, Git, Rust (`cargo`, `rustc`, installed through rustup) and
the Visual Studio C++ Build Tools; with `-Install` it installs the 2022 Build Tools with the
VCTools workload and a Windows 11 SDK. WebView2 is also needed; Windows 11 includes it.
Node.js 22 or newer and npm 10 or newer are expected (`.nvmrc` names 22), and Rust 1.96 or
newer (`rust-version` in `src-tauri/Cargo.toml`); CI and releases build with 1.96. Restart the
terminal if a new tool is still missing from `PATH`.

Two more things must be on `PATH` in the shell that builds:

- **Cargo.** rustup puts it in `%USERPROFILE%\.cargo\bin`. The repository's scripts add that
  folder themselves; in your own shell:

  ```powershell
  $env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
  ```

- **CMake.** The `transcribe-cpp` crate compiles transcribe.cpp from C++ source, so the first
  build needs `cmake`. The Build Tools ship one:

  ```powershell
  $vs = & "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -property installationPath
  $env:PATH = "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;$env:PATH"
  ```

### GPU builds: Vulkan SDK

Only the `gpu-vulkan` feature needs it. Install the Vulkan SDK, then make sure `VULKAN_SDK`
is set and `%VULKAN_SDK%\Bin` (for `glslc`) is on `PATH`. `src-tauri/build.rs` adds
`%VULKAN_SDK%\Lib` to the linker path, so `LIB` does not need setting. The first GPU build
takes about 1 GB of extra disk for compiled shaders. The resulting executable imports
`vulkan-1.dll` and will not start on a computer without a Vulkan loader.

### Runtime assets

Everything a source build loads at run time lives in `.localflow-runtime/` at the repository
root. Git ignores it; never commit models or binaries (AGENTS.md).

```text
.localflow-runtime\
  whisper\Release\whisper-cli.exe, whisper-server.exe,      Get-WhisperRuntime.ps1; bundled, every build needs them
    whisper.dll, ggml.dll, ggml-base.dll, ggml-cpu-*.dll
  models\ggml-tiny.en-q5_1.bin                       Get-AsrModel.ps1 -Model tiny.en; bundled, every build needs it
  models\parakeet-tdt-0.6b-v3-Q5_K_M.gguf            Get-AsrModel.ps1
  models\phonon\                                     Get-PhononModel.ps1
  models\silero-vad-v6.2-F32.gguf                    Get-VadModel.ps1
  models\smart-turn-v3.2-gpu.onnx                    Get-SmartTurnModel.ps1
  models\fish\, fish\                                Get-FishModel.ps1, Build-FishS2.ps1 (optional)
  bench-corpus\manifest.tsv, wav\                    benchmarks and real-model tests
  bench-results\                                     Bench-AsrSpeed.ps1 output
```

- Every `Get-*.ps1` script downloads a pinned revision, checks each file's SHA-256, and
  takes `-Destination`. Run from a checkout they default to `.localflow-runtime`; run from
  anywhere else (an installed copy) they default to `%LOCALAPPDATA%\LocalFlow`. The model
  scripts are listed in [MODEL_SETUP.md](MODEL_SETUP.md); the bench corpus is described in
  [benchmarks/ASR_BENCHMARK.md](benchmarks/ASR_BENCHMARK.md).
- `Get-WhisperRuntime.ps1` extracts only the files LocalFlow runs from the pinned whisper.cpp
  v1.9.5 Windows x64 release. `tauri.conf.json` lists exactly those files as bundle
  resources (not `*.dll`, which would pull in SDL2 and the release's own `parakeet.dll`).
  The programs need the Visual C++ runtime, which the Build Tools install; the release
  workflow ships it beside them.
- The Tauri build script copies the bundle resources even for `cargo test`, so without
  these files the build fails. CI stands in an empty file for each one
  (`.github/workflows/ci.yml`); you can do the same if you only need Parakeet, but the
  Whisper fallback will then not work. The build script also embeds the built web UI, so
  run `npm run build` once before the first `cargo test`.
- The app looks for programs and models in `localflow-runtime` in its resource folder
  (next to the executable), then in `%LOCALAPPDATA%\LocalFlow`, then in the checkout's
  `.localflow-runtime` (`runtime_paths.rs`). The last one exists only with the `dev-runtime`
  Cargo feature, which is on by default, so a local build reads the checkout it was built
  from. Release builds turn it off (`--no-default-features`); see [Releases](#releases).

## Building

| Command                                  | What it does                                                                                          |
| ---------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| `npm run dev`                            | Vite only, for the web UI in a browser (port 1420). Commands fall back to browser stubs.              |
| `npm run tauri:dev`                      | Vite plus a debug build of the app, with hot reload. `.\scripts\Start-Dev.ps1` does the same.         |
| `npm run tauri:dev:gpu`                  | The same with `--features gpu-vulkan`.                                                                |
| `npm run tauri:build`                    | Release app plus the MSI and NSIS installers. `.\scripts\Build-Installer.ps1` wraps it.               |
| `npm run tauri:build:gpu`                | The same with `--features gpu-vulkan`.                                                                |
| `npm run tauri:build -- --no-bundle`     | Only `src-tauri\target\release\localflow.exe`, no installers. Quicker when you only want to run it.   |
| `npm run tauri:build:gpu -- --no-bundle` | The GPU executable only.                                                                              |
| `npm start`                              | `scripts\Start-LocalFlow.ps1`: builds the release app if missing, starts Ollama, starts the tray app. |

After a new release build, `.\scripts\Start-LocalFlow.ps1 -Restart` replaces the running copy.
CPU and GPU builds write the same `target\release\localflow.exe`.

`tauri:dev` builds the debug profile, whose cache grows very large (it reached 13 GB). To try
a change in the real app, a `--no-bundle` release build is usually the better choice.

## Tests

The testing reference, the manual checklists and the latest verified results are in
[TESTING.md](TESTING.md). In short:

### Frontend

```powershell
npm run test         # vitest, tests in tests/
npm run lint
npm run format       # prettier --check
npm run typecheck    # tsc -b: the SDK project, then the app and tests
npm run build        # tsc -b, then vite build
npm run check        # format, lint, test and build
```

The app project (`tsconfig.json`) reads the SDK package (`packages/localflow-sdk`) through
the declarations its own project (`tsconfig.sdk.json`) emits into `node_modules/.tmp`. On a
fresh clone those do not exist yet, so run `npm run typecheck` or `npm run build` once before
`npx tsc --noEmit`, which does not build referenced projects and otherwise reports the SDK's
files as not built.

The settings window lives in `src/app` (the shell and its hooks), `src/screens` (one
component per screen), `src/components/ui`, `src/domain` and `src/styles`; there is no
`src/App.tsx` any more. See [ARCHITECTURE.md](ARCHITECTURE.md#frontend-layer).

### Rust

From `src-tauri`, in PowerShell with Cargo and CMake on `PATH`:

```powershell
cargo fmt --check
cargo test --release --lib
```

Always pass `--release`. The in-process speech engine compiles C++, and the debug profile's
cache reached 13 GB; tests in release reuse the release cache. CI runs `cargo fmt --check`,
`cargo clippy --release --lib --examples --locked -- -D warnings`, the tests with `--locked`
and `cargo build --release --examples --locked`, and on the web side
`npm audit --omit=dev --audit-level=high` before the frontend checks.
`.\scripts\Run-Checks.ps1` runs the frontend checks, then `cargo fmt --check`,
`cargo test --release --lib` and `cargo build --release --examples`; it does not run clippy.

### Ignored tests

Tests that need models, a running Ollama, hardware, the real clipboard, a stand-in agent or
another program are marked `#[ignore = "..."]` with the reason. Run them by name with `-- --ignored`:

| Filter                   | Needs                                                                               | Command                                                                                                        |
| ------------------------ | ----------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- |
| `real_parakeet`          | Parakeet and the bench corpus                                                       | `cargo test --release --lib real_parakeet -- --ignored`                                                        |
| `real_server`            | `whisper-server.exe`, a Whisper model, the bench corpus                             | `cargo test --release --lib real_server -- --ignored`                                                          |
| `real_smart_turn`        | the Smart Turn model                                                                | `cargo test --release --lib real_smart_turn -- --ignored`                                                      |
| `real_silero`            | the Silero model                                                                    | `cargo test --release --lib real_silero -- --ignored`                                                          |
| `real_phonon`            | the Phonon voice                                                                    | `cargo test --release --lib real_phonon -- --ignored`                                                          |
| `real_ollama`            | Ollama running, with `llama3.2:3b` pulled; read-only (`/api/show`)                  | `cargo test --release --lib real_ollama -- --ignored`                                                          |
| `real_parakeet_gpu`      | a GPU build and a discrete GPU                                                      | `cargo test --release --lib --features gpu-vulkan real_parakeet_gpu -- --ignored --nocapture --test-threads=1` |
| `_speed`                 | nothing; timing tests for the canceller, resampler and spectrum                     | `cargo test --release --lib _speed -- --ignored --nocapture`                                                   |
| `coding_agents`          | `cargo build --release --example fake_agent` first                                  | `cargo test --release --lib coding_agents -- --ignored`                                                        |
| `live_windows_terminal`  | `cargo build --release --example wt_probe`; `LOCALFLOW_LIVE_WT=1`; opens a real tab | `cargo test --release --lib live_windows_terminal -- --ignored --nocapture`                                    |
| `live_delayed_rendering` | `LOCALFLOW_LIVE_CLIPBOARD=1`; replaces the real clipboard for a moment              | `LOCALFLOW_LIVE_CLIPBOARD=1 cargo test --release --lib live_delayed_rendering -- --ignored`                    |

- The `coding_agents` run includes `detects_what_is_installed_here`, which prints what this
  computer has and launches nothing. The other tests there run `examples/fake_agent.rs`, a
  stand-in for `claude.exe -p` that never touches the network.
- `live_windows_terminal_passes_the_folder_and_arguments_through` opens a real Windows
  Terminal tab running `examples/wt_probe.rs` (a stand-in for `claude.exe` that writes down
  the folder and arguments it received, then exits) in a temporary folder with a space in its
  name. It does nothing unless `LOCALFLOW_LIVE_WT=1` is set, so a plain `-- --ignored` run
  never opens a window.
- `live_delayed_rendering_round_trip` (`platform/clipboard.rs`) runs the paste's clipboard
  path on the real Windows clipboard: it publishes text with delayed rendering, reads it from
  another thread, checks that the read was seen and the earlier text put back, then checks
  that text copied by someone else in between is left alone. It does nothing unless
  `LOCALFLOW_LIVE_CLIPBOARD=1` is set, skips itself unless the clipboard holds plain text
  that no program marked private, and puts that text back at the end. In PowerShell, set the
  variable with `$env:LOCALFLOW_LIVE_CLIPBOARD = "1"` first; the `VAR=1 command` form above
  is for Git Bash.
- Before sending LocalFlow hotkeys from a script in a live test, confirm that the foreground
  window is your test window: the reading hotkey reads whatever has the focus aloud, and
  dictation pastes into it.

## Benchmarks

Run benchmarks on a quiet machine, plugged in, with nothing compiling and no other benchmark
running: timings on the development laptop move with background load, and only numbers from
one session can be compared. Harnesses print timings and counts only, never transcript text.

| What                             | How                                                                                                                                        | Results and method                                                                                 |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| Speech-recognition engines       | `cargo run --release --example asr_bench -- --config inprocess:parakeet-tdt-0.6b-v3-Q5_K_M.gguf:1:6 --json ..\docs\benchmarks\my-run.json` | [ASR_BENCHMARK.md](benchmarks/ASR_BENCHMARK.md), [TESTING.md](TESTING.md#speech-to-text-benchmark) |
| CPU threads and the GPU          | `.\scripts\Bench-AsrSpeed.ps1` (`-Part Threads`, `-Part Gpu` or `All`; `-SweepThreads "4,6,8,12"`, quoted; `-VulkanSdk <path>`)            | [ASR_SPEED_BENCHMARK.md](benchmarks/ASR_SPEED_BENCHMARK.md)                                        |
| Conversation end to end          | `cargo run --release --example conversation_bench -- ..\.localflow-runtime --passes 2`                                                     | [CONVERSATION_BENCHMARK.md](benchmarks/CONVERSATION_BENCHMARK.md)                                  |
| The voice alone, the model alone | `cargo run --release --example tts_bench -- ..\.localflow-runtime\models\phonon --passes 3`; `cargo run --release --example chat_bench`    | [CONVERSATION_BENCHMARK.md](benchmarks/CONVERSATION_BENCHMARK.md)                                  |
| Text judge against Smart Turn    | `turn_judge_bench run` and `report`, on clips from `.\scripts\Get-SmartTurnTestClips.ps1`                                                  | [TURN_JUDGE_BENCHMARK.md](benchmarks/TURN_JUDGE_BENCHMARK.md#how-to-reproduce)                     |
| What a window would be read as   | `cargo run --release --example page_probe -- "<window title>"`                                                                             | [ARCHITECTURE.md](ARCHITECTURE.md#reading-a-window-aloud)                                          |
| Dictation clean-up in Ollama     | `.\scripts\Measure-OllamaCleanup.ps1 -Model llama3.2:3b -Rounds 3`                                                                         | [TESTING.md](TESTING.md#speech-to-text-benchmark)                                                  |

Notes:

- Run the Rust harnesses from `src-tauri`. `asr_bench` options (engines, `--sweep-threads`,
  `--list-devices`, the GPU device field) are documented at the top of
  `examples/asr_bench.rs`.
- The GPU part of `Bench-AsrSpeed.ps1` builds with `--features gpu-vulkan`; `-VulkanSdk`
  defaults to `$env:VULKAN_SDK`. Results go to `.localflow-runtime\bench-results`; copy a
  run you want to keep into `docs\benchmarks`.
- `conversation_bench` needs Ollama running with the model, plus the voice and Silero. To
  measure the GPU path, build it with `--features gpu-vulkan`.
- The Smart Turn test clips are third-party data (CC-BY-4.0, 344 MB). Keep them out of the
  repository and delete the folder afterwards. While `turn_judge_bench.exe` runs, copy it out
  of `target` first: a running executable there blocks other builds on Windows.

## Releases

Releases are built by `.github/workflows/release.yml` when a `v*` tag is pushed. It:

1. checks that the tag matches the version in `package.json` (which `tauri.conf.json` reads)
   and `src-tauri/Cargo.toml`;
2. installs Rust 1.96 (or the toolchain `rust-toolchain.toml` names) and runs `npm ci`, with no dependency
   caches;
3. refuses to continue if `THIRD_PARTY_NOTICES.md` is out of date;
4. fetches the whisper.cpp programs and tiny.en with the same scripts as above;
5. copies the Visual C++ runtime (`msvcp140.dll`, `vcruntime140.dll`, `vcruntime140_1.dll`,
   and `vcomp140.dll` for the ggml DLLs) from the runner's Visual Studio into
   `.localflow-runtime`, and `src-tauri/tauri.release.conf.json` maps them next to
   `localflow.exe` and next to the whisper.cpp programs;
6. builds with `npx tauri build --config src-tauri/tauri.release.conf.json -- --no-default-features --locked`:
   the CPU build without `dev-runtime`, with the workspace and Cargo's home remapped in debug
   paths and the executable stripped (`[profile.release.package.localflow]` in
   `Cargo.toml`);
7. checks with `dumpbin` that every shipped program loads only Windows DLLs or DLLs beside
   it, so the installers work on a computer without the Visual C++ redistributable;
8. attaches the MSI, the NSIS installer and `SHA256SUMS` to a **draft** GitHub release, with
   the version's section of `CHANGELOG.md` as the notes. A maintainer publishes it.

To release: move the `Unreleased` entries in `CHANGELOG.md` under a new version heading,
set the same version in `package.json` and `src-tauri/Cargo.toml`, merge, then tag the
merge commit (`git tag v0.1.0`, `git push origin v0.1.0`).

To try a release-like build locally, without the checkout's runtime folder:

```powershell
npm run tauri:build -- --no-bundle -- --no-default-features
```

Such a build looks for models only next to the executable and in
`%LOCALAPPDATA%\LocalFlow`, and ignores `LOCALFLOW_WHISPER_CLI`.

### Third-party notices

`THIRD_PARTY_NOTICES.md` is installed next to the app. Its crate and npm lists are generated
from the lock files; the sections above them (ported code, the bundled whisper.cpp programs
and model, model credits) are written by hand. After changing dependencies:

```powershell
.\scripts\Generate-ThirdPartyNotices.ps1        # -Check only reports whether it is current
npx prettier --write THIRD_PARTY_NOTICES.md
```

### Desktop shortcut smoke check

`.\scripts\Test-DesktopBoot.ps1` checks a checkout-based desktop setup: that the desktop
shortcut runs `Start-LocalFlow.vbs` from the repository, that the launchers start the
release executable, that the window settings and hotkey registration are as expected, and
the health file the app writes (`-RequireRunning`, `-ExpectRecentHotkeySeconds`). It reads
source files to do so, so update it when those change. It does not apply to an installed
copy.

## Rules for contributors

The full list is in [AGENTS.md](../AGENTS.md). In brief:

- Keep domain logic testable outside Tauri commands, and platform code behind narrow modules
  (`platform`, `hotkeys`, `context`, `insertion`).
- Use explicit state transitions for workflow changes, and typed provider traits with mock
  providers rather than direct dependencies in workflow code.
- No cloud speech recognition, language model, telemetry or analytics, and no silent network
  fallback. Ordinary use must work offline once the models are installed. The coding-agent
  commands are the one recorded exception, and only after a spoken "yes".
- Keep dictated, spoken and page content out of logs and events. Never collect password or
  protected-field content.
- Keep models and sidecar binaries in `.localflow-runtime/` or packaged resources, never in
  Git.
- New workflow logic gets focused unit tests. Update `docs/development/PLAN.md` with what was
  done, its known limitations and its risks, and make every user-facing claim match what was
  actually run.

## Adding a coding agent

Codex and Cursor Agent already exist as `AgentKind` values with spoken names in `grammar.rs`,
but behind `NotSetUp`. To make one work:

1. **Implement `CodingAgent`** (`coding_agents/agent.rs`): `kind`, `detect` (find it and
   report an `AgentStatus`; it may run only `--version` and `--help`), `open_in_folder`
   (open it visibly, with no task) and `ask` (a read-only question, honouring the
   `CancelFlag`). Take a `Locator` and a `Launcher` as `ClaudeCode` does, so tests can see
   exactly what would start.
2. **Find the real executable** in `detect.rs`, never a `.cmd` or `.ps1` shim, which would
   put `cmd.exe` or PowerShell and their quoting rules in between.
3. **Build arguments in `args.rs`.** One element per argument, no shell, no user text on a
   Windows Terminal command line, paths through `check_terminal_path`, and every list through
   `check_no_permission_skipping`. If the agent has its own way of skipping permission
   prompts, add its spelling to `FORBIDDEN_TEXT` and test that the guard refuses it.
4. **Run background work through `run::run_captured`**, which puts the process tree in a
   kill-on-close job (`platform/process.rs`), with a time limit and the question on standard
   input. Visible launches go through `Launcher`, which checks the arguments again.
5. **Register it** in `CodingAgents::system()` in place of its `NotSetUp`.
6. **Report what it can do.** `AgentAvailability` has fields only for Claude Code and the
   Cursor editor: add the new agent's, fill them in `CodingAgents::detect_all`, and change
   `grammar::availability`, which today answers `NotSetUpYet` for Codex and Cursor Agent.
7. **Speech.** Check every sentence in `speech.rs`: read-backs, `outcome`, and the
   Claude-specific lines (the sign-in advice, and `STILL_WORKING` in `conversation/agents.rs`,
   which says "Claude is still working"). Run the test `no_part_of_a_read_back_is_itself_a_yes`
   over any new read-back.
8. **The conversation.** Decide whether opening it should end the conversation for dictation
   (`ExecutionOutcome::ends_conversation`, Claude Code only today).
9. **Settings and privacy.** Update `describeAgent` in `src/domain/codingAgents.ts` if the
   screen needs to say something new, and update `PRIVACY.md` with what the agent sends and
   to whom before it ships.
10. **Tests.** Add phrasings and near-misses to the corpus tests in `grammar.rs`, argument
    tests in `args.rs` (including the never-skips-permissions assertion), and a fake for the
    background path like `examples/fake_agent.rs`.

## Adding a conversation phrase

Control phrases live in `conversation/router.rs` as lists per intent (`END_PHRASES`,
`QUIET_PHRASES`, `REPEAT_PHRASES` and the `READ_*_PHRASES`).

1. **Write it normalised:** lower case, apostrophes removed ("thats all"), any other
   punctuation as a single space. Polite words (`POLITE_WORDS`) are stripped from both ends
   before matching, so do not add "please" variants.
2. **Keep it short.** Only turns of up to `MAX_COMMAND_WORDS` (8) words are matched; a longer
   sentence that contains a phrase is a question for the model.
3. **Check what is matched first.** Every turn goes to the coding-agent dialog before the
   router. While a read-back waits, any turn is a yes or a decline (`dialog.rs`); otherwise a
   turn that starts with an action word and an agent's name is taken as a command
   (`grammar.rs`). A new phrase must not start like one, and must not be a yes in
   `dialog::YES`.
4. **Decide what it does over a reply.** `barge_in::is_command` makes end, quiet, repeat and
   the reading moves real interruptions; `BACKCHANNELS` and `STOP_WORDS` in `barge_in.rs`
   decide one-word and acknowledgement cases. A phrase meaning "carry on" belongs in
   `BACKCHANNELS`.
5. **A new intent** needs a `TurnIntent` variant, a branch in `route_turn`, handling in
   `answer` in `conversation/engine.rs`, and a decision in `barge_in::is_command`.
6. **Test it.** Add the phrase, with its punctuation variants, to the router tests, and add
   sentences that merely contain it to
   `a_sentence_that_only_contains_a_control_word_is_a_question` or
   `talking_about_reading_is_still_a_question`. For a new intent, add a scripted test in
   `engine.rs`.
7. **Document it** in [USER_GUIDE.md](USER_GUIDE.md#spoken-phrases).

## Gotchas

- **Line endings.** `.gitattributes` checks text files out with LF on every platform, as
  prettier and rustfmt expect, whatever `core.autocrlf` says. A checkout made before that
  file existed may still have CRLF files: `git add --renormalize .` fixes it. Run
  `npx prettier --write` on any Markdown you change, including under `docs/`.
- **Cargo from PowerShell.** Cargo is in `%USERPROFILE%\.cargo\bin`, which a Git Bash or other
  non-login shell may not have on `PATH`. `tauri dev` and `tauri build` call cargo, so they
  fail the same way. Run Rust commands from PowerShell with that folder added, or through the
  repository's scripts.
- **CMake** must be on `PATH` for the first build (see [Setup](#tools)).
- **Release tests only.** `cargo test` without `--release` builds the debug cache, which
  reached 13 GB.
- **A running app locks its executable.** Quit LocalFlow from the tray before rebuilding
  `target\release\localflow.exe`. A running example executable in `target` blocks builds
  the same way.
- **One copy at a time.** The single-instance lock is named after the app identifier, so
  while any LocalFlow runs, a second start (a `tauri dev` build included) only shows the
  running copy's settings window and exits. Quit the tray copy first. Closing the settings
  window does not quit; it hides.
- **Logs.** A release build has no console. Its log is
  `%LOCALAPPDATA%\app.localflow.desktop\logs\localflow.log` (and `.log.1`); a debug build
  prints the same lines to its console too. The filter is fixed in `init_logging`
  (`localflow=info,tauri=warn`); `RUST_LOG` does not change it.
- **New Tauri commands** must be added to `APP_COMMANDS` in `src-tauri/build.rs`,
  `capabilities/main.json` and `generate_handler!` in `lib.rs`. A missing entry fails only at
  run time, in the web UI. A debug-only command goes in `DEBUG_ONLY_COMMANDS` instead, with
  `#[cfg(debug_assertions)]` in `lib.rs` and its permission in `capabilities/debug/`, which
  only a debug build reads (the mock dictation commands are the example).
- **New settings fields** need a default: the settings structs are `#[serde(default)]` so
  older stored settings still load.
- **One Ollama context size.** Every caller uses `OLLAMA_NUM_CTX` (4096). Asking for another
  size makes Ollama reload the model, about 10 s for `llama3.2:3b`.
- **Smart Turn file.** Use `smart-turn-v3.2-gpu.onnx`; the 8-bit `-cpu` file loads in `tract`
  but gives wrong answers.
- **Voice threads.** Phonon is capped at four threads on purpose; one per logical core made it
  four times slower.
- **Disk space.** LocalFlow's models need about 0.7 GB, plus the Ollama model. A GPU build
  takes about 1 GB more, and the optional Fish voice 3.6 GB plus its engine.
- **Live hotkey tests** act on whatever window has the focus. Check the foreground window
  before sending any LocalFlow hotkey from a script.
