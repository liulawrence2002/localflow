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

Run:

```powershell
.\scripts\Run-Checks.ps1
```

- Engine planning (`plan_asr_engine`): GGUF models in-process, Whisper models in the server, low-resource mode, and missing pieces.
- Read-aloud: text preparation and bounding, synthesis deadlines, the Fish S2 request against a stand-in HTTP server (text sent, WAV decoded, error status reported without the response body, junk audio rejected), missing-engine handling, install discovery, and playback sample preparation.

Seven Rust tests are ignored by default because they need local assets or a release build:

```powershell
cd src-tauri
cargo test --release --lib real_ -- --ignored                          # real whisper-server and Parakeet model
cargo test --release --lib resampler_speed -- --ignored --nocapture    # resampler timing
```

The Rust crate now compiles transcribe.cpp from source, so `cmake` must be on `PATH` for the
first build. The Visual Studio Build Tools ship one under
`Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin`. The debug profile's build
cache is large (it reached 13 GB); running tests with `--release` reuses the release cache.

Read-aloud has no automated end-to-end test: it needs the s2.cpp engine built with
`scripts\Build-FishS2.ps1`. Manual check: dictate a sentence, choose "Read last transcript
aloud" from the tray, confirm speech plays, confirm "Stop reading" interrupts it, and confirm
no `s2.exe` process remains five minutes later or after Quit.

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

Current verification on this workstation:

- `npm run format`
- `npm run lint`
- `npm run test` with 163 passing tests.
- `npm run build`
- `cargo fmt --check`
- `cargo test --release --lib` with 496 passing tests and 7 ignored (2026-11-23, with natural turn-taking).
- `cargo test --release --lib real_phonon -- --ignored` and `real_silero -- --ignored`, both passing with the models installed.
- `cargo check`
- `npm run tauri:build`, producing release EXE, MSI, and NSIS setup EXE.
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
8. Confirm deterministic text appears in the target field quickly after local Whisper finishes; local Ollama cleanup should continue in the background and update Diagnostics/Copy Last Transcript when available.
9. Open Diagnostics, click Refresh, and record `Latency: speech end to visible text`, `Latency: Whisper sidecar`, `Latency: Ollama cleanup`, and the `ASR engine` row. The engine should read `transcribe-cpp` with the Parakeet model; `fallback engine used` means it failed and the Whisper CLI took over.
10. If you select a Whisper model instead, confirm one `whisper-server.exe` process exists after the first dictation, that it is gone after Quit from the tray, and that it is also gone after ending `localflow.exe` from Task Manager.
11. Press Win+V after a dictation and confirm the dictated text is not in clipboard history.
12. Confirm `Get-NetTCPConnection -LocalPort 1420 -State Listen` returns nothing during this release-app smoke test.

This validates microphone capture, end-of-speech auto-stop, pitch-reactive overlay events, local `whisper.cpp` execution, deterministic quick insert, background local Ollama `llama3.2:3b` cleanup, JSON parsing, clipboard paste, and temporary file cleanup for the current native path.

Use `.\scripts\Start-Dev.ps1` only when intentionally testing the developer Vite/Tauri loop.
