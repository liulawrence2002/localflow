# Model Setup

LocalFlow does not download large model files during normal builds or tests.

## Whisper.cpp

This workstation has a local dev runtime in `.localflow-runtime/`:

- `models\parakeet-tdt-0.6b-v3-Q5_K_M.gguf` (549 MB): the default speech model
- `models\ggml-tiny.en-q5_1.bin` (32 MB): the small fallback model the installer bundles
- `whisper\Release\whisper-cli.exe` and `whisper-server.exe`: engines for the fallback model
- `models\fish\` and `fish\`: the optional read-aloud voice (see "Read-aloud voice" below)

The native app first checks explicit environment overrides, then bundled Tauri resources, then the dev `.localflow-runtime/` folder.

### Choosing a speech model

| Model | Engine | Median, 5.8 s median clip | Word error rate | Memory | How it is selected |
|---|---|---|---|---|---|
| `parakeet-tdt-0.6b-v3-Q5_K_M.gguf` (549 MB) | in-process transcribe.cpp, kept loaded | 534 ms | 0.93 % | about 1.2 GB while loaded | default when the file is present |
| `ggml-tiny.en-q5_1.bin` (32 MB) | persistent whisper server | 453 ms | 3.31 % | about 130 MB | default when Parakeet is not installed; fallback otherwise |

Measured on CPU; details in `benchmarks/ASR_BENCHMARK.md`. Parakeet is quicker than tiny.en
on short dictations (334 ms against 424 ms for clips up to 5 s) and slower on long ones
(1.1 s against 0.5 s beyond 10 s). It writes its own punctuation and does not invent text
for silence.

```powershell
.\scripts\Get-AsrModel.ps1                    # Parakeet, the default
.\scripts\Get-AsrModel.ps1 -Model tiny.en     # the small fallback model
```

LocalFlow never downloads a model by itself. Any other GGUF model supported by
transcribe.cpp, or any ggml Whisper model, can be used by putting its full path in
Settings > Models > Whisper model path; `.gguf` files run in-process and `ggml-*.bin` files
run in the whisper.cpp sidecars. If the configured file is missing, LocalFlow uses an
installed model instead of failing.

If the in-process engine fails, LocalFlow falls back to the one-shot Whisper CLI with
tiny.en when that model is installed. Low-resource mode uses that CLI path on purpose so
that no model stays in memory between dictations.

```powershell
$env:LOCALFLOW_WHISPER_CLI = "C:\path\to\whisper-cli.exe"
$env:LOCALFLOW_WHISPER_MODEL = "C:\path\to\ggml-base.en.bin"
```

The native hotkey path resamples the recording to mono 16 kHz, transcribes it with the loaded speech model (dictionary terms are passed as vocabulary where the model supports it, or as a prompt for Whisper), applies deterministic formatting, verifies the target window, inserts the text, and then asks your configured local Ollama model for a background cleanup. The model is loaded when recording starts and freed when LocalFlow quits. Every run has a deadline; a run that passes it is cancelled.

GPU acceleration: an app built with `npm run tauri:build:gpu` (the `gpu-vulkan` feature, which needs the Vulkan SDK) runs Parakeet on the discrete GPU and falls back to the CPU if there is none or it fails; set `LOCALFLOW_ASR_DEVICE=cpu` to force the CPU. See `benchmarks/ASR_SPEED_BENCHMARK.md`.

Still pending: streaming partial transcripts, and unloading the speech model after a period without use.

## Conversation mode models

Conversation mode needs two more small models. Both are explicit, hash-verified downloads
from a pinned revision; LocalFlow never downloads anything itself.

```powershell
.\scripts\Get-PhononModel.ps1     # the conversation voice, about 65 MB (required)
.\scripts\Get-VadModel.ps1        # voice-activity detection, 1.2 MB (recommended)
.\scripts\Get-SmartTurnModel.ps1  # end-of-turn audio model, about 32 MB (recommended)
```

| Model | Used for | Licence | Where it runs |
|---|---|---|---|
| Gradium Phonon 31m, q8 (`models\phonon\`) | the assistant's voice, English | Apache-2.0 | in-process, at most four CPU threads, about 300 MB of memory while a conversation is on |
| Silero VAD v6.2 (`models\silero-vad-v6.2-F32.gguf`) | telling speech from other sound, finding the end of a turn | MIT | in-process, through the speech library |
| Smart Turn v3.2, fp32 (`models\smart-turn-v3.2-gpu.onnx`) | hearing from the voice whether a turn sounds finished, alongside the text judge | BSD-2-Clause | in-process on the CPU (tract), up to four threads, at each pause |

Without the voice, conversation mode will not start and says which script to run. Without
the Silero model it starts with a loudness rule instead, which any loud sound can trigger.
The language model is the Ollama model chosen in Settings > Models (default `llama3.2:3b`),
the same one dictation cleanup uses. Models that do not fit entirely in GPU memory answer
too slowly for conversation.

## Read-aloud voice (Fish Audio S2 Pro)

LocalFlow can read the last transcript aloud with Fish Audio S2 Pro running locally through
[s2.cpp](https://github.com/rodrigomatta/s2.cpp). Nothing is bundled; both pieces are
explicit steps:

```powershell
.\scripts\Get-FishModel.ps1                   # 3.6 GB model (q4_k_m) and tokenizer
.\scripts\Build-FishS2.ps1                    # CPU engine; or -Backend vulkan / -Backend cuda
```

Then use the tray item "Read last transcript aloud" or the Read aloud button on the Home
screen. "Stop reading" interrupts playback.

What to expect:

- **Status.** The engine was built and run on the development machine (Core Ultra 7 165H,
  RTX 4070 Laptop 8 GB) on CPU, Vulkan, and CUDA, both from the command line and through the
  same local server endpoint the app uses. Reading aloud from inside the running app, and
  how the voice sounds, were not checked by a person in that session.
- **Speed (measured, q4_k_m).** S2 Pro is a 4B-parameter model and is slower than real time
  on every backend tried:

  | Backend | Speed | Start-up |
  |---|---|---|
  | CUDA (`-Backend cuda`) | about 2.8 times slower than real time; a 4 s sentence took 11 to 12 s | about 35 s |
  | CPU (default) | about 11 times slower than real time; 2 s of speech took 23 s | about 4 s |
  | Vulkan | about 17 times slower than real time on either GPU | 45 to 55 s |

  CUDA is the only practical choice here and used about 3.3 GB of VRAM. It needs the CUDA
  Toolkit 12.4 or later at build time only; the script header explains how to supply just
  the components it needs without a system-wide install. The CUDA engine folder is about
  740 MB because it carries the cuBLAS runtime.
- **Memory.** The engine is started only when read-aloud is used and is stopped after five
  minutes without use, so the 3.6 GB model is not held in memory otherwise.
- **Licence.** The S2 Pro weights are under the Fish Audio Research License: research and
  personal non-commercial use only. This is why they are never bundled with the installer.
- **Maturity.** s2.cpp describes itself as alpha software. The build script pins the commit
  this integration was written against.
- **Privacy.** Text is sent only to the sidecar on `127.0.0.1`. No voice cloning is used.

## Ollama

Install Ollama and make sure the local model you want is available. The default is a fast
small model, `llama3.2:3b` — pull it once:

```powershell
ollama pull llama3.2:3b
```

It is chosen for low latency: LocalFlow's deterministic layer already handles punctuation,
self-corrections, filler cleanup, and your replacements/snippets, so the LLM only does light
semantic cleanup and a small model is plenty (this mirrors the small Llama-family model Wispr
Flow fine-tunes for its cleanup). You can change the model in Settings > Models — good
alternatives are `qwen2.5:3b` (quality) or the original `gemma4:12b-it-qat` (heaviest, slowest)
— and native dictation uses your choice (it never silently switches to a remote model). To
skip the LLM entirely for the fastest possible insertion, enable low-resource mode; the
deterministically formatted text is inserted directly.

Check availability:

```powershell
.\scripts\Check-Ollama.ps1
```

The shared provider calls `http://127.0.0.1:11434/api/tags` for discovery and `http://127.0.0.1:11434/api/generate` for cleanup. Native dictation calls `http://127.0.0.1:11434/api/generate` with the configured model, `stream: false`, JSON-format output, a fixed `num_ctx` of 4096, and `num_predict` capped at 1024. The fixed context matters: without it Ollama sized the context from the model's maximum and, on the development machine, failed with an out-of-memory error. Model names with a cloud tag (`:cloud`, `-cloud`) are refused because they run on Ollama's servers.

Localhost communication with Ollama is allowed. Remote model fallback is not allowed. If the model is unavailable or returns invalid JSON twice, LocalFlow falls back to the deterministically formatted transcript so the user does not lose dictated text.
