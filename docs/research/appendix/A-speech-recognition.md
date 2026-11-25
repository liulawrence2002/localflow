# Local speech recognition for real-time dictation: state of the art and a recommendation for LocalFlow

> **Research snapshot dated 2026-11-22.** Kept as written; model choices, measurements and plans may have changed since, and it may not match the current code. For what was decided and built, see [VOICE_ASSISTANT_PLAN.md](../../VOICE_ASSISTANT_PLAN.md) and [development/PLAN.md](../../development/PLAN.md).

> Research appendix produced on 2026-11-22 by an independent research agent for the voice pipeline review. It is kept as written, apart from removing machine-specific paths. Line references to `native_dictation.rs` describe the code before the changes in `../VOICE_PIPELINE_REVIEW.md`. Figures are labelled by the author as vendor, third-party, reproduced, or unverified; treat unlabelled numbers as unverified.

Research date: 2026-11-22. All versions, dates, sizes and scores below were checked on that date against the primary source cited. Nothing in the LocalFlow repository was modified.

Evidence labels used throughout:

- **[R]** independently reproduced: either measured by me on the LocalFlow machine (Core Ultra 7 165H, CPU-only whisper.cpp 1.9.1 build already in `.localflow-runtime`), or published by the Hugging Face Open ASR Leaderboard, which runs open models with public evaluation scripts.
- **[3P]** third-party measurement that I did not reproduce (mostly the transcribe.cpp model catalog, which WER-tests every GGUF against the reference implementation).
- **[V]** vendor or author claim (model card, blog, docs).
- **[U]** unverified, or my own estimate. Estimates say so.

---

## 1. Recommendation

### What to replace `tiny.en-q5_1` cold-spawn with

|                                   | Model                                                  | Runtime                                                                                               | Expected latency, 5 s utterance                                                                                                                                                                                                                             | File                                 |
| --------------------------------- | ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------ |
| **(a) CPU-only default**          | NVIDIA **Parakeet Unified EN 0.6B**, Q8_0 GGUF, 731 MB | **`transcribe-cpp` Rust crate 0.4.2** (transcribe.cpp, MIT), loaded once in-process and kept warm     | about 0.4 to 1.0 s after end of speech **[U, estimate]**. Basis: 14 to 15x realtime on an older 8-core Ryzen 7 4750U **[3P]**; and 0.8 to 1.0 s encode+decode for the sibling Parakeet TDT v3 in the slower whisper.cpp 1.9.1 build on this machine **[R]** | `parakeet-unified-en-0.6b-Q8_0.gguf` |
| **(b) GPU option**                | Same model, same file                                  | Same crate with the `vulkan` feature (covers both the RTX 4070 and the Intel Arc iGPU with one build) | roughly 50 to 150 ms **[U, estimate]**. Basis: 25x realtime on a Renoir iGPU over Vulkan and 188x on M4 Max Metal **[3P]**; not measured on an RTX 4070                                                                                                     | same file                            |
| Interim, zero new code            | Whisper **base.en-q5_1**, 59.7 MB                      | Existing `whisper-cli.exe`, add `-bs 1 -bo 1 -ac 384` and VAD gating                                  | 0.42 to 0.80 s cold spawn **[R]**                                                                                                                                                                                                                           | `ggml-base.en-q5_1.bin`              |
| Interim, binaries already on disk | **Parakeet TDT 0.6B v3** q8_0 ggml, 669 MB             | Existing `parakeet-cli.exe` (whisper.cpp 1.9.1)                                                       | 1.9 to 2.1 s cold spawn, of which about 1.0 to 1.2 s is model load **[R]**                                                                                                                                                                                  | `ggml-parakeet-tdt-0.6b-v3-q8_0.bin` |

Why this choice:

1. **Accuracy.** Parakeet-class 0.6B transducers are in a different league from `tiny.en`. On LibriSpeech test-clean the transcribe.cpp catalog measures tiny.en at 5.7 to 6.0 % WER, base.en 4.1 to 4.3 %, small.en 3.0 to 3.1 %, and Parakeet Unified EN at 1.6 % **[3P]**. On FLEURS English: 10.72 / 7.60 / 6.14 / 3.99 % **[3P]**. On the current Open ASR Leaderboard, Parakeet TDT v2 and v3 score 6.93 and 7.04 average WER against 8.75 for Whisper large-v3-turbo and 8.24 for large-v3 **[R]** [1].
2. **Latency shape.** Whisper's encoder always processes a fixed 30 s window, so a 2 s utterance costs as much as a 30 s one unless `audio_ctx` is cut down. FastConformer encoders scale with audio length. Measured here: small.en on a 5 s clip takes 4.3 to 6.1 s with defaults **[R]**.
3. **Punctuation and capitalisation are native**, so the deterministic formatter has less to repair before the Ollama pass.
4. **No silence hallucination in my probes.** On 3 s of digital silence and 3 s of low noise, every Whisper size produced text ("you", "[BLANK_AUDIO]", "(buzzing)", "(crickets chirping)"); Parakeet produced an empty string both times **[R]**.
5. **Same stack as the closest comparable app.** Handy 0.10.1 (Tauri 2 + Rust, Windows/macOS/Linux) moved every model onto `transcribe-cpp` 0.4.2 and ships exactly `features = ["dynamic-backends", "vulkan"]` on Windows x64 [8]. VoiceInk also adopted transcribe.cpp [20]. Handy's README marks this exact file as "recommended" [9].
6. **One model serves both tiers.** GPU only changes the backend, not the model, so behaviour is identical with and without a GPU, and CPU fallback is trivial.

Choose **Parakeet TDT 0.6B v3** instead if either of these matters more than the last few tenths of English WER: it is **CC-BY-4.0** (Unified EN is under the NVIDIA Open Model License, which permits commercial use but is a custom licence), and it covers **25 European languages** with automatic language detection. Both are 0.6B and run at the same speed **[3P]**.

### Exact files

Recommended (transcribe.cpp GGUF):

| File                                                                   |       Bytes | URL                                                                                                                   |
| ---------------------------------------------------------------------- | ----------: | --------------------------------------------------------------------------------------------------------------------- |
| `parakeet-unified-en-0.6b-Q8_0.gguf`                                   | 731,357,568 | https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/main/parakeet-unified-en-0.6b-Q8_0.gguf   |
| `parakeet-unified-en-0.6b-Q5_K_M.gguf` (smaller, same measured WER)    | 540,795,264 | https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/main/parakeet-unified-en-0.6b-Q5_K_M.gguf |
| `parakeet-tdt-0.6b-v3-Q8_0.gguf` (CC-BY-4.0, multilingual alternative) | 739,508,576 | https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf/resolve/main/parakeet-tdt-0.6b-v3-Q8_0.gguf           |
| `silero-vad-v6.2-F32.gguf` (VAD for transcribe.cpp)                    |   1,241,056 | https://huggingface.co/handy-computer/silero-vad-v6.2-gguf/resolve/main/silero-vad-v6.2-F32.gguf                      |

Interim (works with the whisper.cpp binaries already in `.localflow-runtime`):

| File                                           |       Bytes | URL                                                                                           |
| ---------------------------------------------- | ----------: | --------------------------------------------------------------------------------------------- |
| `ggml-base.en-q5_1.bin`                        |  59,721,011 | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en-q5_1.bin               |
| `ggml-small.en-q5_1.bin`                       | 190,098,681 | https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.en-q5_1.bin              |
| `ggml-parakeet-tdt-0.6b-v3-q8_0.bin`           | 668,757,119 | https://huggingface.co/ggml-org/parakeet-GGUF/resolve/main/ggml-parakeet-tdt-0.6b-v3-q8_0.bin |
| `ggml-silero-v6.2.0.bin` (VAD for `--vad -vm`) |     885,098 | https://huggingface.co/ggml-org/whisper-vad/resolve/main/ggml-silero-v6.2.0.bin               |

