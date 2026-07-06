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
- Native overlay audio-feature extraction, including pitch distinction for lower and higher voice tones.
- Topographic overlay surface math: deterministic heights, boundedness under extreme inputs, pitch-as-spacing (never loudness), soft edge clamping, and depth projection ordering.
- Overlay feature smoothing: frame-rate-independent attack/release, invalid-value (NaN/Infinity/null) clamping, bounded circular level history, and missed-event gap bounding.
- Overlay phase-to-visual mapping, including processing/refining sweeps, success/cancel collapse, restrained state tints, and reduced-motion behavior.
- Overlay renderer lifecycle: single animation loop under rapid state changes, frame cancellation on stop/dispose, pause while the document is hidden, listener removal on dispose, and dynamics reset when a new session starts.
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

Run:

```powershell
.\scripts\Run-Checks.ps1
```

Current verification on this workstation:

- `npm run format`
- `npm run lint`
- `npm run test` with 77 passing tests.
- `npm run build`
- `cargo fmt --check`
- `cargo test` with 14 passing tests.
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

1. Quiet room, no speech: terrain shows slow ambient breathing, no false speech ridges; auto-cancel hides the overlay after the no-speech timeout.
2. Soft speech: visible but gentle ridge response; no visual collapse between words.
3. Loud speech: strong central ridges propagating into rear rows; peaks compress smoothly near the pill edge without clipping.
4. Low-pitch speech: wider ridge spacing; overall brightness unchanged.
5. High-pitch speech: tighter ridge spacing; overall brightness unchanged (pitch must not read as loudness).
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
9. Open Diagnostics, click Refresh, and record `Latency: speech end to visible text`, `Latency: Whisper sidecar`, and `Latency: Ollama cleanup`.
10. Confirm `Get-NetTCPConnection -LocalPort 1420 -State Listen` returns nothing during this release-app smoke test.

This validates microphone capture, end-of-speech auto-stop, pitch-reactive overlay events, local `whisper.cpp` execution, deterministic quick insert, background local Ollama `llama3.2:3b` cleanup, JSON parsing, clipboard paste, and temporary file cleanup for the current native path.

Use `.\scripts\Start-Dev.ps1` only when intentionally testing the developer Vite/Tauri loop.
