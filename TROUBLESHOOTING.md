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

Native dictation now listens for end-of-speech while the hotkey is held. After speech is detected, a short pause triggers transcription automatically; releasing the hotkey still finishes immediately. If it stops too early, speak in one steadier pass for now. A configurable end-of-speech control is still pending.

## Blank Or Silent Audio

LocalFlow now rejects near-silent recordings before Whisper runs and reports the opened input device with peak/RMS diagnostics in the native log. The capture path also avoids multi-channel phase cancellation by selecting the loudest active input channel instead of averaging channels together.

If silence is still reported, check the Windows default input device, input gain, and microphone privacy access for desktop apps.

## Waveform Does Not Appear

LocalFlow launches the settings window hidden and shows a small overlay only during active dictation states. While you speak, the blue bars should move with microphone level. If no overlay appears, check that the `localflow.exe` process is running and that the global hotkey is registered.

## Ollama Is Unavailable

Run:

```powershell
.\scripts\Check-Ollama.ps1
```

Native hotkey dictation is pinned to local Ollama model `gemma4:12b-it-qat`. If `gemma4:12b-it-qat` is missing or Ollama is not running, LocalFlow logs the cleanup failure and preserves the raw Whisper transcript rather than dropping the dictation.