Seven of these eight URLs returned HTTP 200 with the listed Content-Length on 2026-11-22; the Q5_K_M and TDT v3 GGUF sizes come from the Hugging Face file listing rather than a direct request, and the four interim files were actually downloaded and run.

### Things to verify before committing

- **CPU and GPU latency of transcribe.cpp on this machine was not measured.** I did not download and run third-party executables. The prebuilt `transcribe-native-0.4.2-windows-x86_64-cpu-vulkan.tar.gz` (18 MB) from the v0.4.2 release makes this a short check with `transcribe-cli`.
- **Laptop dGPU wake-up.** A sleeping Optimus dGPU may add first-inference latency **[U]**. Warm the session on key-down and keep CPU as a fallback.
- **Build cost.** `transcribe-cpp-sys` compiles the native library from source with CMake and MSVC [10]. Both are already Tauri prerequisites, but the Vulkan feature needs the Vulkan SDK at build time [11].
- **Maturity.** The crate's README still says "Status: in development". Handy isolates the backend in its own worker and replaces it after a backend error; LocalFlow should do the same.
- **Licence text.** Read the NVIDIA Open Model License before shipping Unified EN as a bundled default; the model card states it is "ready for commercial/non-commercial use" **[V]** [5].
- **Privacy.** The runtime does no networking **[U, not audited]**. Keep model download an explicit, user-initiated step (Handy uses `hf-hub` for this; LocalFlow should not adopt silent downloads).

---

## 2. Local measurements on the LocalFlow machine [R]

Setup: Core Ultra 7 165H, whisper.cpp **1.9.1** CPU build from `.localflow-runtime` (backend `ggml-cpu-alderlake.dll`), 8 threads, one process spawned per utterance, model files in the OS file cache. Audio: whisper.cpp `samples/jfk.wav` (11.0 s) and clips cut from it. Wall time includes process start and model load. The machine was under variable load, so ranges are wide and these are indicative, not a rigorous benchmark. The sample is one speaker and a handful of clips; transcripts are anecdotes, not WER.

### 5 s utterance ("And so my fellow Americans, ask not")

| Model and flags                                  |                                                               Wall time, cold spawn | Transcript                                                          |
| ------------------------------------------------ | ----------------------------------------------------------------------------------: | ------------------------------------------------------------------- |
| tiny.en-q5_1, defaults (current LocalFlow)       |                                                                      0.53 to 1.03 s | `And so my fellow Americans asked not` (wrong word, no punctuation) |
| tiny.en-q5_1, `-bs 1 -bo 1 -ac 384`              |                                                                      0.38 to 0.62 s | `And so my fellow Americans ask not...`                             |
| base.en-q5_1, defaults                           |                                                                      0.82 to 1.91 s | `And so my fellow Americans ask not`                                |
| base.en-q5_1, `-bs 1 -bo 1 -ac 384`              |                                                                      0.42 to 0.80 s | `And so my fellow Americans asked not.` (wrong word)                |
| small.en-q5_1, defaults                          |                                                                        4.3 to 6.1 s | not captured                                                        |
| small.en-q5_1, `-bs 1 -bo 1 -ac 384`             |                                                                      1.17 to 2.04 s | `And so my fellow Americans, ask not.`                              |
| large-v3-turbo-q5_0, defaults                    |                                                                              35.6 s | not captured                                                        |
| large-v3-turbo-q5_0, `-bs 1 -bo 1 -ac 384`       |                                                                              11.5 s | not captured                                                        |
| Parakeet TDT 0.6B v3 q8_0 via `parakeet-cli.exe` | 1.9 to 2.1 s (an earlier batch gave 2.2 to 3.2 s with one 7.7 s outlier under load) | `And so, my fellow Americans, ask not!`                             |

Parakeet's internal timers: load 1.0 to 1.2 s, encode 0.81 to 1.6 s, decode and predict about 20 ms. So about half of the cold-spawn wall time is model load, which a persistent process removes. Thread count barely mattered between 4 and 12 threads (encode 0.81 to 0.96 s); 16 was slower.

Conclusions:

- **Whisper large-v3-turbo is not usable on this CPU** in whisper.cpp (35 s for a 5 s clip).
- **`audio_ctx` is the single biggest lever for Whisper on short audio**: 2 to 4x faster. It is a non-standard truncation of the encoder window and has a failure mode, shown below.
- **Parakeet in whisper.cpp 1.9.1 works on CPU out of the box** with the binaries already on disk, with the best transcript of the set, but cold-spawn load cost makes it slower end to end than tiny.en.

### 11 s utterance

| Model and flags                |    Wall time | Transcript quality           |
| ------------------------------ | -----------: | ---------------------------- |
| tiny.en defaults               |        1.6 s | correct words, missing comma |
| base.en defaults               |        2.4 s | correct                      |
| small.en `-bs 1 -bo 1 -ac 768` |        3.4 s | correct                      |
| Parakeet v3 q8_0               | 2.6 to 3.2 s | correct, best punctuation    |

### `whisper-server.exe` persistent mode

base.en-q5_1, `-bs 1 -bo 1`, bound to 127.0.0.1, multipart POST to `/inference`: **0.91 to 1.05 s** for the 5 s clip over four requests and 1.17 to 1.33 s for the 11 s clip over two. That is no faster than a cold `whisper-cli` spawn with `-ac 384`, because for tiny and base the model load is only 70 to 150 ms; the encoder's 30 s window dominates. Persistent mode pays off for models with a long load (Parakeet about 1 s, turbo 1 to 1.8 s) and on GPU, not for tiny or base on CPU. In 1.9.1 there is no server binary for Parakeet, only `parakeet-cli.exe`.

### Silence, noise and very short audio

| Input                                   | tiny.en                         | base.en                       | small.en (`-ac 384`)          | tiny.en + `--vad` (Silero v6.2.0) | tiny.en + `-sns` | Parakeet v3                    |
| --------------------------------------- | ------------------------------- | ----------------------------- | ----------------------------- | --------------------------------- | ---------------- | ------------------------------ |
| 3 s digital silence                     | `you`                           | `you`                         | `you`                         | _(empty)_                         | `you`            | _(empty)_                      |
| 3 s low Gaussian noise                  | `[BLANK_AUDIO]`                 | `(buzzing)`                   | `(crickets chirping)`         | _(empty)_                         | `You`            | _(empty)_                      |
| 2 s speech "And so my fellow Americans" | `And so am I fellow Americans.` | `and so my fellow Americans.` | `And so my fellow Americans.` | same as tiny                      | same as tiny     | `And so, my fellow Americans.` |

The same 2 s clip through tiny.en with `-ac 256` gave `and so my fellow Americans and so my fellow Americans` (a repetition loop).

So on this build: VAD gating removed the hallucination; `--suppress-nst` did not (it turned a bracketed marker into "You"); and too small an `audio_ctx` caused repetition.

---

## 3. Models

### 3.1 Why leaderboard numbers changed in 2026

The Hugging Face Open ASR Leaderboard is at version **1.16.2 (2026-11-20)** [1]. Its default English average is now taken over **AMI-Cleaned, Earnings22-Cleaned-AA-chunked, Gigaspeech-Cleaned, SPGISpeech, URGENT24, Voice Arena Monsoon and Voxpopuli-AA-Cleaned**; private scripted and conversational sets are reported alongside. LibriSpeech and TED-LIUM are no longer in the default average. Averages quoted on model cards ("Open ASR avg 6.05" and similar) use the older eight-set mix and are **not comparable** to the table below. The same update removed `distil-whisper/distil-large-v3.5`, and the 2026-11-18 update removed several open models above 10 % average WER. Whisper tiny/base/small/medium, Moonshine and Kyutai STT are absent from the current table.

