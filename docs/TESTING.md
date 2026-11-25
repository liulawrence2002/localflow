# Testing

## Automated Tests

Current tests cover:

- State-machine transitions.
- Overlapping-session rejection.
- Stale-session rejection.
- Transcript stable-prefix commitment.
- Duplicate-free final transcript commitment.
- Rolling ASR overlap-window planning.
- `whisper.cpp` sidecar command planning, config validation, vocabulary prompt building, and JSON transcript parsing.
- Bounded audio ring buffers.
- RMS-based VAD and end-of-speech detection.
- Mono downmixing and sample-rate normalization helpers.
- Native spectrum analysis for the overlay: a tone peaks in its own band, two tones resolve, silence is exactly zero, louder never reads lower, white noise fills the range, the same tone lands in the same band at every sample rate, pitch follows a harmonic voice without octave errors, brightness follows the spectral centroid, steady background noise sinks while new sound still shows, bad input (NaN, infinity, clipping, zero rate) stays finite and in range, and the analysis is deterministic.
- Overlay spectrum math: band resampling (including bytes from the native layer), the synthetic spectrum fallback, floor and silence gating, frame-rate-independent attack and release, spline interpolation without overshoot, mirrored layout, edge envelope, the soft ceiling that keeps peaks inside the pill, and the colour ramp.
- Overlay feature smoothing and history: frame-rate-independent attack/release, invalid-value (NaN/Infinity/null) clamping, missed-event gap bounding, and the bounded spectrum history ring with interpolation across both time and columns.
- Overlay phase-to-visual mapping, including processing/refining sweeps over the frozen spectrum, success/cancel collapse, restrained state tints, and reduced-motion behavior.
- Overlay renderer: single animation loop under rapid state changes, frame cancellation on stop/dispose, pause while the document is hidden, listener removal on dispose, a clean start for each session (no stale spectrum), following real bands, ignoring the microphone while processing, finite in-bounds coordinates at full scale, and reduced work at low quality and with reduced motion.
- Boundary-aware replacements.
- Snippet expansion.
- Spoken punctuation.
- Explicit self-correction examples.
- Context snapshot privacy gates, context length limits, protected-field blocking, application category classification, and cleanup input mapping.
- LLM JSON validation.
- LLM JSON repair retry and deterministic fallback.
- Timeout handling.
- Performance metric recording, native latency diagnostics, missing-measurement warnings, and latency/memory formatting.
- Cursor-aware insertion spacing.
- Insertion target validation.
- Clipboard fallback delayed-restoration planning.
- Duplicate insertion rejection.
- Command-mode selected-text requirement, OS-command rejection, preview decisions, and undo text.
- Undo AI cleanup restore behavior.
- Privacy and retention rules.
- Redacted diagnostics export with transcript text and local model paths excluded by default.
- Localhost-only dictation network policy.
- Ollama local model discovery, remote URL blocking, unavailable/missing-model errors, non-streaming generate requests, and cleanup pipeline integration.
- Settings mutations for dictionary entries, replacements, snippets, and style profiles.
- Saved-settings normalization for older local fallback data.

