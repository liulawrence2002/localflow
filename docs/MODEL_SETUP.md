# Model Setup

LocalFlow never downloads a model by itself, and normal builds and tests download nothing.
Every model comes from a script you run. Each script downloads one pinned revision over
HTTPS, checks every file against its SHA-256 before it replaces anything, and can be run
again safely: files that already match are left alone.

## Where models go

| You are using                             | Models go to                              | Scripts are in                     |
| ----------------------------------------- | ----------------------------------------- | ---------------------------------- |
| The `-setup.exe` installer (just for you) | `%LOCALAPPDATA%\LocalFlow`                | `%LOCALAPPDATA%\LocalFlow\scripts` |
| The `.msi` installer (for all users)      | `%LOCALAPPDATA%\LocalFlow`, for each user | `%ProgramFiles%\LocalFlow\scripts` |
| A source checkout (a folder with `.git`)  | `.localflow-runtime` in the checkout      | `scripts` in the checkout          |

Both folders have the same layout: `models\` (speech, voice detection, end of turn),
`models\phonon\` (the conversation voice), `models\fish\` and `fish\` (the optional Fish
voice), and `whisper\Release\` (the whisper.cpp programs, for source builds). Every script
takes `-Destination <folder>` to put files somewhere else.

LocalFlow looks for each model in this order and uses the first it finds:

1. `localflow-runtime` next to `localflow.exe`: what the installer ships (the whisper.cpp
   fallback programs and Whisper tiny.en).
2. `%LOCALAPPDATA%\LocalFlow`.
3. Only in builds from source with the default `dev-runtime` feature: the checkout's
   `.localflow-runtime`. Release installers are built without it and never look there.

The `-setup.exe` installer installs LocalFlow itself into `%LOCALAPPDATA%\LocalFlow` too, so
the app's files and your models share that folder. Uninstalling removes only the app's files;
delete the folder by hand to remove the models.

## Running the scripts

Windows does not run PowerShell scripts by default, so pass `-ExecutionPolicy Bypass` for the
one command. For an installed copy (with the `.msi`, use `$env:ProgramFiles` instead of
`$env:LOCALAPPDATA`):

```powershell
$scripts = "$env:LOCALAPPDATA\LocalFlow\scripts"
powershell -ExecutionPolicy Bypass -File "$scripts\Get-AsrModel.ps1"          # Parakeet, 549 MB
powershell -ExecutionPolicy Bypass -File "$scripts\Get-PhononModel.ps1"       # the voice, about 65 MB
powershell -ExecutionPolicy Bypass -File "$scripts\Get-VadModel.ps1"          # voice detection, 1.2 MB
powershell -ExecutionPolicy Bypass -File "$scripts\Get-SmartTurnModel.ps1"    # end of turn, 32 MB
```

From a source checkout, run them as `.\scripts\Get-AsrModel.ps1` and so on.

| Script                            | Model                                   | Size        | Needed for                                                             | Source, pinned revision                                |
| --------------------------------- | --------------------------------------- | ----------- | ---------------------------------------------------------------------- | ------------------------------------------------------ |
| `Get-AsrModel.ps1`                | NVIDIA Parakeet TDT 0.6B v3, Q5_K_M     | 549 MB      | Dictation and conversation; the default speech model                   | `handy-computer/parakeet-tdt-0.6b-v3-gguf` @ `d8d8c32` |
| `Get-AsrModel.ps1 -Model tiny.en` | Whisper tiny.en, q5_1                   | 32 MB       | The fallback. The installer contains it; source builds need it         | `ggerganov/whisper.cpp` @ `5359861`                    |
| `Get-PhononModel.ps1`             | Gradium Phonon 31m (English), q8        | about 65 MB | Conversation and reading aloud. Required for both                      | `gradium/phonon-31m-en` @ `92741c5`                    |
| `Get-VadModel.ps1`                | Silero VAD v6.2                         | 1.2 MB      | Conversation. Without it a loudness rule starts turns                  | `handy-computer/silero-vad-v6.2-gguf` @ `a9b2d61`      |
| `Get-SmartTurnModel.ps1`          | Pipecat Smart Turn v3.2, full precision | 32 MB       | Conversation. Without it the words alone decide when you have finished | `pipecat-ai/smart-turn-v3` @ `f766f81`                 |
| `Get-FishModel.ps1`               | Fish Audio S2 Pro, q4_k_m (optional)    | 3.6 GB      | "Read last transcript aloud" (see below). Non-commercial use only      | `rodrigomt/s2-pro-gguf` @ `a732069`                    |
| `Get-WhisperRuntime.ps1`          | whisper.cpp v1.9.5 programs             | 9 MB        | Source builds; the installer contains them                             | `ggml-org/whisper.cpp` release `b5454`                 |

The model licences and credits are in [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md).

## Speech recognition

### Choosing a speech model

| Model                                       | Engine                                 | Median, 5.8 s median clip | Word error rate | Memory                    | How it is selected                                         |
| ------------------------------------------- | -------------------------------------- | ------------------------- | --------------- | ------------------------- | ---------------------------------------------------------- |
| `parakeet-tdt-0.6b-v3-Q5_K_M.gguf` (549 MB) | in-process transcribe.cpp, kept loaded | 534 ms                    | 0.93 %          | about 1.2 GB while loaded | default when the file is present                           |
| `ggml-tiny.en-q5_1.bin` (32 MB)             | persistent whisper server              | 453 ms                    | 3.31 %          | about 130 MB              | default when Parakeet is not installed; fallback otherwise |

Measured on CPU; details in `benchmarks/ASR_BENCHMARK.md`. Parakeet is quicker than tiny.en
on short dictations (334 ms against 424 ms for clips up to 5 s) and slower on long ones
(1.1 s against 0.5 s beyond 10 s). It writes its own punctuation and does not invent text
for silence.

Any other GGUF model supported by transcribe.cpp, or any ggml Whisper model, can be used by
putting its full path in Settings > Models > Speech model file; `.gguf` files run in-process
and `ggml-*.bin` files run in the whisper.cpp sidecars. If the configured file is missing,
LocalFlow uses an installed model instead of failing.

If the in-process engine fails, LocalFlow falls back to the one-shot Whisper CLI with
tiny.en when that model is installed. Low-resource mode uses that CLI path on purpose so
that no model stays in memory between dictations.

Two environment variables override the choice:

```powershell
$env:LOCALFLOW_WHISPER_MODEL = "C:\path\to\ggml-base.en.bin"   # any build: a model file
$env:LOCALFLOW_WHISPER_CLI = "C:\path\to\whisper-cli.exe"      # source builds only
```

Release builds ignore `LOCALFLOW_WHISPER_CLI`: they start programs only from their own
folder and `%LOCALAPPDATA%\LocalFlow`.

The native hotkey path resamples the recording to mono 16 kHz, transcribes it with the loaded speech model (dictionary terms are passed as vocabulary where the model supports it, or as a prompt for Whisper), applies deterministic formatting, verifies the target window, inserts the text, and then asks your configured local Ollama model for a background cleanup. The model is loaded when recording starts and freed when LocalFlow quits. Every run has a deadline; a run that passes it is cancelled.

GPU acceleration: an app built with `npm run tauri:build:gpu` (the `gpu-vulkan` feature, which needs the Vulkan SDK) runs Parakeet on the discrete GPU and falls back to the CPU if there is none or it fails; set `LOCALFLOW_ASR_DEVICE=cpu` to force the CPU. The release installers are CPU builds. See `benchmarks/ASR_SPEED_BENCHMARK.md`.

Still pending: streaming partial transcripts, and unloading the speech model after a period without use.

## Conversation mode models

Conversation mode needs the voice, and works better with two more small models.

| Model                                                     | Used for                                                                        | Licence      | Where it runs                                                                           |
| --------------------------------------------------------- | ------------------------------------------------------------------------------- | ------------ | --------------------------------------------------------------------------------------- |
| Gradium Phonon 31m, q8 (`models\phonon\`)                 | the assistant's voice, English                                                  | Apache-2.0   | in-process, at most four CPU threads, about 300 MB of memory while a conversation is on |
| Silero VAD v6.2 (`models\silero-vad-v6.2-F32.gguf`)       | telling speech from other sound, finding the end of a turn                      | MIT          | in-process, through the speech library                                                  |
| Smart Turn v3.2, fp32 (`models\smart-turn-v3.2-gpu.onnx`) | hearing from the voice whether a turn sounds finished, alongside the text judge | BSD-2-Clause | in-process on the CPU (tract), up to four threads, at each pause                        |

Without the voice, conversation mode will not start and says which script to run. Without
the Silero model it starts with a loudness rule instead, which any loud sound can trigger.
The language model is the Ollama model chosen in Settings > Models (default `llama3.2:3b`),
the same one dictation cleanup uses. Models that do not fit entirely in GPU memory answer
too slowly for conversation.

## Read-aloud voice (Fish Audio S2 Pro)

LocalFlow can read the last transcript aloud with Fish Audio S2 Pro running locally through
[s2.cpp](https://github.com/rodrigomatta/s2.cpp). Nothing is bundled; both pieces are
explicit steps, and both must go to the same folder (the same `-Destination`, or the
default for both):

```powershell
.\scripts\Get-FishModel.ps1                   # 3.6 GB model (q4_k_m) and tokenizer
.\scripts\Build-FishS2.ps1                    # CPU engine; or -Backend vulkan / -Backend cuda
```

Building the engine needs Git and the Visual Studio Build Tools with the C++ CMake tools.
It compiles in a fresh folder in your user profile (`-BuildDir` to change it) and installs
only `s2.exe` and the ggml DLLs the backend needs.

Then use the tray item "Read last transcript aloud" or "Read aloud with Fish S2" under
Recovery on the Home screen. "Stop reading" interrupts playback. Reading a transcript aloud
stops a window being read aloud.

What to expect:

- **Status.** The engine was built and run on the development machine (Core Ultra 7 165H,
  RTX 4070 Laptop 8 GB) on CPU, Vulkan, and CUDA, both from the command line and through the
  same local server endpoint the app uses. Reading aloud from inside the running app, and
  how the voice sounds, were not checked by a person in that session.
- **Speed (measured, q4_k_m).** S2 Pro is a 4B-parameter model and is slower than real time
  on every backend tried:

  | Backend                | Speed                                                                 | Start-up   |
  | ---------------------- | --------------------------------------------------------------------- | ---------- |
  | CUDA (`-Backend cuda`) | about 2.8 times slower than real time; a 4 s sentence took 11 to 12 s | about 35 s |
  | CPU (default)          | about 11 times slower than real time; 2 s of speech took 23 s         | about 4 s  |
  | Vulkan                 | about 17 times slower than real time on either GPU                    | 45 to 55 s |

  CUDA is the only practical choice here and used about 3.3 GB of VRAM. It needs the CUDA
  Toolkit 12.4 or later at build time only; the script header explains how to supply just
  the components it needs without a system-wide install. The CUDA engine folder is about
  740 MB because it carries the cuBLAS runtime.

- **Memory.** The engine is started only when read-aloud is used and is stopped after five
  minutes without use, so the 3.6 GB model is not held in memory otherwise. Stopping a
  reading while it is still being prepared stops the engine too, so the next reading loads
  the model again.
- **Licence.** The S2 Pro weights are under the Fish Audio Research License: research and
  personal non-commercial use only. This is why they are never bundled with the installer.
- **Maturity.** s2.cpp describes itself as alpha software. The build script pins the commit
  this integration was written against.
- **Privacy.** Text is sent only to the sidecar on `127.0.0.1`. No voice cloning is used.

## Ollama

Install Ollama from [ollama.com](https://ollama.com) and make sure the local model you want
is available. The default is a fast small model, `llama3.2:3b` — pull it once:

```powershell
ollama pull llama3.2:3b
```

It is chosen for low latency: LocalFlow's deterministic layer already handles punctuation,
self-corrections, filler cleanup, and your replacements/snippets, so the LLM only does light
semantic cleanup and a small model is plenty. You can change the model in Settings > Models
— good alternatives are `qwen2.5:3b` (quality) or `gemma4:12b-it-qat` (heaviest, slowest) —
and native dictation uses your choice (it never silently switches to a remote model). To
skip the LLM entirely for the fastest possible insertion, enable low-resource mode; the
deterministically formatted text is inserted directly.

Check availability from a checkout with `.\scripts\Check-Ollama.ps1`, or from an installed
copy:

```powershell
powershell -ExecutionPolicy Bypass -File "$env:LOCALAPPDATA\LocalFlow\scripts\Check-Ollama.ps1"
```

In the desktop app the native layer lists the installed models (`/api/tags`); the web code's shared provider calls `http://127.0.0.1:11434/api/tags` and `/api/generate` only when the settings window runs in a browser during development, and only on port 11434 of this computer, refusing redirects. Native dictation calls `http://127.0.0.1:11434/api/generate` with the configured model, `stream: false`, JSON-format output, a fixed `num_ctx` of 4096, and `num_predict` capped at 1024. The fixed context matters: without it Ollama sized the context from the model's maximum and, on the development machine, failed with an out-of-memory error.

Models that run on Ollama's servers are refused: names with a cloud tag (`:cloud`, `-cloud`) at once, and any other model whose description from Ollama (`POST /api/show`, asked before text is sent) names a `remote_host` or `remote_model`, whatever it is called. A model Ollama cannot describe is refused too. Settings > Models leaves such models out of the installed list.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. If the model is unavailable or returns invalid JSON twice, LocalFlow falls back to the deterministically formatted transcript so the user does not lose dictated text.