The leaderboard's RTFx is GPU batch throughput on a datacentre card. It says nothing about single-utterance CPU latency.

### 3.2 Current Open ASR Leaderboard, English short-form, selected rows [R]

Source: leaderboard Space data, version 1.16.2, read 2026-11-22 [1]. 68 rows in total; lower WER is better.

|   Rank | Model                                                     |    Avg WER |     RTFx | Open weights | Local on 8 GB        |
| -----: | --------------------------------------------------------- | ---------: | -------: | ------------ | -------------------- |
|      1 | zoom/scribe_v2.1_pro                                      |       4.87 |      n/a | no           | no (cloud)           |
|      2 | OtterAI/speech-v3.1                                       |       5.21 |      n/a | no           | no                   |
|      4 | microsoft/azure-speech-09-2026                            |       5.50 |      n/a | no           | no                   |
|      6 | Musci-research/Musci-ASR-qwen-1019-hf                     |       5.70 |      775 | yes          | [U]                  |
|      8 | elevenlabs/scribe_v2                                      |       5.93 |      n/a | no           | no                   |
|     13 | FluidVoice-Labs/fluid-speech-pro                          |       6.06 |     3690 | yes          | [U]                  |
| 15, 16 | nvidia/NVIDIA-Nemotron-3.5-Transcribe-30B-A4B (FP8, base) | 6.12, 6.13 | 418, 473 | yes          | no (30B)             |
|     17 | Qwen/Qwen3-ASR-1.7B-hf                                    |       6.15 |      836 | yes          | yes                  |
|     18 | assemblyai/universal-3-5-pro                              |       6.21 |      n/a | no           | no                   |
|     22 | ibm-granite/granite-speech-5.0-2b                         |       6.68 |      548 | yes          | yes                  |
|     23 | CohereLabs/cohere-transcribe-03-2026                      |       6.72 |      924 | yes          | yes                  |
|     25 | TheStageAI/thewhisper-large-v3-turbo                      |       6.74 |     1184 | yes          | yes                  |
|     27 | openai/gpt-transcribe                                     |       6.80 |      n/a | no           | no                   |
|     32 | nvidia/canary-qwen-2.5b                                   |       6.92 |      882 | yes          | yes                  |
|     34 | **nvidia/parakeet-tdt-0.6b-v2**                           |   **6.93** |     6148 | yes          | yes, CPU-viable      |
|     37 | ibm-granite/granite-speech-5.0-470m-turboctc-nc           |       6.98 |    21082 | yes          | yes, CPU-viable      |
|     40 | **nvidia/parakeet-tdt-0.6b-v3**                           |   **7.04** |     6322 | yes          | yes, CPU-viable      |
|     43 | Qwen/Qwen3-ASR-0.6B-hf                                    |       7.07 |      750 | yes          | yes                  |
|     46 | ibm-granite/granite-speech-5.0-470m-turboctc              |       7.41 |    21435 | yes          | yes, CPU-viable      |
|     47 | mistralai/Voxtral-Small-24B-2507                          |       7.44 |      101 | yes          | no                   |
|     48 | nvidia/nemotron-speech-streaming-en-0.6b                  |       7.47 |     1116 | yes          | yes, CPU-viable      |
|     51 | openai/whisper-large-v3                                   |       8.24 |      471 | yes          | GPU only in practice |
|     52 | mistralai/Voxtral-Mini-3B-2507                            |       8.32 |      180 | yes          | GPU                  |
|     53 | nvidia/canary-1b-v2                                       |       8.43 |     1854 | yes          | yes                  |
|     56 | openai/whisper-large-v3-turbo                             |       8.75 |      809 | yes          | GPU only in practice |
|     61 | mistralai/Voxtral-Mini-4B-Realtime-2602                   |       9.72 |      103 | yes          | no (16 GB)           |
|     66 | nvidia/nemotron-3.5-asr-streaming-0.6b                    |      11.09 |     1353 | yes          | yes                  |

`parakeet-unified-en-0.6b` is not on the leaderboard. NVIDIA's own card puts it at 5.91 average on the legacy eight-set mix, against 6.04 for Parakeet TDT v2 and 6.92 for Nemotron Speech Streaming EN in the same table **[V]** [5]. Reading across: the best cloud systems are about two WER points ahead of Parakeet-class local models on this benchmark, and the heaviest local-capable open models (Qwen3-ASR-1.7B) are about 0.8 points ahead.

### 3.3 Candidate models for LocalFlow

Column key: LS is LibriSpeech test-clean WER and FL is FLEURS English WER, both as measured by the transcribe.cpp catalog on its GGUF ports (Q8_0 unless noted) **[3P]** [7]. "CPU x" and "iGPU x" are that catalog's speed-up over realtime on a Ryzen 7 PRO 4750U (CPU, and Vulkan on its integrated Radeon) **[3P]**; a 165H should be faster **[U]**. Leaderboard is the current Open ASR average **[R]**. PnC means native punctuation and capitalisation.