- Speech-to-text provider layer: fallback chain ordering and failure reporting, per-utterance deadlines, whisper JSON parsing, multipart request building, missing-binary handling, start-failure backoff, temp-file cleanup after a failed run, and path-free error messages.
- Sidecar process helpers: kill-on-close job object and TCP listener ownership lookup (Windows).
- Band-limited resampler: pass-band gain, alias rejection above 8 kHz, tone frequency, edge handling, unusual sample rates, and clamping.
- In-memory WAV encoding and round trip.
- Capture channel locking, the end-of-speech timeouts on empty polls, cloud-tagged Ollama model refusal, and cleanup-error redaction.
- Engine planning (`plan_asr_engine`): GGUF models in-process, Whisper models in the server, low-resource mode, and missing pieces.
- Read-aloud: text preparation and bounding, synthesis deadlines, the Fish S2 request against a stand-in HTTP server (text sent, WAV decoded, error status reported without the response body, junk audio rejected), missing-engine handling, install discovery, and playback sample preparation.
- Runtime folders (`runtime_paths.rs`): the search order (bundled, per-user, then the checkout only with `dev-runtime`), and "missing" messages that name files and scripts but no folder.
- The readiness checklist (`readiness.rs`): every item's ready state and fix for empty, partial and complete installs in temporary folders, Ollama answers (model present or missing, not answering, timed out, cloud-tagged), model names that are not echoed into a command, no path in any label or fix, and the camelCase shape the settings window reads.
- Dictation's tap-or-hold rules (`gesture.rs`): a quick release goes hands-free and a late one ends push-to-talk, silence never stops a held hotkey but the maximum length does, a second tap stops hands-free only after 700 ms, a press within 700 ms of an automatic stop is ignored, and the End-of-speech wait is clamped to 300-3000 ms.
- The recovery slot (`recovery.rs`): a late worker never replaces a newer transcript, a superseded transcript is kept and announced once, and only while it is still the one Copy last transcript returns.
- The paste's checks: a focus change inside the same window and an unreadable focus at one end block it, higher-integrity (administrator) targets are detected, unknown integrity levels are not treated as blocked, and the modifier-key wait returns at once, waits, or gives up.
- The clipboard: privacy markers make content private (an unreadable marker too), the restore plan for text, private content, other formats and a clipboard changed since the paste, the restore timing after the target's read, another program's read and no read, never earlier than the fixed delay, and longer waits in a remote session.
- Messages, logs and health: user folders removed from messages, log lines and diagnostics; the log rotates at its cap, keeps whole records and counts an existing file; desktop health keeps no microphone name, loads older files that had one, and reports the last crash.
- Settings storage: a damaged database is moved aside and replaced while a healthy one is kept, a data folder that cannot be made runs on defaults and says why, and settings load leniently (unknown values reset, out-of-range numbers clamped, readable list entries kept, a section of the wrong kind defaulted) with a copy of the original row kept.
- The Ollama model check: `/api/show` answers that name a remote host or model are refused whatever the model is called, answers that describe no model are refused, and cloud entries are left out of the installed list.
- The settings window (`tests/app.test.tsx` and others): only the screens that do something, the Home problem banner and how the last conversation ended, the Setup checklist shown only while something required is missing, save failures reported and editing locked when settings cannot load, validated number and text fields, list rows that are never deleted by emptying a field, read-aloud state, the diagnostics export removing the user folder and microphone names, the hotkey table, the privacy facts, and the version from the build.

Run everything with:

```powershell
.\scripts\Run-Checks.ps1
```

It runs the frontend checks (`npm run format`, `lint`, `test`, `build`), then in `src-tauri`
`cargo fmt --check`, `cargo test --release --lib` and `cargo build --release --examples`.

23 Rust tests are ignored by default because they need local models, a running Ollama, a
built stand-in program, real hardware or the real clipboard, or are timing runs (the full
table is in `DEVELOPMENT.md`):

```powershell
cd src-tauri
cargo test --release --lib real_ -- --ignored --test-threads=1          # real models: Parakeet, Whisper server, Silero, Smart Turn, Phonon, and Ollama with llama3.2:3b
cargo test --release --lib _speed -- --ignored --nocapture             # timing runs
cargo build --release --example fake_agent
cargo test --release --lib coding_agents -- --ignored                  # background-question runner against the stand-in
cargo build --release --example wt_probe
$env:LOCALFLOW_LIVE_WT = "1"
cargo test --release --lib live_windows_terminal -- --ignored --nocapture   # opens a Windows Terminal tab
$env:LOCALFLOW_LIVE_CLIPBOARD = "1"
cargo test --release --lib live_delayed_rendering -- --ignored         # uses the real clipboard for a moment
```

In Git Bash the last one is
`LOCALFLOW_LIVE_CLIPBOARD=1 cargo test --release --lib live_delayed_rendering -- --ignored`.
It replaces the clipboard briefly and puts its text back; it skips itself unless the
clipboard holds plain text that no program marked private.

The GPU build has one more, `real_parakeet_gpu_agrees_with_the_cpu_on_the_bench_corpus`
(`--features gpu-vulkan`).

The Rust crate now compiles transcribe.cpp from source, so `cmake` must be on `PATH` for the
first build. The Visual Studio Build Tools ship one under
`Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin`. The debug profile's build
cache is large (it reached 13 GB); running tests with `--release` reuses the release cache.

Read-aloud has no automated end-to-end test: it needs the s2.cpp engine built with
`scripts\Build-FishS2.ps1`. Manual check: dictate a sentence, choose "Read last transcript
aloud" from the tray, confirm speech plays, confirm "Stop reading" interrupts it, confirm
that stopping while it is still preparing speech ends `s2.exe`, and confirm no `s2.exe`
process remains five minutes after the last reading or after Quit.

## Speech-to-Text Benchmark

`src-tauri/examples/asr_bench.rs` runs audio through the same provider code as the app and
reports latency percentiles, real-time factor, word error rate, peak memory, crash recovery
time, behaviour under concurrent requests, and whether silence produces text.

```powershell
cd src-tauri
cargo run --release --example asr_bench -- `
  --passes 3 --idle-secs 30 `
  --config inprocess:parakeet-tdt-0.6b-v3-Q5_K_M.gguf:1:6 `
  --config server:ggml-tiny.en-q5_1.bin:1:6 `
  --json ..\docs\benchmarks\my-run.json
```

