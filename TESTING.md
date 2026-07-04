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
- Boundary-aware replacements.
- Snippet expansion.
- Spoken punctuation.
- Explicit self-correction examples.
- Context snapshot privacy gates, context length limits, protected-field blocking, application category classification, and cleanup input mapping.
- LLM JSON validation.
- LLM JSON repair retry and deterministic fallback.
- Timeout handling.
- Performance metric recording, missing-measurement warnings, and latency/memory formatting.
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
- `.\scripts\Check-Ollama.ps1`, confirming local model `gemma4:12b-it-qat`.
- Direct local Ollama generate smoke test with `gemma4:12b-it-qat`, returning strict cleanup JSON.

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

## Current Native Smoke Test

1. Start the Tauri app with `.\scripts\Start-Dev.ps1`.
2. Open a local text target such as Notepad.
3. Click in the target field.
4. Tap `Ctrl+Alt+Space`, or `Ctrl+Alt+Shift+Space` if the primary hotkey is unavailable.
5. Speak a short sentence.
6. Pause briefly after speaking, or press the hotkey again.
7. Confirm the small waveform overlay appears while listening, the colored ribbon changes with speech pitch and volume, and it switches to processing/refining after the pause or second hotkey press.
8. Confirm cleaned text appears in the target field.

This validates microphone capture, end-of-speech auto-stop, pitch-reactive overlay events, local `whisper.cpp` execution, local Ollama `gemma4:12b-it-qat` cleanup, JSON parsing, clipboard paste, and temporary file cleanup for the current native path.