| Model                             |     Params | Licence                                           | Languages                            | PnC                                  |       LS |       FL |                                                                              Leaderboard |    CPU x |   iGPU x |           Q8_0 size | Notes                                                                         |
| --------------------------------- | ---------: | ------------------------------------------------- | ------------------------------------ | ------------------------------------ | -------: | -------: | ---------------------------------------------------------------------------------------: | -------: | -------: | ------------------: | ----------------------------------------------------------------------------- |
| Whisper tiny.en                   |        39M | Apache-2.0 on the GGUF card; OpenAI's repo is MIT | en                                   | yes                                  |     5.72 |    10.72 |                                                                                   absent |       41 |       47 | 43.6 MB (ggml q8_0) | current LocalFlow model, at q5_1                                              |
| Whisper base.en                   |        74M | same                                              | en                                   | yes                                  |     4.16 |     7.60 |                                                                                   absent |       21 |       32 |               85 MB |                                                                               |
| Whisper small.en                  |       244M | same                                              | en                                   | yes                                  |     3.09 |     6.14 |                                                                                   absent |      6.3 |     12.5 |              270 MB |                                                                               |
| Whisper large-v3-turbo            |       809M | MIT                                               | 99                                   | yes                                  |     2.01 |     4.38 |                                                                                     8.75 |      1.4 |      3.1 |              886 MB | too slow on CPU [R]                                                           |
| Whisper large-v3                  |      1.55B | MIT                                               | 99                                   | yes                                  |      n/a |      n/a |                                                                                     8.24 |      n/a |      n/a |                 n/a | 3.1 GB as full ggml                                                           |
| distil-large-v3.5                 |       756M | MIT                                               | en                                   | yes                                  | 2.37 [V] |      n/a |                                                                       removed 2026-11-20 |      n/a |      n/a |                 n/a | about 1.5x turbo speed [V] [15]                                               |
| **Parakeet Unified EN 0.6B**      |       600M | NVIDIA Open Model License                         | en                                   | yes                                  | **1.60** | **3.99** |                                                                                   absent | **14.0** | **25.3** |              731 MB | offline and buffered streaming from one model; released 2026-04-07 [5]        |
| Parakeet TDT 0.6B v2              |       600M | CC-BY-4.0                                         | en                                   | yes                                  |     1.69 |     4.11 |                                                                                     6.93 |     14.3 |     27.4 |              730 MB |                                                                               |
| **Parakeet TDT 0.6B v3**          |       600M | CC-BY-4.0                                         | 25 European, auto-detect             | yes                                  |     1.94 |     4.83 |                                                                                     7.04 |     13.7 |     25.1 |              740 MB | supported by the whisper.cpp build already on disk                            |
| moondream/parakeet-ultra          |       600M | CC-BY-4.0                                         | 25                                   | yes                                  |     1.80 |     4.26 |                                                                                   absent |     13.3 |     23.0 |                 n/a | fine-tune of TDT v3; claims 5.80 against 6.26 on seven English sets [V] [16]  |
| Parakeet TDT-CTC 110M             |       110M | CC-BY-4.0                                         | en                                   | [U]                                  |     2.43 |     6.11 |                                                                                   absent |       50 |       76 |                 n/a | very fast, small                                                              |
| Canary 1B v2                      |       978M | CC-BY-4.0                                         | 25, plus translation                 | yes                                  |     1.91 |     4.47 |                                                                                     8.43 |      8.3 |     14.1 |                 n/a |                                                                               |
| Canary 180M Flash                 |       180M | CC-BY-4.0                                         | en, de, es, fr                       | yes                                  |     1.93 |     5.98 |                                                                                   absent |     21.7 |     33.1 |                 n/a |                                                                               |
| Granite Speech 5.0 TurboCTC 470M  |       470M | Apache-2.0                                        | 7 (en, fr, de, es, pt, it, ja)       | yes, since the 2026-10-21 update [V] |     1.33 |     4.61 | 7.41 (6.98 for the `-nc` variant); the entry is still the older English-only release [V] |     17.2 |     22.9 |              506 MB | non-autoregressive CTC [17]                                                   |
| Granite Speech 5.0 2B             |         2B | [U]                                               | multi                                | yes                                  |      n/a |      n/a |                                                                                     6.68 |      n/a |      n/a |                 n/a |                                                                               |
| Moonshine base (v1)               |        61M | MIT                                               | en                                   | yes                                  |     3.26 |    12.25 |                                                                                   absent |     23.5 |     35.6 |                 n/a |                                                                               |
| Moonshine Streaming medium (v2)   |       245M | MIT                                               | en                                   | yes                                  |     2.16 |     7.87 |                                                   absent; 6.65 on the legacy average [V] |      9.7 |     14.3 |              296 MB | sliding-window encoder, autoregressive decoder [18]                           |
| Moonshine Streaming small (v2)    |       123M | MIT                                               | en                                   | yes                                  |     2.54 |     8.55 |                                                                  absent; 7.84 legacy [V] |     13.5 |     21.8 |                 n/a |                                                                               |
| Nemotron Speech Streaming EN 0.6B |       600M | NVIDIA Open Model License                         | en                                   | yes                                  |     2.31 |     6.43 |                                                                                     7.47 |     13.6 |     27.1 |              730 MB | cache-aware streaming, 80 ms to 1.12 s chunks [V]                             |
| Nemotron 3.5 ASR Streaming 0.6B   |       600M | OpenMDW-1.1                                       | 40 locales                           | yes                                  |     3.05 |     7.90 |                                                                                    11.09 |     10.6 |     17.1 |              751 MB | released 2026-06-04 [V]                                                       |
| Qwen3-ASR 0.6B                    |       0.6B | Apache-2.0                                        | 30 languages and 22 Chinese dialects | yes                                  |     2.11 |     4.23 |                                                                                     7.07 |      5.3 |      9.7 |                 n/a | LLM-style decoder                                                             |
| Qwen3-ASR 1.7B                    |       1.7B | Apache-2.0                                        | same                                 | yes                                  |      n/a |      n/a |                                                                                     6.15 |      n/a |      n/a |             2.19 GB | best open model that fits 8 GB                                                |
| Cohere Transcribe 03-2026         |        [U] | Apache-2.0                                        | 14                                   | yes                                  |     1.27 |     5.08 |                                                                                     6.72 |      4.3 |      8.1 |             2.41 GB |                                                                               |
| Voxtral Mini 4B Realtime 2602     |         4B | Apache-2.0                                        | 13                                   | yes                                  |     2.07 |     6.27 |                                                                                     9.72 | **0.58** |     0.76 |                 n/a | needs a GPU with at least 16 GB in BF16 [V]; slower than realtime on CPU [3P] |
| Voxtral Mini 3B 2507              |         3B | [U]                                               | multi                                | yes                                  |      n/a |      n/a |                                                                                     8.32 |      n/a |      n/a |                 n/a |                                                                               |
| Kyutai STT 1B en_fr               |   about 1B | CC-BY-4.0                                         | en, fr                               | yes                                  |      n/a |      n/a |                                                                                   absent |      n/a |      n/a |                 n/a | 0.5 s delay, semantic VAD [V] [19]                                            |
| Kyutai STT 2.6B en                | about 2.6B | CC-BY-4.0                                         | en                                   | yes                                  |      n/a |      n/a |                                                                                   absent |      n/a |      n/a |                 n/a | 2.5 s delay [V]; no transcribe.cpp port                                       |
| SenseVoice Small                  |        [U] | FunASR model licence (custom)                     | zh, yue, en, ja, ko                  | [U]                                  |     3.13 |     7.14 |                                                                                   absent |     19.7 |     27.7 |                 n/a |                                                                               |

Answers by tier:

- **CPU, English dictation:** Parakeet Unified EN, or TDT v2/v3 for a standard licence. Runner-up worth a bake-off: **Granite Speech 5.0 TurboCTC 470M** (Apache-2.0, smaller at 506 MB, faster, lowest LibriSpeech WER in the catalog), but its punctuation and multilingual update is one month old and the leaderboard has not re-scored it. Low-RAM fallback: Whisper base.en, or Parakeet TDT-CTC 110M.
- **8 GB GPU:** the same Parakeet model is already near-instant. If maximum accuracy matters more than simplicity, Qwen3-ASR-1.7B (Q8_0 2.19 GB) is the strongest open model that fits. Whisper large-v3-turbo remains the choice for 99-language coverage and for vocabulary biasing through an initial prompt.
- **Released in 2026 and relevant:** Parakeet Unified EN (April), Nemotron 3.5 ASR Streaming (June), Moonshine v2 streaming (paper in February), Voxtral Realtime (2602), Cohere Transcribe (03-2026), Granite Speech 5.0 TurboCTC (August, multilingual update in October), moondream parakeet-ultra. Release dates for the last two Voxtral and Cohere items are inferred from their names **[U]**.

---

## 4. Runtimes for Windows and Rust

