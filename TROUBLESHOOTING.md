# Troubleshooting

## Rust Or MSVC Is Missing

Install common Windows development prerequisites:

```powershell
.\scripts\Install-Prereqs.ps1 -Install
```

The script checks Node, npm, Git, Rust/Cargo, and Microsoft Visual Studio C++ Build Tools with the VCTools workload. Restart the terminal if a newly installed tool is still missing from PATH.

## Port 1420 Is Busy

The Tauri dev server expects port 1420. Stop the process using that port or adjust `src-tauri/tauri.conf.json` and `vite.config.ts` together.

## Hotkey Does Not Trigger

LocalFlow first tries `Ctrl+Alt+Space`. If another app owns it, LocalFlow registers `Ctrl+Alt+Shift+Space` instead and logs the fallback in `.localflow-tauri-dev.log`.

## Whisper Model Not Found

The dev runtime expects:

```text
.localflow-runtime\whisper\Release\whisper-cli.exe
.localflow-runtime\models\ggml-tiny.en-q5_1.bin
```

You can override either path:

```powershell
$env:LOCALFLOW_WHISPER_CLI = "C:\path\to\whisper-cli.exe"
$env:LOCALFLOW_WHISPER_MODEL = "C:\path\to\ggml-base.en.bin"
```

## Nothing Is Inserted

Click into the target text field before holding the hotkey. The current native path uses clipboard paste fallback, so the focused app must accept `Ctrl+V`.

## Dictation Feels Slow To Finish

Native dictation listens for end-of-speech after the shortcut starts recording. Tap the shortcut, speak, then pause briefly; a short post-speech silence triggers transcription automatically. You can also press the hotkey again to stop manually, or use a longer hold-and-release gesture. If no voice arrives after a tap, LocalFlow times out instead of staying open indefinitely. A configurable end-of-speech control is still pending.

## Waveform Opens Then Immediately Closes

Update to the latest build from this repository. Quick shortcut releases are treated as tap-to-start, so the waveform should stay open long enough for you to speak. If it still closes immediately, check whether another app is also intercepting `Ctrl+Alt+Space` or `Ctrl+Alt+Shift+Space`.

## Terminal Window Flashes During Dictation

Update to the latest build from this repository. LocalFlow launches the bundled `whisper-cli.exe` sidecar with the Windows no-console flag, so ordinary hotkey dictation should show only the small waveform overlay.

## Blank Or Silent Audio

LocalFlow now rejects near-silent recordings before Whisper runs and reports the opened input device with peak/RMS diagnostics in the native log. The capture path also avoids multi-channel phase cancellation by selecting the loudest active input channel instead of averaging channels together.

If silence is still reported, check the Windows default input device, input gain, and microphone privacy access for desktop apps.

## Waveform Does Not Appear

LocalFlow launches the settings window hidden and shows a small overlay only during active dictation states. While you speak, the colored ribbon should move with microphone level and pitch. If no overlay appears, check that the `localflow.exe` process is running and that the global hotkey is registered.

## Ollama Is Unavailable

Run:

```powershell
.\scripts\Check-Ollama.ps1
```

Native hotkey dictation uses a configurable local Ollama model (default `llama3.2:3b`; run `ollama pull llama3.2:3b`). If the selected model is missing or Ollama is not running, LocalFlow logs the cleanup failure and inserts the deterministically formatted transcript rather than dropping the dictation. For the fastest possible insertion, enable low-resource mode in Settings > Models to skip the LLM entirely.