A config is `engine:model:beam:threads[:context]` with engines `inprocess`, `server`, `cli`. Corpus preparation, results, and limits
are documented in `benchmarks/ASR_BENCHMARK.md`. Close other heavy applications first and do
not compile while it runs; the numbers are sensitive to CPU load.

`scripts/Measure-OllamaCleanup.ps1` measures the background cleanup request.

Latest verified results, on the development machine:

- `npm run format`
- `npm run lint`
- `npm run test` with 257 passing tests (2026-11-25, after the hardening round). Earlier: 163.
- `npm run build`
- `cargo fmt --check`
- `cargo test --release --lib` with 850 passing tests and 23 ignored (2026-11-25, after the hardening round). Earlier: 722 passing and 21 ignored (2026-11-24, after the PR #2 review fixes), when the real-model, stand-in and live Windows Terminal tests passed with the models installed and the examples built; 497 passing and 7 ignored (2026-11-23).
- `cargo test --release --lib real_phonon -- --ignored` and `real_silero -- --ignored`, both passing with the models installed.
- `npm run tauri:build`, producing release EXE, MSI, and NSIS setup EXE (a local build with the development runtime; the release workflow's installers have not been run yet).
- `.\scripts\Start-LocalFlow.ps1`, launching the release app without Vite.
- `.\scripts\Check-Ollama.ps1`, confirming local model `llama3.2:3b`.
- Direct local Ollama generate smoke test with `llama3.2:3b`, returning strict cleanup JSON.

## Manual Dictation Checklist

1. Short personal message.
2. Short work message.
3. Long paragraph.
4. Numbered list.
5. Bullet list.
6. Person name from dictionary.
7. Acronym from dictionary.
8. Technical term replacement.
9. Currency amount.
10. Date.
11. Email address.
12. URL.
13. Code identifier.
14. Explicit correction.
15. False start.
16. Quiet speech.
17. Background noise.
18. No speech.
19. Microphone disconnection.
20. Window switch while processing.
21. Browser single-line input.
22. Browser content-editable field.
23. Notepad.
24. Visual Studio Code.
25. Multiline text area.

Manual acceptance tests must record exact app version, model, hardware, and observed latency. Do not invent performance claims.

## Manual Overlay Visualization Checklist

Run with the packaged app and a real microphone; record hardware, Windows scaling, and observed frame smoothness for each item.

1. Quiet room, no speech: ridges show slow ambient breathing, no false speech peaks; auto-cancel hides the overlay after the no-speech timeout.
2. Soft speech: visible but gentle peaks near the centre; no visual collapse between words.
3. Loud speech: tall peaks that recede into the rear ridges over about a second; peaks compress smoothly near the top of the pill without clipping.
4. Vowels against consonants: vowels ("ah", "oo") raise the centre (low frequencies, pale green to cyan); sibilants ("s", "sh") raise the outer parts (high frequencies, blue to violet). The picture is mirrored left to right.
5. A steady background sound (fan, hum): visible at first, then sinks out of view within a few seconds while speech still shows clearly.
6. Sudden short utterance: fast attack, slower settle; no jump or flicker.
7. Long continuous dictation: motion stays smooth for 60+ seconds; no drift, memory growth, or slowdown.
8. Processing and refining: terrain stops following the microphone; a slow traveling crest sweeps the pill, slower during refining.
9. Escape during listening: overlay energy collapses and the window hides without inserting.
10. Successful insertion: terrain flattens toward a calm line with a brief green tint, then hides on the existing timing.
11. Insertion failure (change focus before paste completes): restrained red tint; transcript recoverable via Copy Last Transcript.
12. Several consecutive dictations: each session starts from a calm terrain (no leftover energy from the previous session).
13. Move the target app across monitors mid-session: overlay repositions to the active monitor and stays sharp.
14. 100%, 125%, 150%, and 200% Windows scaling: dots stay crisp (no blur, no oversized canvas).
15. Low-resource mode: overlay remains functional; reduced density is acceptable.
16. Sleep/resume: next dictation renders normally.
17. Open and close the settings window during dictation: overlay animation and insertion are unaffected.
18. Windows "reduce animation" / `prefers-reduced-motion`: displacement and travel are reduced but level feedback remains visible.

## Current Native Smoke Test

1. Start the packaged app with `.\scripts\Start-LocalFlow.ps1`.
2. Open a local text target such as Notepad.
3. Click in the target field.
4. Tap `Ctrl+Alt+Space`, or `Ctrl+Alt+Shift+Space` if the primary hotkey is unavailable.
5. Speak a short sentence.
6. Pause briefly after speaking, or press the hotkey again.
7. Confirm only the small frosted waveform pill appears above the taskbar while listening, with no larger transparent rectangle or outline; the waveform should change with speech pitch and volume, then switch to processing/refining after the pause or second hotkey press.
8. Confirm deterministic text appears in the target field quickly after speech recognition finishes; local Ollama cleanup should continue in the background and update Diagnostics/Copy Last Transcript when available.
9. Open Diagnostics, click Refresh, and record `Latency: speech end to visible text`, `Latency: Whisper sidecar` (the speech-recognition time), `Latency: Ollama cleanup`, and the `ASR engine` row. With Parakeet installed the engine should read `transcribe-cpp` with the Parakeet model; `fallback engine used` means it failed and the Whisper CLI took over.
10. If you select a Whisper model instead, confirm one `whisper-server.exe` process exists after the first dictation, that it is gone after Quit from the tray, and that it is also gone after ending `localflow.exe` from Task Manager.
11. Press Win+V after a dictation and confirm the dictated text is not in clipboard history.
12. Confirm `Get-NetTCPConnection -LocalPort 1420 -State Listen` returns nothing during this release-app smoke test.

This validates microphone capture, end-of-speech auto-stop, pitch-reactive overlay events, local speech recognition (Parakeet in-process, or the whisper.cpp fallback), deterministic quick insert, background local Ollama `llama3.2:3b` cleanup, JSON parsing, and clipboard paste for the current native path.

## Manual Checks for the November 2026 Changes

These behaviours are unit-tested; these steps check them in the real app. Record the build
and what you saw.

1. **Tap.** Tap `Ctrl+Alt+Space`, speak and pause: it stops after the End-of-speech wait
   (900 ms by default). Set the wait to 2000 ms in Settings > Dictation and confirm a 1 s
   pause no longer ends it.
2. **Hold.** Hold the hotkey, speak with a pause of several seconds, then let go: it does not
   stop before you let go.
3. **The 700 ms rule.** Tap, speak, let it stop by itself, and tap again at once: no new
   recording starts and the first dictation is pasted.
4. **Held modifiers.** Hold the hotkey, speak, let go of Space only and keep `Ctrl+Alt` down
   for 3 s: nothing is pasted, and Home says "Keys were still held".
5. **Focus inside a window.** In a classic Windows dialog with two text boxes (each its own
   window), start a dictation in one and click into the other before it is pasted: nothing
   is pasted, and Home offers Copy last transcript. In a browser page the same move is not
   detected (see the user guide).
6. **Administrator window.** Dictate into Notepad started with "Run as administrator":
   nothing is typed, and Home says "Can't paste into an administrator window".
7. **Superseded dictation.** Turn on Low-resource mode, so that transcription takes longer.
   Hold the hotkey through a long sentence, let go, then at once tap it again and press
   `Escape`: the first dictation is not pasted, Home says "Previous dictation was not
   pasted", and Copy last transcript returns the first dictation's text. (A press within
   700 ms of a dictation that stopped by itself is ignored, which is why this uses a hold.)
8. **Clipboard.** Copy some text, dictate, then press `Win+V`: the earlier text is on the
   clipboard and neither text is in the history. Copy a password from a password manager,
   dictate, and confirm the clipboard is empty afterwards.
9. **Tray and single instance.** Close the settings window: LocalFlow stays in the tray.
   Start LocalFlow from the Start menu: the window comes back and Task Manager shows one
   `localflow.exe`.
10. **Quit.** Choose Quit in the tray while a dictation is transcribing: the process ends
    within a few seconds and no `whisper-server.exe` or `s2.exe` remains.
11. **Log.** Open `%LOCALAPPDATA%\app.localflow.desktop\logs\localflow.log`: it has lines from
    this session, no dictated text and no user folder.

Use `.\scripts\Start-Dev.ps1` only when intentionally testing the developer Vite/Tauri loop.

## Installer Smoke Test (not yet run)

On a clean Windows 11 computer or virtual machine without Visual Studio, Rust or the Visual
C++ redistributable:

1. Check both installers from a draft release against `SHA256SUMS`.
2. Install with the `-setup.exe` (no administrator prompt expected), and separately with the
   `.msi`. Confirm the install folder holds `localflow.exe`, `msvcp140.dll`,
   `vcruntime140.dll`, `vcruntime140_1.dll`, `LICENSE.txt`, `THIRD_PARTY_NOTICES.md`,
   `scripts\` and `localflow-runtime\whisper\Release\` with the Visual C++ DLLs beside the
   whisper.cpp programs.
3. Start LocalFlow from the Start menu with no models downloaded: the readiness checklist
   lists what is missing with a script to run, and dictation works with the bundled tiny.en.
4. Run the model scripts from the install folder as in the user guide; confirm the files land
   in `%LOCALAPPDATA%\LocalFlow` and that LocalFlow then uses Parakeet and starts a
   conversation.
5. Select a Whisper model and dictate, to start `whisper-server.exe` from the install folder.
6. Uninstall; confirm the models in `%LOCALAPPDATA%\LocalFlow` remain.