| Runtime                                           | Version (date)                                                                                | Licence    | Windows status                                                                                                                                                                                                                                                                           | Relevance                                                                                                                                                                                                                                                                                         |
| ------------------------------------------------- | --------------------------------------------------------------------------------------------- | ---------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **transcribe.cpp** and the `transcribe-cpp` crate | v0.4.2 (2026-11-12) [10]                                                                      | MIT        | Prebuilt `windows-x86_64-cpu-vulkan` (18 MB) and `windows-x86_64-cuda` (203 MB) archives; the crate builds from source with CMake                                                                                                                                                        | **Recommended.** ggml-based; 16 model families and 60+ variants in GGUF; Vulkan, CUDA, Metal, ROCm; in-process sessions, a streaming API, built-in Silero VAD v6.2, vocabulary and prompt options, memory-mapped weights. Crate features: `vulkan`, `cuda`, `dynamic-backends`, `openmp`, `serde` |
| **whisper.cpp**                                   | v1.9.5 (2026-10-06); nightly b5907 (2026-11-20). The local copy is **1.9.1** (2026-06-19) [2] | MIT        | Prebuilt assets live on the `bNNNN` build tags: `whisper-bin-x64.zip` (9.2 MB, CPU), `whisper-blas-bin-x64.zip` (22 MB), `whisper-bin-win-cuda-12.4.0-x64.zip` (736 MB), `whisper-bin-win-cuda-11.8.0-x64.zip` (766 MB), plus Win32 and ARM64 builds. **No prebuilt Vulkan or OpenVINO** | Fine for Whisper. See below for Parakeet and VAD                                                                                                                                                                                                                                                  |
| whisper-rs                                        | 0.16.0 (2026-03-12), hosted on Codeberg                                                       | [U]        | builds whisper.cpp from source                                                                                                                                                                                                                                                           | In-process Whisper only                                                                                                                                                                                                                                                                           |
| faster-whisper and CTranslate2                    | 1.2.1 (2025-10-31) and 4.9.1 (2026-11-22)                                                     | MIT        | Python; `ct2rs` 0.11.2 gives Rust bindings                                                                                                                                                                                                                                               | Good for batch Whisper; adds a Python or CTranslate2 dependency with no gain for Parakeet                                                                                                                                                                                                         |
| sherpa-onnx                                       | v2.0.3 (2026-11-22)                                                                           | Apache-2.0 | Official safe Rust crate `sherpa-onnx` 2.0.3; large ONNX model zoo, for example `sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2` (487 MB)                                                                                                                                            | Credible alternative: ONNX Runtime on CPU with int8 models, streaming zipformers, Silero and TEN VAD. `sherpa-rs` (0.6.8, 2025-10) is the older community binding                                                                                                                                 |
| transcribe-rs                                     | 0.3.12 (2026-10-02)                                                                           | [U]        | Rust wrapper over ONNX and whisper engines                                                                                                                                                                                                                                               | Handy's previous engine layer; Handy has since moved to `transcribe-cpp`                                                                                                                                                                                                                          |
| parakeet-rs                                       | 0.4.4 (2026-10-31)                                                                            | MIT        | ONNX via `ort`                                                                                                                                                                                                                                                                           | Parakeet, streaming and diarization in Rust                                                                                                                                                                                                                                                       |
| ONNX Runtime with DirectML                        | `ort` 2.0.0-rc.15 wraps ONNX Runtime 1.31                                                     | MIT        | Microsoft now lists DirectML as "legacy"; new work goes to Windows ML [21]                                                                                                                                                                                                               | Not a path to start building on in 2026                                                                                                                                                                                                                                                           |
| parakeet.cpp                                      | mudler/parakeet.cpp v0.6.1 (2026-10-07, ggml); frikallo/parakeet.cpp (Apple MPS focus)        | MIT        | [U]                                                                                                                                                                                                                                                                                      | Superseded for this purpose by transcribe.cpp                                                                                                                                                                                                                                                     |

### whisper.cpp details

- **Parakeet support** arrived in the 1.9 line. The README covers one model, `nvidia/parakeet-tdt-0.6b-v3`, with ggml files in `ggml-org/parakeet-GGUF`: f32 2.51 GB, f16 1.26 GB, q8_0 669 MB, q4_k 416 MB, q4_0 356 MB [3]. The CLI is minimal: `-t`, `-m`, `-f`, `-ng`, `-dev`, `-ps` (token timestamps), `-otxt`, `-of`, `-np`. There is **no JSON output, no VAD flag, no prompt, and no server**, so LocalFlow's JSON parser would need a plain-text path.
- **Fixes since 1.9.1 that matter for dictation:** VAD min-speech and min-silence flag parsing fixed (1.9.2); a heap out-of-bounds read in the mel spectrogram on very short audio fixed, and invalid tensor headers rejected for Whisper and Parakeet (1.9.3); Parakeet TDT decode fixed (1.9.4) [2]. Updating the runtime to the b5454 build (v1.9.5) or newer is worthwhile whatever else changes.
- **`whisper-server.exe`**: multipart `/inference`, and `/load` to hot-swap models; the same decode and VAD flags as the CLI; binds 127.0.0.1:8080 by default. Whisper models only.
- **Built-in Silero VAD**: `--vad -vm <model>`, with `-vt` threshold 0.50, `-vspd` min speech 250 ms, `-vsd` min silence 100 ms, `-vmsd` max speech, `-vp` pad 30 ms, `-vo` overlap 0.10 s. Models `ggml-silero-v5.1.2.bin` and `ggml-silero-v6.2.0.bin` (885 KB each) [4]. Present in the local 1.9.1 binaries and working **[R]**.
- **GPU builds from source**: `-DGGML_CUDA=1`, `-DGGML_VULKAN=1`, `-DWHISPER_OPENVINO=1` (encoder only, on an Intel CPU or Intel GPU; OpenVINO 2026.3.0 recommended; needs a converted encoder file per model) [2].

### What other dictation apps run

| App                             | Platform                                | Stack                                                                                                                                                                                                                                                                                                                                                                                                                                    | Evidence                                               |
| ------------------------------- | --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| **Handy** 0.10.1 (2026-11-14)   | Windows, macOS, Linux; Tauri 2 and Rust | `transcribe-cpp` 0.4.2 for every model: Vulkan on Windows x64, static CPU on Windows ARM, Metal on Apple Silicon. Silero VAD through transcribe.cpp plus an experimental `earshot` pure-Rust VAD; `rubato` for resampling; `handy-recorder`; `enigo` and `rdev` for input. The transcription backend runs in its own worker. Models: Whisper small, medium-q4_1, turbo, large-v3-q5_0, and Parakeet Unified EN Q8_0 marked "recommended" | `Cargo.toml` and README read directly [8][9]           |
| **VoiceInk** v2.27 (2026-11-11) | macOS, GPL-3.0                          | whisper.cpp for Whisper, FluidAudio for Parakeet on CoreML, and transcribe.cpp through a Swift package for GGUF models                                                                                                                                                                                                                                                                                                                   | README [20]                                            |
| Superwhisper                    | macOS, closed                           | Whisper locally through whisper.cpp; Parakeet and cloud models on the paid tier                                                                                                                                                                                                                                                                                                                                                          | vendor docs seen only through a search summary **[V]** |
| MacWhisper                      | macOS, closed                           | whisper.cpp and/or WhisperKit                                                                                                                                                                                                                                                                                                                                                                                                            | **[U]**; sources disagree                              |
| **Wispr Flow**                  | cloud only                              | Context-conditioned cloud ASR plus a personalised formatting LLM. Budget: under 200 ms ASR, under 200 ms LLM, 200 ms network, target 700 ms from end of speech                                                                                                                                                                                                                                                                           | Wispr engineering post, 2025-09-11 **[V]** [22]        |

---

## 5. Quantisation in whisper.cpp

The published prequantised ggml files use `q5_0`, `q5_1` and `q8_0` [6]; the `quantize` tool also accepts 4-bit types (the README example uses `q5_0`; the full type list is **[U]**, not read from source).

| Model          | full f16 |          q8_0 |          q5_1 |          q5_0 |
| -------------- | -------: | ------------: | ------------: | ------------: |
| tiny.en        |  77.7 MB |       43.6 MB |       32.2 MB | not published |
| base.en        |   148 MB |       81.8 MB |       59.7 MB | not published |
| small.en       |   488 MB |        264 MB |        190 MB | not published |
| medium.en      |  1.53 GB |        823 MB | not published |        539 MB |
| large-v3-turbo |  1.62 GB |        874 MB | not published |        574 MB |
| large-v3       |  3.10 GB | not published | not published |       1.08 GB |

Measured accuracy impact. The best controlled data I found is the transcribe.cpp catalog, which scores every quant on the full 2,620-utterance LibriSpeech test-clean split **[3P]** [7]. These are GGUF K-quants, not whisper.cpp's `q5_1`, but the bit depths are comparable:

| Model                  |  F16 | Q8_0 | Q6_K | Q5_K_M | Q4_K_M |
| ---------------------- | ---: | ---: | ---: | -----: | -----: |
| Whisper tiny.en        | 5.78 | 5.72 | 5.83 |   5.91 |   5.96 |
| Whisper base.en        | 4.13 | 4.16 | 4.15 |   4.16 |   4.29 |
| Whisper small.en       | 2.97 | 3.09 | 2.97 |   3.11 |   3.09 |
| Whisper large-v3-turbo | 2.01 | 2.01 | 2.01 |   2.03 |   2.04 |
| Parakeet Unified EN    | 1.59 | 1.60 | 1.61 |   1.58 |   1.62 |
| Parakeet TDT v3        | 1.95 | 1.94 | 1.93 |   1.92 |   1.98 |

