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

## Waveform Does Not Appear

LocalFlow launches the settings window hidden and shows a small overlay only during active dictation states. If no overlay appears, check that the `localflow.exe` process is running and that the global hotkey is registered.

## Ollama Is Unavailable

Run:

```powershell
.\scripts\Check-Ollama.ps1
```

Native hotkey dictation is pinned to local Ollama model `gemma4:12b-it-qat`. If `gemma4:12b-it-qat` is missing or Ollama is not running, LocalFlow logs the cleanup failure and preserves the raw Whisper transcript rather than dropping the dictation.
