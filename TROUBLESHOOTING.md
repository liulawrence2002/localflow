# Troubleshooting

## Rust Is Missing

If `cargo --version` fails, install Rust stable and restart the terminal:

```powershell
.\scripts\Install-Prereqs.ps1
```

Then run:

```powershell
npm run tauri:dev
```

## Port 1420 Is Busy

The Tauri dev server expects port 1420. Stop the process using that port or adjust `src-tauri/tauri.conf.json` and `vite.config.ts` together.

## Ollama Is Unavailable

Run:

```powershell
.\scripts\Check-Ollama.ps1
```

The Models screen can check the local Ollama API and populate installed local models. Shared provider errors distinguish unavailable Ollama, no selected model, missing local model, and blocked remote URLs.

The desktop-native dictation workflow still uses the mock path until real ASR and insertion are wired.

## Whisper Model Not Found

Configure a local model path:

```powershell
.\scripts\Set-WhisperModelPath.ps1 -ModelPath "C:\models\ggml-base.en.bin"
```

Milestone 2 will add validation in the app UI.