Takeaways:

- **Quantising down to 5 bits costs at most about 0.2 WER points, and on Parakeet nothing measurable.** Moving `tiny.en` from q5_1 to q8_0 would buy roughly 0.2 points. Moving from tiny to base buys about 1.6; to Parakeet, about 4.
- **Speed.** Quantisation mainly cuts disk and RAM. One paper found INT8 about 15 % faster than float for whisper.cpp base, with INT5 and INT4 no faster, but on only ten files [12], which is weak evidence. A community report on a 2010-era CPU found 5-bit formats several times slower than q4_0 [13], which is anecdotal and on old hardware. I did not measure quant-against-quant speed here.
- **Practical rule:** q5_1 for tiny, base and small is fine; q8_0 only if a few tenths of a point matter and RAM is free. For Parakeet GGUF, Q8_0 is the safe default and Q5_K_M (541 MB) is a 190 MB saving at no measured cost.

---

## 6. VAD and endpointing

| VAD                  | Version                                                 | Licence                                                                                                                                           | Frame                        | Notes                                                                                                                                                                                                                                                                          |
| -------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Silero VAD**       | package v6.2.3 (2026-09-23); v6.2 model 2025-11-06 [14] | MIT                                                                                                                                               | 512 samples, 32 ms at 16 kHz | Reference defaults: threshold 0.5, `neg_threshold` = threshold minus 0.15, min speech 250 ms, min silence 100 ms, speech pad 30 ms. v6.2 improved unusual, child, muted and phone-quality voices **[V]**. Available as ggml for whisper.cpp, GGUF for transcribe.cpp, and ONNX |
| TEN VAD              | 2025 releases                                           | Apache-2.0 **with added conditions**: it may not be deployed in ways that compete with Agora, and derivative works stay under the same terms [23] | 10 or 16 ms hops             | Claims faster speech-to-silence transitions and lower compute than Silero **[V]**; Rust crate `ten-vad-rs` 0.1.7. The licence conditions make it a poor default for an open local app                                                                                          |
| WebRTC VAD           | legacy                                                  | BSD [U]                                                                                                                                           | 10, 20 or 30 ms              | GMM-based and weak in noise **[U]**; only worth it as a near-free pre-filter                                                                                                                                                                                                   |
| earshot              | 1.2.2 (2026-08)                                         | [U]                                                                                                                                               | 16 ms                        | Pure Rust; Handy ships it as an experimental alternative at threshold 0.5                                                                                                                                                                                                      |
| whisper.cpp built-in | Silero v5.1.2 or v6.2.0 ggml                            | MIT                                                                                                                                               |                              | Runs before Whisper and passes only speech segments; maps timestamps back to the original audio since 1.9.2                                                                                                                                                                    |

### Recommended settings for push-to-talk dictation

For push-to-talk, key release is the endpoint. VAD has three smaller jobs: decide whether the recording contains any speech, trim dead air, and (in a hands-free mode) stop automatically.

Handy's production values, read from source, are a good starting point [8]:

| Parameter                                    | Handy value | Silero reference default |
| -------------------------------------------- | ----------: | -----------------------: |
| Silero speech threshold                      |     **0.3** |                      0.5 |
| Pre-roll (audio kept before speech onset)    |  **450 ms** |                30 ms pad |
| Onset (consecutive speech needed to trigger) |   **60 ms** |                     none |
| Hangover, offline or push-to-talk            |  **450 ms** |       100 ms min silence |
| Hangover, streaming or hands-free            | **1650 ms** |                     none |

Suggested LocalFlow policy (my judgement, informed by the above):

1. **Gate, do not carve.** Run Silero over the whole utterance. If there is no speech segment, skip ASR and paste nothing. If there is speech, pass the audio from 300 to 450 ms before the first speech to 300 to 450 ms after the last, in one piece, rather than deleting internal pauses. A November 2026 dictation study found that putting Silero VAD in front of Whisper raised whispered-speech WER from 10.1 % to 51.4 %, because quiet speech was discarded as silence [24]. A low threshold (0.3) and generous padding guard against that.
2. **Keep capturing 150 to 300 ms after key release** so the last word is not clipped. Handy fixed exactly this in 0.9.7 ("stop losing tail audio when a recording ends").
3. **Replace the RMS detector with Silero for auto-stop.** RMS cannot tell speech from a fan or a keyboard. For hands-free mode use 700 to 1000 ms of trailing silence for short commands and up to about 1.6 s for prose dictation.
4. Reject recordings with under about 250 ms of detected speech.

### Semantic endpointing and turn detection

This only matters for a hands-free mode, where a fixed silence timer cuts people off mid-thought.

| Model                        | Input                                              | Size and speed                                                                                                                                            | Licence                      | Fit                                                                                    |
| ---------------------------- | -------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------- | -------------------------------------------------------------------------------------- |
| **Pipecat Smart Turn v3.2**  | raw audio, the last 8 s of the turn, 16 kHz        | Whisper-tiny backbone; 8 MB int8 ONNX for CPU, 32 MB fp32; "as little as 10 ms on some CPUs", about 65 ms on a small cloud instance **[V]**; 23 languages | BSD-2-Clause                 | Best fit: audio-native, tiny, permissive. Run it only when Silero reports silence [25] |
| LiveKit turn detector        | **transcribed text**, up to 6 turns and 128 tokens | Distilled from Qwen2.5-0.5B, int8 ONNX; about 400 MB RAM and about 25 ms per inference **[V]**; 14 languages                                              | custom LiveKit model licence | Needs a transcript first, and the licence is not a standard open one [26]              |
| Kyutai STT 1B "semantic VAD" | built into the ASR model                           | about 1B                                                                                                                                                  | CC-BY-4.0                    | Ties you to that model **[V]**                                                         |
| Deepgram Flux                | built into cloud ASR                               | about 260 ms end-of-turn **[V]**                                                                                                                          | cloud                        | Not usable here                                                                        |

---

## 7. Streaming and partial transcripts

| Approach                                                                                                      | How it works                                                                                                                                                                                                                                                 | Realistic on this CPU?                                                                                                                                                                                                                                                |
| ------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Whisper with LocalAgreement** (whisper_streaming, 2023)                                                     | Re-transcribe a growing buffer; commit the longest common prefix of consecutive hypotheses                                                                                                                                                                   | Only with tiny or base, since every update re-runs the encoder (0.4 to 1 s each here **[R]**). Latency is at least two update intervals                                                                                                                               |
| **SimulStreaming / AlignAtt** (2025), packaged in **WhisperLiveKit** v0.2.28 (2026-11-22, Apache-2.0, Python) | Uses cross-attention to decide when a token is stable; latency about 1 to 2x the chunk length                                                                                                                                                                | No. SimulStreaming's own docs say CPU "would be too slow for real-time" with large-v3 and recommend a 10 GB GPU [27]. It is also a Python server                                                                                                                      |
| `whisper-stream.exe` (already on disk)                                                                        | Naive sliding window every 500 ms                                                                                                                                                                                                                            | Demo quality; no stable prefix                                                                                                                                                                                                                                        |
| **Parakeet Unified EN, buffered streaming**                                                                   | One model; left, chunk and right context are chosen at run time. WER on the legacy leaderboard sets: offline 5.91, 2.08 s latency 6.14, 1.12 s 6.29, 0.56 s 6.52, 0.24 s 7.35, 0.16 s 8.44 **[V]** [5]. transcribe.cpp implements all six configurations [7] | Plausible at the 1 to 2 s settings. Buffered streaming recomputes about 5.6 s of left context per chunk, so each step costs roughly an offline pass over 7 to 8 s of audio: about 0.5 s on a 14x-realtime CPU **[U, estimate]**. The 80 to 160 ms settings need a GPU |
| **Nemotron Speech Streaming EN 0.6B**                                                                         | Cache-aware FastConformer-RNNT: only new audio is processed; 80, 160, 560 or 1120 ms chunks                                                                                                                                                                  | The most CPU-friendly true streaming model of this quality. Leaderboard 7.47 **[R]**; 13.6x realtime on CPU offline **[3P]**                                                                                                                                          |
| **Moonshine Streaming (v2)**                                                                                  | Sliding-window encoder with 80 ms lookahead, autoregressive decoder                                                                                                                                                                                          | Yes; designed for edge CPUs. Lower accuracy than Parakeet (FLEURS English 7.9 to 8.6 against 4.0 **[3P]**), and the card warns it can hallucinate or repeat on short or noisy segments [18]                                                                           |
| Kyutai STT                                                                                                    | Delayed-streams model, 0.5 s or 2.5 s delay                                                                                                                                                                                                                  | GPU; no ggml port                                                                                                                                                                                                                                                     |
| Voxtral Mini 4B Realtime                                                                                      | Native streaming, 240 ms to 2.4 s delay                                                                                                                                                                                                                      | No: 0.58x realtime on CPU **[3P]**, 16 GB GPU **[V]**                                                                                                                                                                                                                 |

