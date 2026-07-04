# Testing

## Automated Tests

Current tests cover:

- State-machine transitions.
- Overlapping-session rejection.
- Transcript stable-prefix commitment.
- Duplicate-free final transcript commitment.
- Boundary-aware replacements.
- Snippet expansion.
- Spoken punctuation.
- LLM JSON validation.
- Cursor-aware insertion spacing.

Run:

```powershell
npm run test
```

Rust tests should be run after installing Rust:

```powershell
cd src-tauri
cargo test
```

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