For push-to-talk dictation that pastes on release, partials are a display nicety, not a correctness feature. A sound design is that **the final text always comes from one offline pass over the full utterance** (best accuracy, one paste), and the overlay may optionally show partials from the same Parakeet Unified model in streaming mode on machines with a GPU. With an offline pass that costs well under a second, streaming buys little for utterances of 2 to 30 s.

---

## 8. Whisper failure modes on short or silent audio

What goes wrong:

- **Stock-phrase hallucination on silence.** Typical outputs are "you", "Thank you.", "Thanks for watching", and bracketed markers such as `[BLANK_AUDIO]`, `(buzzing)` and `(crickets chirping)`. "you" and all three markers appeared in my probes on this machine **[R]**. A 2026 dictation study reports hosted Whisper writing "you" and "Thank you for watching." on all ten of its no-speech clips, per a search summary of [24] that I did not confirm in the abstract. The usual explanation, that Whisper learned these from subtitled video endings, is plausible but **[U]**.
- **The log-probability check does not catch it.** A confident hallucination has a high average log-prob and passes the default `-lpt -1.0` threshold [28] (secondary source).
- **Short clips are zero-padded to 30 s**, which costs latency and feeds the model mostly padding.
- **Repetition loops** when `audio_ctx` is set too small (reproduced: 2 s clip with `-ac 256`) or when previous-text conditioning carries an error forward.
- **Word errors on short, clipped audio** ("And so am I fellow Americans" from tiny.en on the 2 s clip **[R]**).

Mitigations, in order of value for LocalFlow:

1. **VAD-gate before ASR.** If Silero finds no speech, do not call the model. This removed every hallucination in the probes **[R]**. Mind the whispered-speech caveat in section 6.
2. **Use a model that does not hallucinate on silence.** Transducer and CTC models emit blanks; Parakeet returned empty text for both silent probes **[R]**.
3. **Post-filter.** Drop outputs that are only a bracketed or parenthesised marker, or that exactly match a short blocklist ("you", "Thank you.", "Thanks for watching.") when VAD speech duration is under about 1 s. Handy has a similar guard ("skip pasting a transcript that is only a period", 0.10.1).
4. **Decode settings for short dictation:** `-bs 1 -bo 1` (greedy: faster, and no worse on these few clips), `-nf` (no temperature fallback), `-mc 0` (no previous-text context between utterances), and keep `-nth 0.6`. `--suppress-nst` did **not** stop the silence hallucination here **[R]**; it only blocks non-speech tokens, so "You" came out instead.
5. **Size `audio_ctx` to the audio, with margin.** The encoder emits 50 frames per second, so use about `duration_s * 50 + 128`, with a floor around 384 (my rule of thumb, **[U]**). Too small causes repetition. It is a whisper.cpp extension rather than standard Whisper behaviour, so test before shipping.
6. **Prefer real room tone to digital zeros when padding short audio**; the pre-roll gives you that for free **[U]**. whisper.cpp 1.9.3 also fixed a memory-safety bug on very short audio [2], another reason to update the runtime.
7. **Prompt handling.** An initial prompt biases vocabulary and style, but a prompt can be echoed into the output on short or empty audio **[U]**. Keep it to a short vocabulary list and never send one with a no-speech clip.

---

## 9. Cloud and speech-to-speech, for comparison only

Cloud ASR is prohibited in LocalFlow. These numbers only calibrate what "good" looks like.

| Service                                                             | Accuracy                                      | Latency                                                                                                                      | Evidence                                       |
| ------------------------------------------------------------------- | --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------- |
| Zoom Scribe v2.1 Pro                                                | 4.87, rank 1 on the Open ASR Leaderboard      | n/a                                                                                                                          | [R] [1]                                        |
| ElevenLabs Scribe v2 and v2 Realtime                                | 5.93 on the leaderboard                       | about 150 ms median to committed transcript                                                                                  | [R], [V] [29]                                  |
| AssemblyAI Universal-3.5 Pro; U3 Pro Streaming                      | 6.21 on the leaderboard                       | about 150 ms P50 and 240 ms P90 after the VAD endpoint; the older Universal-Streaming about 300 ms to immutable text         | [R], [V] [30]                                  |
| OpenAI GPT-Transcribe and GPT-Live-Transcribe (released 2026-07-28) | gpt-transcribe 6.80 on the leaderboard        | streaming with tunable latency; $0.0045 and $0.017 per minute; gpt-4o-transcribe is no longer the recommended starting point | [R], [V] [31]                                  |
| Deepgram Nova-3 and Flux                                            | Nova-3 5.26 % batch WER on Deepgram's own set | Flux end-of-turn about 260 ms; independent benchmarks report much higher WER and around 1 s time-to-first-token              | [V], with conflicting third-party numbers [32] |
| Wispr Flow                                                          | n/a                                           | 700 ms from end of speech to formatted text, as a design target                                                              | [V] [22]                                       |

So a warm local Parakeet at an estimated 0.4 to 1.0 s on CPU, or about 0.1 s on GPU, would be in the same latency class as the cloud products once their network hop is counted, at roughly one to two WER points behind the best of them on the leaderboard.

**Full-duplex speech-to-speech.** Kyutai's Moshi (2024) remains the reference open full-duplex model. 2026 follow-ups include NVIDIA PersonaPlex-7B (January 2026, built on Moshi, with role and voice control, under the NVIDIA Open Model License), MoshiRAG, and an RL-tuned Moshika variant under a non-commercial licence [33]. These are conversational agents that listen and speak at once. They are 7B-class GPU models and solve a different problem from dictation.

**Diarization.** Not needed for single-user dictation. For reference, the practical local options are NVIDIA's Streaming Sortformer (4 speakers, v2.1; ported to transcribe.cpp), the multitalker Parakeet streaming model that transcribes and attributes in one pass, pyannote pipelines in Python, and `parakeet-rs` in Rust. whisper.cpp itself only offers stereo-channel diarization and the experimental tinydiarize model. If LocalFlow ever adds meeting transcription, Sortformer through the same transcribe.cpp runtime would avoid a second dependency.

---

## 10. Suggested sequence

1. **Now, no new dependencies:** add Silero VAD gating (`--vad -vm ggml-silero-v6.2.0.bin`, threshold near 0.3) and the output blocklist; switch decode flags to `-bs 1 -bo 1 -nf -mc 0`; consider `base.en-q5_1` with a duration-scaled `-ac`. Measured 0.42 to 0.80 s for 5 s **[R]**.
2. **Try Parakeet with what is on disk:** download `ggml-parakeet-tdt-0.6b-v3-q8_0.bin` and call `parakeet-cli.exe`. Better text, about 2 s cold **[R]**. Update the whisper.cpp runtime to v1.9.5 or newer first for the Parakeet decode fix.
3. **Target:** add `transcribe-cpp` behind LocalFlow's existing typed provider trait, load `parakeet-unified-en-0.6b-Q8_0.gguf` once at start-up or on first key-down, pass 16 kHz f32 PCM in memory (no temp WAV, no process spawn), and use its Silero session for gating. Enable `vulkan` as an opt-in setting with CPU fallback. Benchmark on this machine before making it the default.

Benchmark leftovers: the models and clips I used are in `<scratch>/bench-models` and `<scratch>/bench-audio` (about 1.5 GB), with the scripts in `<scratch>/bench-scripts`. They are safe to delete.

---

## Sources

1. Hugging Face Open ASR Leaderboard, version 1.16.2 (2026-11-20); table data and version changelog read from the Space: https://huggingface.co/spaces/hf-audio/open_asr_leaderboard ; version registry: https://huggingface.co/spaces/hf-audio/open_asr_leaderboard/blob/main/init.py ; paper: https://arxiv.org/abs/2510.06961
2. whisper.cpp releases and README (versions, assets, changelogs, VAD, OpenVINO, CUDA, Vulkan): https://github.com/ggml-org/whisper.cpp/releases ; https://github.com/ggml-org/whisper.cpp
3. whisper.cpp parakeet-cli README and model files: https://github.com/ggml-org/whisper.cpp/tree/master/examples/parakeet-cli ; https://huggingface.co/ggml-org/parakeet-GGUF
4. whisper.cpp VAD models: https://huggingface.co/ggml-org/whisper-vad
5. NVIDIA Parakeet Unified EN 0.6B model card: https://huggingface.co/nvidia/parakeet-unified-en-0.6b ; paper: https://arxiv.org/abs/2604.19079
6. whisper.cpp ggml model files and sizes: https://huggingface.co/ggerganov/whisper.cpp
7. transcribe.cpp GGUF model cards with per-quant WER and speed (handy-computer organisation): https://huggingface.co/handy-computer ; https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf ; https://huggingface.co/handy-computer/parakeet-tdt-0.6b-v3-gguf ; model doc with benchmark method: https://github.com/handy-computer/transcribe.cpp/blob/main/docs/models/parakeet-unified-en-0.6b.md
8. Handy source (dependencies, Windows backend choice, VAD constants) and releases: https://github.com/cjpais/Handy/blob/main/src-tauri/Cargo.toml ; https://github.com/cjpais/Handy/releases
9. Handy README (models, manual install URLs): https://github.com/cjpais/Handy
10. transcribe.cpp repository, releases and Rust binding: https://github.com/handy-computer/transcribe.cpp ; https://github.com/handy-computer/transcribe.cpp/releases ; https://crates.io/crates/transcribe-cpp
11. transcribe.cpp Windows build guide and VAD doc: https://github.com/handy-computer/transcribe.cpp/blob/main/docs/build-windows.md ; https://github.com/handy-computer/transcribe.cpp/blob/main/docs/vad.md
12. "Quantization for OpenAI's Whisper Models: A Comparative Analysis": https://arxiv.org/abs/2503.09905
13. whisper.cpp discussion #3752, quantisation benchmark on legacy hardware (seen through a search summary): https://github.com/ggml-org/whisper.cpp/discussions/3752
14. Silero VAD releases and defaults: https://github.com/snakers4/silero-vad/releases ; https://github.com/snakers4/silero-vad/blob/master/src/silero_vad/utils_vad.py
15. distil-whisper large-v3.5 model card: https://huggingface.co/distil-whisper/distil-large-v3.5
16. moondream parakeet-ultra model card: https://huggingface.co/moondream/parakeet-ultra
17. IBM Granite Speech 5.0 TurboCTC model card: https://huggingface.co/ibm-granite/granite-speech-5.0-470m-turboctc
18. Moonshine Streaming model card and Moonshine v2 paper: https://huggingface.co/UsefulSensors/moonshine-streaming-medium ; https://arxiv.org/abs/2602.12241
19. Kyutai STT model cards: https://huggingface.co/kyutai/stt-1b-en_fr ; https://huggingface.co/kyutai/stt-2.6b-en
20. VoiceInk README: https://github.com/Beingpax/VoiceInk
21. Microsoft Windows ML supported execution providers, DirectML listed as legacy (seen through a search summary): https://learn.microsoft.com/windows/ai/new-windows-ml/supported-execution-providers
22. Wispr Flow, "Technical challenges and breakthroughs behind Flow" (2025-09-11): https://wisprflow.ai/post/technical-challenges
23. TEN VAD repository and licence: https://github.com/TEN-framework/ten-vad
24. "Beyond Word Error Rate: A Failure Taxonomy of Streaming Dictation ASR" (2026-11-12): https://arxiv.org/abs/2611.16919
25. Pipecat Smart Turn v3.2: https://github.com/pipecat-ai/smart-turn ; https://huggingface.co/pipecat-ai/smart-turn-v3
26. LiveKit turn detector model card: https://huggingface.co/livekit/turn-detector ; https://pypi.org/project/livekit-plugins-turn-detector
27. SimulStreaming and WhisperLiveKit: https://github.com/ufal/SimulStreaming ; https://github.com/QuentinFuxa/WhisperLiveKit ; Simul-Whisper paper: https://arxiv.org/abs/2406.10052
28. Whisper silence hallucination write-ups (secondary, seen through a search summary): https://yage.ai/share/whisper-repetition-hallucination-en-20260526.html ; https://community.openai.com/t/whisper-api-hallucinating-on-empty-sections/93646
29. ElevenLabs Scribe v2 Realtime latency methodology (search summary): https://elevenlabs.io/docs/eleven-api/concepts/latency-methodology
30. AssemblyAI Universal-3 Pro Streaming (search summary): https://www.assemblyai.com/blog/universal-3-pro-streaming ; https://www.assemblyai.com/docs/voice-agents/u3-pro-partials
31. OpenAI GPT-Live-Transcribe and transcription guide (search summary): https://developers.openai.com/api/docs/models/gpt-live-transcribe ; https://developers.openai.com/api/docs/guides/transcription
32. Deepgram Flux docs and a third-party benchmark (search summary): https://developers.deepgram.com/docs/flux/quickstart ; https://developers.deepgram.com/docs/measuring-streaming-latency ; https://benchmarks.coval.ai/models/universal-streaming
33. Moshi and PersonaPlex (search summary): https://arxiv.org/abs/2410.00037 ; https://research.nvidia.com/labs/adlr/personaplex
34. Other model cards read directly: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v2 ; https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3 ; https://huggingface.co/nvidia/canary-1b-v2 ; https://huggingface.co/nvidia/nemotron-speech-streaming-en-0.6b ; https://huggingface.co/nvidia/nemotron-3.5-asr-streaming-0.6b ; https://huggingface.co/Qwen/Qwen3-ASR-0.6B ; https://huggingface.co/mistralai/Voxtral-Mini-4B-Realtime-2602
35. Runtime versions (GitHub and crates.io APIs, read 2026-11-22): https://github.com/SYSTRAN/faster-whisper ; https://github.com/OpenNMT/CTranslate2 ; https://github.com/k2-fsa/sherpa-onnx ; https://crates.io/crates/sherpa-onnx ; https://crates.io/crates/whisper-rs ; https://crates.io/crates/transcribe-rs ; https://crates.io/crates/parakeet-rs ; https://crates.io/crates/ort ; https://github.com/mudler/parakeet.cpp
