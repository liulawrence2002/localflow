# LocalFlow: inference and performance optimisation research

> Research appendix produced on 2026-11-22 by an independent research agent for the voice pipeline review. It is kept as written, apart from removing machine-specific paths. Line references to `native_dictation.rs` describe the code before the changes in `../VOICE_PIPELINE_REVIEW.md`. Figures are labelled by the author as vendor, third-party, reproduced, or unverified; treat unlabelled numbers as unverified.

Date: 2026-11-22. Scope: local speech pipeline (whisper.cpp + Ollama) on Windows, Tauri 2 / Rust. Cloud inference is out of scope except for the economics table in section 7.

Evidence labels used throughout:

- **[P]** published figure or documented behaviour, with a numbered source.
- **[L]** local measurement taken today on the target machine (Core Ultra 7 165H) with the binaries in `.localflow-runtime` (whisper.cpp 1.9.1, `ggml-tiny.en-q5_1.bin`, `jfk.wav`, 11 s). Five runs per configuration, median reported. **The machine was under roughly 30% background CPU load from other work during the runs, and individual configurations varied a lot between runs. Treat these as indicative only and re-measure on an idle machine before acting on small differences.**
- **[E]** my estimate. Not measured, not published.

---

## 0. Headline findings

1. **The Whisper encoder cost is fixed, not proportional to utterance length.** Whisper pads every clip to 30 s, so a 3 s utterance costs about the same encoder time as an 11 s one. This is why `audio_ctx` and model choice dominate, and why cold-start overhead is a smaller slice than it looks.
2. **A warm `whisper-server` removes model load and process spawn** (about 130-190 ms load plus roughly 100-250 ms spawn/teardown per utterance [L]) and also removes the temp WAV and JSON file round trip, because the server decodes the uploaded bytes in memory [1][2].
3. **Thread count is currently a real risk on this CPU.** Over-threading was badly slower in the local runs: `-t 14` and `-t 22` took 3.3 s and 6.2 s against about 1.7 s at `-t 6` to `-t 8` [L]. Use the P-core count (6 on the 165H).
4. **`audio_ctx` is the largest single lever for short audio** (about 3x published [9]; encode 1248 ms to 508 ms locally at `-ac 768` [L]) but it is unsafe on stock models if set too low [10]. It needs a guard-banded formula and a WER regression test, or the ACFT fine-tuned models.
5. **In the current `whisper-server` source, `-nf/--no-fallback` is parsed but never applied.** Fallback is controlled by the request field `temperature_inc`; send `temperature_inc=0.0` to disable it [2]. Verify against the shipped binary.
6. **whisper.cpp now ships Parakeet.** The 1.9.x runtime already in `.localflow-runtime/whisper/Release` contains `parakeet-cli.exe` and `parakeet.dll`, and `ggml-org/parakeet-GGUF` publishes `parakeet-tdt-0.6b-v3` in q4_0 (339 MiB), q4_k (396 MiB), q8_0 (638 MiB) and f16 [6][7]. That is an accuracy upgrade path with no new runtime dependency, but the CLI is minimal (no server, no prompt) so it would be cold-spawn only today.
7. **The official whisper.cpp v1.9.5 release has no Vulkan or OpenVINO Windows binary.** Only CPU, BLAS and CUDA zips are published [3]. Vulkan (the only route to the Arc iGPU and to non-NVIDIA GPUs) means building it yourself.
8. **Ollama: pin `num_ctx`, cap `num_predict`, keep the prompt prefix byte-stable, and send `think: false`.** Several current small models (Qwen3, Qwen3.5, Gemma 4) are thinking models; leaving thinking on is a silent multi-second latency cost [20][24].

---

## 1. whisper.cpp

### 1.1 Current release and Windows prebuilt binaries

Latest tagged release is **v1.9.5 (2026-10-06)**, a maintenance release that updates ggml to v0.26.0. The tagged release links to nightly build **b5454** for binaries. The newest nightly is **b5907 (2026-11-20)** [3]. LocalFlow's runtime reports `whisper.cpp version: 1.9.1` [L].

Windows assets in b5454 (the v1.9.5 build) [3]:

| Asset | Size | What it is |
|---|---|---|
| `whisper-bin-x64.zip` | 8.9 MB | CPU only. Ships multiple `ggml-cpu-*.dll` variants and picks one at run time (the local copy loads `ggml-cpu-alderlake.dll`) |
| `whisper-blas-bin-x64.zip` | 21.7 MB | CPU plus OpenBLAS |
| `whisper-bin-win-cuda-12.4.0-x64.zip` | 685 MB | CUDA 12.4 with cuBLAS |
| `whisper-bin-win-cuda-11.8.0-x64.zip` | 285 MB | CUDA 11.8 with cuBLAS |
| `whisper-bin-Win32.zip`, `whisper-blas-bin-Win32.zip` | 5.5 / 12.6 MB | 32-bit |
| `whisper-bin-win-cpu-arm64.zip`, `whisper-bin-win-cuda-13.4-arm64.zip`, `whisper-bin-win-opencl-adreno-arm64.zip` | 4.4 / 291 / 4.9 MB | Windows on ARM |

Notes:

- **Naming changed.** Up to v1.9.4 the CUDA zips were called `whisper-cublas-12.4.0-bin-x64.zip` and `whisper-cublas-11.8.0-bin-x64.zip`. From v1.9.5 they are `whisper-bin-win-cuda-<ver>-x64.zip` [3]. Any download script keyed on the old name will break.
- **No Vulkan and no OpenVINO Windows zip** in the official assets [3]. Both are build-from-source: `cmake -B build -DGGML_VULKAN=1` and `cmake -B build -DWHISPER_OPENVINO=1` (recommended OpenVINO 2026.3.0) [4].
- The CUDA zip is 285-685 MB. That is 30 to 75 times the CPU zip and is not something to bundle in the installer. Treat it as an optional download.
- BLAS build: no published evidence that it helps tiny/base models on recent builds. I would not ship it [E].

### 1.2 Persistent `whisper-server`

Startup flags (from the current README and `server.cpp`) [1][2]:

```
whisper-server.exe
  -m <path>\ggml-tiny.en-q5_1.bin
  -l en
  -t 6                       # P-core count, see 1.5
  --host 127.0.0.1           # this is already the default
  --port <free port chosen by the app>
  -nt                        # no timestamps
  -bs 1 -bo 1                # greedy; server default is already beam -1 (greedy), best_of 2
  -sns                       # suppress non-speech tokens
  --vad -vm <path>\ggml-silero-v6.2.0.bin   # optional, see 1.7
```

Server defaults differ from the CLI, which matters when comparing outputs [1][2][5]:

| Parameter | `whisper-cli` default | `whisper-server` default |
|---|---|---|
| beam size `-bs` | 5 | -1 (greedy) |
| best of `-bo` | 5 | 2 |
| threads `-t` | 4 | min(4, hardware threads) |
| flash attention | on | on in source (`flash_attn = true`); the README help text still says false |
| previous-text context | carried | `no_context = true` |
| host / port | n/a | `127.0.0.1` / `8080` |

Endpoints [1][2]:

- `POST /inference`, multipart form. The `file` field is required.
- `POST /load`, multipart form, field `model` = path to a model file. It frees the current context and loads the new one in place. If the load fails the server calls `exit(1)`, so the app must supervise and restart it.
- `GET /health` returns `{"status":"ok"}` when ready. Poll this after spawn before sending audio.
- `GET /` serves a small HTML form.

Per-request form fields accepted by `/inference` (read from `server.cpp`) [2]:

`file`, `temperature`, `temperature_inc`, `response_format`, `prompt`, `carry_initial_prompt`, `no_timestamps`, `token_timestamps`, `language`, `detect_language`, `translate`, `beam_size`, `best_of`, `audio_ctx`, `max_context`, `max_len`, `split_on_word`, `word_thold`, `entropy_thold`, `logprob_thold`, `no_speech_thold`, `offset_t`, `offset_n`, `duration`, `suppress_nst` (alias `suppress_non_speech`), `vad`, `vad_threshold`, `vad_min_speech_duration_ms`, `vad_min_silence_duration_ms`, `vad_max_speech_duration_s`, `vad_speech_pad_ms`, `vad_samples_overlap`, `diarize`, `tinydiarize`, `no_language_probabilities`, `debug_mode`.

Recommended request:

```
curl http://127.0.0.1:<port>/inference ^
  -F file=@utterance.wav;type=audio/wav ^
  -F response_format=json ^
  -F temperature=0.0 ^
  -F temperature_inc=0.2 ^        (0.0 to disable fallback)
  -F no_timestamps=true ^
  -F suppress_nst=true ^
  -F prompt="<vocabulary prompt>"
```

`response_format` values: `json` (returns `{"text": ...}`), `text`, `srt`, `vtt`, `verbose_json`. Token-level timestamps are only computed for `verbose_json` or when `max_len`/`split_on_word` is set, so plain `json` is the cheapest [2].

Things to know before shipping it:

- **`-nf` appears to be a no-op in the server.** `no_fallback` is parsed and printed but the request path sets `wparams.temperature_inc = params.temperature_inc` directly. Use the `temperature_inc` form field [2].
- **Requests are serialised** behind one mutex [2]. Fine for single-user dictation.
- **Audio is decoded from the request body in memory** unless `--convert` is passed (which writes a temp file and shells out to ffmpeg). Do not pass `--convert` [2].
- **Security posture.** The README warns not to run it with administrative privileges and to sandbox it because it accepts uploads [1]. Bind to `127.0.0.1` (default), use a random free port, do not pass `--convert`, and consider `--public` pointing at an empty directory. There is no authentication, so any local process can post audio to it or call `/load` with an arbitrary path. That is a local-only exposure, but it is worth stating in the plan.
- It prints `Received request: <filename>` to stdout, not the transcript. Still, do not log the server's stdout verbatim without checking, to stay inside the "transcripts out of logs" rule.

### 1.3 Server versus in-process bindings versus cold CLI

| Option | Latency | Build and ship cost | Regression risk |
|---|---|---|---|
| Cold `whisper-cli` (today) | Pays model load + process spawn + temp WAV + JSON file every utterance | None | None |
| `whisper-server` sidecar | Model stays loaded; audio and result travel over loopback HTTP, no temp files | Low. The exe already ships in the runtime. Needs spawn, health poll, restart-on-crash, port selection, shutdown on app exit | Low to medium. Different decode defaults (table above) will change output unless you set them explicitly |
| `whisper-rs` in-process | Same warm benefit, no HTTP hop, direct `&[f32]` input, full access to `FullParams` (audio_ctx, callbacks, abort) | Medium to high. `whisper-rs` 0.16.0 (2026-03-12) builds whisper.cpp from source via `whisper-rs-sys` 0.15, needing CMake and a C++ toolchain and, unless `WHISPER_DONT_GENERATE_BINDINGS` is set, libclang. Features: `cuda`, `vulkan`, `openblas`, `metal`, `hipblas`. No OpenVINO feature. Repo is on Codeberg; 0.14.0 and 0.15.0 were yanked [8] | Medium. A native crash takes the whole Tauri process down; a sidecar crash does not. The crate tracks an older whisper.cpp than 1.9.5 (exact pinned version not stated on docs.rs) |

Recommendation: **server sidecar first.** It captures nearly all of the latency win, keeps a crash boundary between native inference and the UI process, and fits the existing "platform code behind narrow modules, typed provider traits" convention: add a `WhisperServerProvider` beside the CLI provider and keep the CLI as the fallback path. Revisit `whisper-rs` only if you need streaming partials or per-token callbacks.

### 1.4 Cold-start overhead

I found **no published measurement** of whisper-cli cold start versus server for tiny models. The only published load figures are examples in the bench README (for example `load time = 240.82 ms` for small.en on an unnamed machine) [11]. Published memory figures: tiny about 273 MB, base about 388 MB, small about 852 MB resident for the unquantised models [4].

Local measurement, 11 s clip, `tiny.en-q5_1`, medians of 5 [L]:

| Configuration | Wall (ms) | whisper "load" (ms) | Encode (ms) | whisper "total" (ms) |
|---|---|---|---|---|
| CLI `-t 4`, beam 5 (CLI default) | 1900 | 127 | 952 | 1793 |
| CLI `-t 6`, beam 5 | 1856 | 130 | 931 | 1702 |
| CLI `-t 6 -bs 1 -bo 1` | 1975 | 151 | 1248 | 1837 |
| CLI `-t 8 -bs 1 -bo 1` | 1815 | 186 | 990 | 1705 |
| CLI `-t 14 -bs 1 -bo 1` | 3500 | 190 | 1186 | 3304 |
| CLI `-t 22 -bs 1 -bo 1` | 6472 | 182 | 1457 | 6227 |
| CLI `-t 6 -bs 1 -bo 1 -ac 768` | 1401 | 159 | 508 | 1279 |
| **Server**, `-t 6`, greedy, 6 requests | **880-1160** | 0 (already loaded) | n/a | n/a |
| **Server**, same, `audio_ctx=768`, 3 requests | **434-446** | 0 | n/a | n/a |

Reading this honestly:

- Directly attributable cold cost per utterance is the whisper-reported load time (127-190 ms) plus the gap between wall time and whisper's own total (107-250 ms, process creation, DLL loading, backend selection, teardown). Call it **250-400 ms per utterance** [L].
- The server rows are about 800 ms faster than the CLI rows, which is more than that. Some of the gap is noise from background load (note the encode times of 931-1248 ms for a tiny model, and that greedy was not faster than beam here), so **do not quote the 800 ms figure**. A defensible claim is "at least 250-400 ms saved, plus file I/O".
- Encode time near 1 s for tiny on six P-cores is slower than I would expect on an idle machine. Re-run `whisper-bench.exe -m ggml-tiny.en-q5_1.bin -t 6` idle to get a clean baseline.

### 1.5 Flags for short-utterance latency

**Threads (`-t`).** Use the number of **performance cores**, not logical processors.

- Published guidance: Intel's oneMKL guide recommends running threads on P-cores only on hybrid CPUs, because static load balancing waits on the slower E-cores [13]. Mozilla's measurements reach the same rule (use physical or performance core count; adding threads past that slowed inference) [12]. A llama.cpp discussion reports about 3x better throughput on a 12th-gen Intel CPU when restricted to P-cores [14]. None of these is a whisper.cpp benchmark; ggml uses the same synchronised thread-pool style, so the reasoning carries over [E].
- Local: `-t 14` and `-t 22` were 2x to 3.6x slower than `-t 6`/`-t 8` [L].
- **Setting for the 165H: `-t 6`.** General rule for the app: `threads = clamp(P-core count, 2, 8)`; on CPUs with no hybrid split use physical core count capped at 8; never use `available_parallelism()` directly, since that returns 22 here. P-cores can be identified on Windows with `GetLogicalProcessorInformationEx(RelationProcessorCore)` and the `EfficiencyClass` field (highest class = P-cores) [E, from Windows API knowledge, not re-verified today].
- Leave `-p` (processors) at 1. It splits audio into parallel chunks and is for long files.

**Greedy versus beam (`-bs`, `-bo`).** CLI default is beam 5, best-of 5 [5]. For dictation use `-bs 1 -bo 1`. Beam search multiplies decoder work by roughly the beam width; for tiny/base on CPU the decoder is a minority of total time for a short clip, so expect a modest saving, larger on longer utterances [E]. `-bo` only matters when temperature is above 0 (fallback passes), where it sets the number of sampled candidates; `-bo 1` makes fallback passes cheap. Accuracy cost of greedy on tiny.en is small but real; measure on the bench corpus. The local runs did not show a clean greedy win because of noise [L].

**`-ac` / `audio_ctx`.** Default 0 means the full 1500 encoder frames (30 s). 50 frames = 1 s.

- Published: on an i7-11700K with 2 threads, greedy, 200 Common Voice clips averaging 5.7 s, the formula `audio_ctx = (seconds / 30) * 1500 + 128` cut total time from 121 s to 40 s for tiny.en (WER 25.81 to 25.83) and 204 s to 60 s for base.en (WER 20.06 to 19.2). A fixed `audio_ctx = 256` wrecked accuracy (tiny.en WER 38.73, base.en 43.03) [9].
- Published counter-evidence: FUTO reports that with a tight dynamic context stock models fail to stop and repeat ("the decoder doesn't know when to stop"): tiny.en WER 4.73 at 1500 versus 225.99 dynamic on LibriSpeech clean (first 128 utterances). Their ACFT fine-tuned tiny.en gets 4.96 at 1500 and 5.50 dynamic. ACFT q8_0 `.bin` files for tiny.en, base.en, small.en and the multilingual equivalents are published for whisper.cpp; repository is MIT [10].
- Local: `-ac 768` halved encode (1248 to 508 ms) and the warm server answered in about 440 ms. The transcript lost a sentence break ("...for you ask what..." instead of "...for you. Ask what...") [L]. One clip, but it shows output does change.
- **Risks:** runaway repetition or truncation when the context is too tight; punctuation and casing drift; behaviour is model-specific; the value must cover the whole clip or the tail is silently dropped.
- **If adopted:** `audio_ctx = min(1500, ceil(seconds * 50) + 128)`, and add a floor (I suggest 512, roughly 10 s) so very short clips keep generous padding [E]. Send it per request (`-F audio_ctx=<n>`); it cannot be a fixed server flag. Gate on a WER run over the bench corpus, and add a repetition guard (reject and retry at 1500 if the output is much longer than plausible for the duration). Prefer the ACFT models if you want to go lower than the guard band.

**Temperature fallback (`-nf`, `-tp`, `-tpi`).** Default temperature 0.0 with increment 0.2: if a decode fails the entropy (`-et 2.40`) or log-probability (`-lpt -1.00`) check, the whole decode is retried at 0.2, 0.4 and so on, up to five more passes [5]. This is the main source of latency spikes on mumbled or silent input. The whisper.cpp CLI README also notes that fallback "tends to turn uncertainty into fluent hallucinations" on non-speech [5]. Options: `-nf` on the CLI, or `temperature_inc=0.0` on the server. Cost: no recovery from a degenerate greedy decode (repetition loops). A reasonable middle path is fallback off plus VAD and the app-level repetition guard [E].

**`-mc` / max context.** Limits how many previous-text tokens condition the decoder. For single-shot utterances there is no previous window, and the server already sets `no_context = true` [2]. `-mc 0` is harmless and makes this explicit on the CLI; it does not remove `--prompt`. Negligible latency effect for one-segment audio [E].

**`-nt`.** No timestamp tokens. Slightly fewer decoder steps and cleaner text. Keep it. Side effect: with timestamps off Whisper emits a single segment, which is what dictation wants.

**Flash attention.** `-fa` is on by default in both CLI and server in current source; `-nfa` disables it [2][5]. Nothing to change. Recent releases contain several Vulkan flash-attention fixes [3], so if you build Vulkan yourself, test with and without.

**`-sns` / `suppress_nst`.** Suppresses non-speech tokens (music notes, bracketed sound tags). No latency cost, reduces junk output. Recommended [5].

**`--prompt`.** Costs decoder prompt tokens on every pass. Maximum is half the text context (about 224 tokens) [5]. Keep the vocabulary prompt short; each prompt token is evaluated once per decode pass, so a long prompt is a small but real per-utterance cost on CPU [E].

### 1.6 GPU on this machine

- **RTX 4070 Laptop (CUDA).** Published reference: large-v2 on an RTX 3070 Ti 8 GB, 13 minutes of audio in 1m05s with whisper.cpp and flash attention (about 12x real time) [15]. For tiny/base the GPU is not needed; its value is making `small.en`, `large-v3-turbo` or Parakeet feel instant. Costs: 285-685 MB download [3], first-inference CUDA initialisation delay, VRAM shared with Ollama (llama3.2:3b needs roughly 2-3 GB), and on laptops the discrete GPU may be powered down and take time to wake [E].
- **Intel Arc iGPU.** Reachable via a self-built Vulkan binary or the OpenVINO encoder. The OpenVINO path only accelerates the encoder, requires a separately converted `ggml-<model>-encoder-openvino.xml/.bin`, and the first run on a device is slow while the model is compiled and cached [4]. Published: small model, i7-12700K, 13 minutes of audio: whisper.cpp 2m05s versus whisper.cpp + OpenVINO 1m45s (16% faster) at 1049 MB versus 1642 MB RAM [15]. Not worth the packaging cost for tiny/base [E].

### 1.7 VAD (`--vad`, Silero)

whisper.cpp has built-in VAD using a ggml Silero model. Download `ggml-silero-v6.2.0.bin` (885 KB) from `ggml-org/whisper-vad` [4][16]:

```
--vad -vm ggml-silero-v6.2.0.bin -vt 0.50 -vspd 250 -vsd 100 -vp 30
```

Defaults: threshold 0.50, min speech 250 ms, min silence 100 ms, speech pad 30 ms, overlap 0.10 s [1][5]. On the server, `--vad` and `-vm` are start-up flags and the thresholds can also be sent per request (`vad=true`, `vad_threshold`, ...) [2].

For push-to-talk dictation VAD does **not** shorten the encoder pass (still one 30 s window). Its value is (a) skipping inference entirely when the user said nothing, and (b) trimming leading/trailing silence, which is the main hallucination trigger [5]. For dictation raise `-vp` to about 100-200 ms so word onsets are not clipped [E]. If you adopt `audio_ctx`, compute it from the post-trim duration.

---

## 2. Quantisation

### 2.1 Whisper ggml

File sizes [16]:

| Model | f16 (default `.bin`) | q8_0 | q5_1 / q5_0 |
|---|---|---|---|
| tiny.en | 74.1 MiB | 41.5 MiB | 30.7 MiB |
| base.en | 141.1 MiB | 78.0 MiB | 57.0 MiB |
| small.en | 465.0 MiB | 252.2 MiB | 181.3 MiB |
| large-v3-turbo | 1549.3 MiB | 833.7 MiB | 547.4 MiB (q5_0) |

Published accuracy and speed evidence is thin:

- arXiv 2503.09905 (whisper.cpp base, LibriSpeech, only 10 files, Xeon 2.2 GHz): WER 1.99% unquantised, 1.59% INT4, 1.99% INT5, 1.99% INT8; average latency 10.64 s, 10.55 s, 11.11 s, 9.02 s. So INT5 was the slowest and INT8 the fastest (about 15% faster than unquantised). The sample is too small to rank accuracy, and the paper's own summary numbers do not match its table [17].
- A whisper.cpp community report on a 2010-era i5-460M found q5_0/q5_1 3x to 5.5x slower than q4_0 on that CPU. That is a pre-AVX2 chip and one 11 s clip, so it does not transfer to the 165H, but it shows 5-bit kernels are the least optimised path [18].
- The whisper.cpp README says only that quantised models "require less memory and disk space and depending on the hardware can be processed more efficiently" [4].

Practical reading [E]:

- 5-bit formats need unpacking that does not map neatly onto SIMD byte lanes; q8_0 does. On AVX2/AVX-VNNI CPUs q8_0 is usually as fast as or faster than q5_x, at about 10 MB more disk for tiny.en.
- Tiny models have the least redundancy, so they lose the most from aggressive quantisation. q8_0 is effectively lossless; q5_1 costs a little; f16 buys nothing measurable over q8_0.
- **Action:** benchmark `ggml-tiny.en-q8_0.bin` and `ggml-base.en-q8_0.bin` against the current q5_1 on the bench corpus (WER and encode ms). This is a file swap with no code change. If q8_0 is not slower, switch.

### 2.2 CTranslate2 / faster-whisper INT8

Published (small model, i7-12700K, 8 threads, beam 5, 13 min audio; whisper.cpp v1.7.2, faster-whisper v1.1.0) [15]:

| Implementation | Precision | Time | RAM |
|---|---|---|---|
| openai/whisper | fp32 | 6m58s | 2335 MB |
| whisper.cpp | fp32 | 2m05s | 1049 MB |
| whisper.cpp (OpenVINO) | fp32 | 1m45s | 1642 MB |
| faster-whisper | fp32 | 2m37s | 2257 MB |
| faster-whisper | int8 | 1m42s | 1477 MB |
| faster-whisper, batch 8 | int8 | 51s | 3608 MB |

INT8 made faster-whisper 35% faster than its own fp32 and 18% faster than whisper.cpp fp32, at 40% more memory. Batching does not help single short utterances. The table has no quantised whisper.cpp row. GPU: large-v2 fp16 1m03s versus int8 59s with VRAM 4525 MB to 2926 MB [15].

faster-whisper itself is Python (latest wheel 1.2.1). The engine, CTranslate2 4.9.1, is a C++ library with a 23 MB Windows wheel, and the `ct2rs` crate (0.11.2) provides Rust bindings including Whisper [19]. GPU needs CUDA 12 and cuDNN 9 [15]. You would have to reimplement the mel front end and decoding loop or rely on ct2rs's. Gain over a warm whisper.cpp for tiny/base is at most modest [E].

### 2.3 ONNX INT8 for Parakeet and Moonshine

- **Parakeet TDT 0.6B via sherpa-onnx.** `sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8` (English): encoder 622 MB, decoder 6.9 MB, joiner 1.7 MB. Published RTF with 2 threads on unnamed hardware: 0.118 on a short clip; on an RK3588 Cortex-A76: 0.220 / 0.142 / 0.118 / 0.088 at 1/2/3/4 threads. v3-int8 covers 25 European languages, same encoder size [21]. Open ASR Leaderboard average WER: 6.05% (v2) and 6.32% (v3) [22]. An x86 laptop CPU should beat the A76 numbers comfortably [E].
- **Parakeet inside whisper.cpp.** `ggml-parakeet-tdt-0.6b-v3` q4_0 339 MiB, q4_k 396 MiB, q8_0 638 MiB, f16 1198 MiB [7]. `parakeet-cli` flags: `-t`, `-m`, `-f`, `-ng`, `-dev`, `-ps`, `-otxt`, `-of`, `-np` [6]. No prompt/vocabulary biasing, no server, no JSON output yet.
- **Moonshine.** v2 (streaming, "ergodic encoder") sizes: Tiny 34 M, Small 123 M, Medium 245 M parameters; Open ASR average WER 12.01 / 7.84 / 6.65%. Response latency on a MacBook M3: v2 Tiny 50 ms, v2 Small 148 ms, v2 Medium 258 ms, against Whisper Tiny 289 ms, Base 553 ms, Small 1940 ms, Large v3 11286 ms [23]. Key property: compute scales with audio length (no 30 s padding) and encoding can run while the user is still speaking. Library is MIT, models MIT, except the legacy non-streaming non-English models which are under a non-commercial community licence; Windows is a supported platform with a C API [24a]. INT8 exports: a community Tiny export is 79.8 MB INT8 versus 168 MB FP32; I could not verify an INT8 WER figure from a primary source.

### 2.4 LLM quantisation (cleanup model)

- llama.cpp's perplexity README (Llama-3-8B): mean KL divergence about 0.0013 for Q8_0, 0.0108 for Q5_K_M, 0.0313 for Q4_K_M; perplexity 6.23 / 6.29 / 6.41 [25]. I did not find a reliable 3B-specific table; the usual claim that small models suffer more from quantisation is plausible but unverified here.
- Speed on CPU is bound by memory bandwidth, so tokens/s scales roughly inversely with file size: Q8_0 is about 1.6x the size of Q4_K_M and correspondingly slower; on the GPU the difference is small as long as everything fits in VRAM [E].
- **Recommendation:** stay on Q4_K_M for CPU-only machines. On the RTX 4070, Q5_K_M or Q8_0 of a 3-4B model fits easily and is worth an A/B on cleanup fidelity (the failure that matters is the model changing words, not perplexity). "INT4" in the GGUF world means Q4_K_M/Q4_0; Ollama's default tags are Q4_K_M [26].

---

## 3. Alternative backends on Windows

Sizes are current Windows x64 package sizes; they bound the native payload from above [19].

| Backend | Current version | Payload | Hardware | Realistic for a Tauri app? |
|---|---|---|---|---|
| whisper.cpp CPU | 1.9.5 | 8.9 MB zip | Any x64 | Yes. Already shipped |
| whisper.cpp CUDA | 1.9.5 | 285-685 MB zip | NVIDIA | As an optional download only |
| whisper.cpp Vulkan | build from source | about 10-20 MB [E] | NVIDIA, AMD, Intel Arc | Yes, if you own the build. One binary for all GPUs; needs driver-quality testing |
| ONNX Runtime CPU | 1.32.0 (2026-11-13) | 16.7 MB wheel | Any x64 | Yes. Rust `ort` crate wraps ORT 1.31. sherpa-onnx 2.0.3 bundles ORT plus ASR pipelines in a 26-31 MB Windows archive and has an official `sherpa-onnx` Rust crate |
| ONNX Runtime DirectML | 1.24.4 (2026-03-17) | 25.1 MB wheel | Any DX12 GPU incl. Arc | Possible but fading: DirectML is in sustained engineering and its package has not followed ORT past 1.24; Microsoft directs new work to Windows ML [27] |
| ONNX Runtime CUDA EP | 1.32.0 | 157.8 MB wheel, plus CUDA 12 and cuDNN 9 runtime libraries (several hundred MB more [E]) | NVIDIA | Optional download only |
| OpenVINO | 2026.4.1 (2026-10-01) | 85 MB wheel; GenAI add-on 3.9 MB | Intel CPU, iGPU, NPU | Possible via the C API or the `openvino` crate (0.11.1). Intel-only benefit; needs models converted to IR; first-run compile delay [4] |
| TensorRT | 11.5.0.63 | Very large (multi-hundred MB to GB class [E]) | NVIDIA | No. Engines are built per GPU and driver on the user's machine, taking minutes |
| CTranslate2 | 4.9.1 | 23.1 MB wheel (CPU) | CPU; CUDA 12 + cuDNN 9 | Possible via `ct2rs` 0.11.2. No decisive gain over warm whisper.cpp at tiny/base |

**NPU (Core Ultra 7 165H, Meteor Lake).** OpenVINO's GenAI ASR pipeline documents CPU and GPU ("Use CPU or GPU as devices without any other code change"); the current page does not mention NPU [28]. Earlier release notes did list Whisper on NPU. The NPU plugin requires static shapes and has historically rejected dynamic-shape models [29]. I found no published Whisper-on-NPU latency for Meteor Lake. The first-generation NPU in the 165H is small; treat NPU as a power-saving experiment, not a latency play [E].

**Recommendation.** Stay on whisper.cpp for the CPU path. If accuracy becomes the priority, the next backend to evaluate is ONNX Runtime CPU via sherpa-onnx (Parakeet int8, or Moonshine v2 through its own library), because it works on every machine with a 26-31 MB runtime; or Parakeet GGUF through the whisper.cpp runtime you already ship. Add GPU via an optional CUDA download or a self-built Vulkan binary, not via DirectML or TensorRT.

---

## 4. Ollama latency

Current Ollama release: v0.40.17 (2026-11-10) [30].

### 4.1 keep_alive and preloading

- Default: models stay loaded 5 minutes. `keep_alive` accepts a duration string (`"10m"`, `"24h"`), a number of seconds, a negative value (`-1`, `"-1m"`) for indefinitely, or `0` to unload immediately. `OLLAMA_KEEP_ALIVE` sets the server default; the per-request value overrides it [20].
- Preload with an empty request: `POST /api/generate {"model":"llama3.2:3b"}` or `POST /api/chat {"model":"..."}`. Unload: `{"model":"...","keep_alive":0}` [20][31].
- **Make the warm-up request carry the same `options` and the same system prompt as the real request.** An empty preload loads weights but does not populate the prompt cache, and a preload with a different `num_ctx` leads to a reload on the real request (see 4.2). Best warm-up at record start [E]:

```json
POST /api/chat
{ "model": "llama3.2:3b", "stream": false, "keep_alive": "30m", "think": false,
  "messages": [ {"role":"system","content":"<exact cleanup system prompt>"},
                {"role":"user","content":"ok"} ],
  "options": { "num_ctx": 2048, "num_predict": 1, "temperature": 0 } }
```

This loads the model, allocates the context at the right size, and leaves the system-prompt tokens in the KV cache.

- Check `load_duration` in each response. If it is not near zero on real requests, something is evicting or reloading the model [31].

### 4.2 num_ctx and num_predict

- Default context is 4096 tokens when VRAM is below 24 GiB (32k at 24-48 GiB, 256k above), overridable with `OLLAMA_CONTEXT_LENGTH` or `options.num_ctx` [20][32]. The Modelfile reference still lists a 2048 default for the `num_ctx` parameter [33]; the two pages disagree, so set it explicitly.
- A cleanup call is a short system prompt plus up to a few hundred tokens in and the same out. **`num_ctx: 2048`** is ample and keeps KV memory small. Do not go so low that prompt + output can overflow, since overflow triggers context shifting and truncation.
- **Keep `num_ctx` identical on every request, including the warm-up.** Multiple write-ups report that a different `num_ctx` forces a full model reload, visible as a non-zero `load_duration` [34]. This is from secondary sources, not official docs, but it is consistent with the KV cache being allocated at load.
- `num_predict` default is -1 (unbounded) [33]. Set **`num_predict` to about 1.5x the input token count plus a JSON overhead margin**, for example `min(1024, 64 + 2 * input_words)` [E]. It does not speed up a normal response; it bounds the worst case when a small model rambles or loops, which is the tail-latency risk for a background job.
- `num_ctx` multiplies with `OLLAMA_NUM_PARALLEL` (a 2K context with 4 parallel slots allocates 8K) [20].

### 4.3 Structured output (`format`)

`format` accepts `"json"` or a JSON Schema object on both `/api/generate` and `/api/chat` [31][35]. Official guidance: include the schema in the prompt as well, and set temperature to 0 [35].

```json
"format": { "type":"object",
            "properties": { "text": {"type":"string"} },
            "required": ["text"] }
```

Use a schema, not bare `"json"`: grammar-constrained decoding guarantees the shape, removes the retry path, and stops the model inventing extra keys (fewer output tokens). Keep the schema minimal, with one string field. A constant schema is part of the fixed setup, not the prompt, so it does not disturb prefix caching [E]. Ollama cloud models do not support structured outputs [35], which is irrelevant here but a reminder to keep cloud disabled (4.9).

### 4.4 Streaming versus non-streaming

`stream` defaults to `true` [31]. Streaming does not make generation faster; it only lets the client see tokens early. Because the cleanup result is applied atomically after the paste, **`stream: false` is correct today**. Switch to `stream: true` only if you want (a) to cancel early when the output diverges from the input, by dropping the connection, or (b) a progressive UI. Time to first token is `load_duration + prompt_eval_duration` either way [31].

### 4.5 Prompt (KV) cache reuse

- The Ollama runner keeps an input cache per slot and, for each request, finds the slot with the **longest common token prefix**, keeps those KV entries and evaluates only the remainder (`LoadCacheSlot`, `findLongestCacheSlot`, `countCommonPrefix`; it always leaves at least one token to evaluate) [36]. That source is the v0.5.13 llamarunner; the current repository layout has changed and I could not read the present file, but the API now exposes the result directly:
- The response includes **`prompt_eval_cached_count`** ("number of prompt tokens read from the cache"), and `prompt_eval_duration` covers only uncached tokens [31]. Log these two numbers (they contain no transcript content) to verify caching works.
- llama.cpp's server behaves the same way: reuse stops at the first differing token [37].

How to structure the prompt:

1. Put everything constant first: system prompt, rules, few-shot examples, schema description. Byte-identical on every call.
2. Put everything variable last: app/context hints, custom dictionary, then the transcript. Anything that changes per utterance and sits early in the prompt invalidates everything after it.
3. Never put timestamps, counters or a request id in the system prompt.
4. Use `/api/chat` with a fixed `system` message, or `/api/generate` with a fixed `system` field. Both render through the same template so the prefix is stable either way.
5. Keep `OLLAMA_NUM_PARALLEL=1` so there is exactly one slot and the prefix is always in it.
6. The cache lives in the loaded runner. It is lost on unload (keep_alive expiry) and on reload (changed `num_ctx`).

Expected gain: with a 200-300 token system prompt, the prompt-eval portion drops to just the transcript tokens. On the GPU that is tens of milliseconds; on CPU at roughly 50-150 prompt tokens per second for a 3B model it is on the order of 1-4 s saved per call [E].

### 4.6 `/api/chat` versus `/api/generate`

Functionally equivalent for this job; both accept `format`, `think`, `keep_alive`, `options` and return the same timing fields [31]. `/api/generate` offers `raw: true` to bypass the template, which you do not want. Prefer **`/api/chat`**: role separation makes the stable prefix explicit and it is the endpoint new features land on. No measurable latency difference [E].

### 4.7 num_thread, GPU offload, parallelism

- `options.num_thread` and `options.num_gpu` exist in the API reference example (`"num_gpu": 1, "main_gpu": 0, "use_mmap": true, "num_thread": 8`) [38]. On CPU-only machines set `num_thread` to the P-core count, same rule as whisper. If whisper and the LLM can run at the same time (LLM warm-up during recording, cleanup right after ASR), they compete for the same six P-cores, so sequence them [E].
- GPU: Ollama loads a model fully onto one GPU if it fits; `ollama ps` shows the split ("100% GPU") [20]. Requires NVIDIA driver 550 or newer [39]. llama3.2:3b Q4_K_M (2.0 GB) fits the 8 GB card with room for a CUDA whisper model. Do not set `num_gpu` unless you need to force CPU (`0`).
- `OLLAMA_NUM_PARALLEL`: defaults to 1 on non-MLX backends. Leave at 1. `OLLAMA_MAX_LOADED_MODELS` defaults to 3 per GPU; `OLLAMA_MAX_QUEUE` to 512 [20].
- `OLLAMA_FLASH_ATTENTION` is applied automatically where supported; `OLLAMA_KV_CACHE_TYPE` (f16 default, q8_0 halves KV memory) only matters at large contexts [20].
- These are environment variables on the user's Ollama service. An app that does not own the Ollama process cannot rely on them, which is why per-request `options` and `keep_alive` are the dependable levers.

### 4.8 Small cleanup models in the Ollama library (checked 2026-11-22)

| Tag | Size | Context | Notes |
|---|---|---|---|
| `llama3.2:3b` (current) | 2.0 GB | 128K | Text only, no thinking. Two years old but a clean baseline |
| `llama3.2:1b` | 1.3 GB | 128K | Faster; more likely to alter wording |
| `gemma3:1b` | 806 MB | 32K | Text only, no thinking. Good CPU-only candidate |
| `gemma3:4b` | 3.4 GB | 128K | Text + image |
| `gemma3:270m` | 292 MB | 32K | Too small for faithful rewriting [E] |
| `qwen3:1.7b` | 1.4 GB | 40K | Thinking model: send `think:false` |
| `qwen3:4b` | 2.5 GB | 256K | Thinking model |
| `qwen3.5:0.8b` / `2b` / `4b` | 1.2-1.3 / 2.7-3.1 / 3.3-4.0 GB | 256K | Multimodal, thinking |
| `gemma4:e2b` / `e4b` | 4.6-7.5 / 6.6-9.5 GB | 128K | Multimodal (vision, audio), thinking. Heavy for a cleanup job on 8 GB VRAM |
| `gemma3n:e2b` / `e4b` | 7.8 / 9.8 GB | 32K | Listed sizes are far larger than the effective parameter count suggests |
| `phi4-mini:3.8b` | 2.5 GB | 128K | Text only |
| `granite4:micro` (3b) / `350m` | 2.1 GB / 708 MB | 128K / 32K | Tuned for instruction following and tools |
| `lfm2.5-thinking:1.2b` | 731 MB | 125K | Thinking model |
| `lfm2.5:8b` / `lfm2:24b` | 5.2 GB / 14 GB | 125K / 32K | The Ollama library has no small non-thinking LFM2 tag |

Source: ollama.com library tag pages [26].

Guidance:

- **Thinking models need `think: false`.** The docs state that models which cannot disable thinking fall back to their lowest level, and that a model's default is reported by `/api/show` [24]. Verify with `eval_count`: if it is much larger than the visible output, the model is still reasoning.
- Shortlist to A/B against `llama3.2:3b`: `gemma3:1b` (CPU-only tier), `granite4:micro`, `phi4-mini`, `qwen3.5:2b` and `qwen3.5:4b` with thinking off. Judge on (1) word-level fidelity to the transcript, (2) schema-valid JSON first time, (3) tokens/s. I did not find a published benchmark for dictation cleanup; any ranking between these would be a guess.
- Multimodal tags (qwen3.5, gemma4, gemma3:4b) carry vision/audio weights you never use, which costs disk and load time.

### 4.9 Privacy note that affects configuration

Ollama now has cloud models and web search. They can be disabled with `"disable_ollama_cloud": true` in `~/.ollama/server.json` or `OLLAMA_NO_CLOUD=1` [20]. LocalFlow should never request a `:cloud` tag (for example `gemma4:cloud` exists [26]) and should validate the configured model name against that suffix, to honour "no silent network fallback". Ollama binds to `127.0.0.1:11434` by default [20].

---

## 5. Warm starts and model-load strategy

| Strategy | Per-utterance cost | Idle memory | Notes |
|---|---|---|---|
| Cold CLI, cold page cache (first use after boot) | Disk read of the model + load + spawn | 0 | Worst case. tiny.en-q5_1 is 31 MB so even this is short; small.en-q5_1 (181 MB) and turbo (547 MB) are not |
| Cold CLI, warm page cache (today, steady state) | 127-190 ms load + about 100-250 ms spawn/teardown [L] | 0 private; the file stays in the OS standby list | The OS already does the "prewarm" for you after the first run |
| Page-cache prewarm only (read the model file at app start or record start) | Removes only the disk-read part of the first call | 0 private | Cheap, zero risk. Useful as a fallback when the server is not running |
| Persistent `whisper-server` | 0 load, 0 spawn | About 273 MB for unquantised tiny, 388 MB base, 852 MB small [4]; less for quantised files (roughly file size + buffers [E]) | Best latency. Needs supervision |
| Lazy server + idle shutdown | 0 after the first utterance of a session; one load on the first | Zero when idle | Recommended. Start on record-key press, not on transcription, so the load overlaps the user's speech |
| In-process `whisper-rs` context | 0 | Same as server, inside the app process | No crash isolation |
| Ollama `keep_alive: "30m"` | 0 load while warm | Model size plus KV cache (about 2-3 GB RAM or VRAM for a 3B Q4 model [E]) | Already in place. Consider tying it to app activity instead of a flat 30 minutes |

Points to act on:

- **Start work on key-down.** Recording lasts 3-10 s; a tiny or base model loads in well under 1 s. So a lazily started server whose spawn is triggered at record start is "warm" by the time audio arrives, with no idle memory cost at all. This is the same trick the Ollama warm-up already uses. With this, a permanently resident server buys nothing extra for tiny/base and only matters for large models [E].
- **Idle shutdown:** stop the server after N minutes without dictation (5-10 minutes is a reasonable default [E]) and on app exit. Put the child in a Windows Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so it cannot outlive the app if LocalFlow crashes [E].
- **mmap:** whisper.cpp reads the model into its own buffers; I found no mmap option in the CLI or server flags [1][5]. Ollama exposes `use_mmap` in `options` [38]. With mmap, weights are file-backed pages the OS can drop under pressure and reload lazily, which makes "resident" cheap when idle but can cause a first-token stall after long idle; leave Ollama's default alone [E].
- **GPU warm-up:** the first CUDA inference after load pays kernel/cuBLAS initialisation. If a GPU build is added, run one dummy 1 s inference right after `/health` reports ready [E].
- **Model switching:** `/load` swaps models in a running server without a respawn, useful for a "fast draft with tiny, then re-transcribe with small" design [1][2].

---

## 6. Audio path

### 6.1 Avoiding temp-file round trips

Today: samples -> temp WAV on disk -> CLI reads it -> CLI writes JSON to disk -> app reads and deletes both. With the server:

1. Keep the 16 kHz mono samples in memory.
2. Build the WAV in memory: a 44-byte RIFF header plus 16-bit PCM (the `hound` crate can write into a `Cursor<Vec<u8>>`). A 10 s clip is 320 KB.
3. `POST` it as `multipart/form-data` to `http://127.0.0.1:<port>/inference` with field name `file` and a filename ending `.wav`.
4. Parse `{"text": ...}` from the response body.

The server decodes from the request buffer directly when `--convert` is not set [2]. Latency saved is small on an SSD (single-digit to low tens of milliseconds, more with real-time antivirus scanning new files [E]). The stronger reason is **privacy**: dictated audio and transcript never touch disk, which removes a cleanup-on-crash obligation and fits the retention rules in AGENTS.md.

### 6.2 Resampler quality

Mic input usually arrives at 44.1 or 48 kHz and must reach 16 kHz.

- Downsampling without a low-pass filter folds everything above 8 kHz back into the speech band (aliasing). Linear interpolation has no anti-alias filter.
- `rubato` 5.1.0 (2026-11-12) options [40]: `Fft` synchronous resampler for fixed ratios, fast and high quality, intended for exactly this case; `Async::new_sinc` with the same quality but the most CPU, for varying ratios; `Async::new_poly`, fast, and the docs state it "does not provide any anti-alias filtering"; `Slip`, not a real converter.
- **Does it measurably affect WER?** I found **no controlled study** comparing linear and windowed-sinc resampling for Whisper at 48 to 16 kHz. The closest result, a 2026 Indic-language ASR study, reports that classical resamplers (linear, soxr HQ) cause "moderate loss" and does not isolate the two [41]. So the honest answer is: unproven, likely small on clean close-talk speech, plausibly larger with fricatives, keyboard noise or fans, where there is real energy above 8 kHz [E].
- **Recommendation:** if the current path is linear or nearest-neighbour, switch to rubato's FFT resampler (or sinc). Cost is negligible for a 10 s clip (well under 10 ms [E]) and it removes a known signal-processing defect regardless of the WER evidence. Measure by feeding the bench corpus through both paths at 48 kHz.
- Also check: capture in the device's native format and rate (do not ask WASAPI shared mode for 16 kHz and trust its converter), downmix to mono by averaging, and convert to i16 with clipping protection.

### 6.3 Pre-roll buffering

Not a latency optimisation, an accuracy one. Opening a cpal input stream takes tens to hundreds of milliseconds on Windows, so the first syllable after the hotkey is often lost, and Whisper is sensitive to clipped onsets [E].

- Keep the input stream open while the app is armed and write into a ring buffer holding the last **300-500 ms**; on hotkey press, prepend it. Memory cost is trivial (500 ms at 48 kHz mono f32 is 96 KB).
- **Privacy trade-off:** a continuously open microphone shows the Windows mic-in-use indicator permanently and means audio is captured (though discarded) before the user asks. That conflicts with a privacy-first positioning unless it is an explicit, off-by-default setting with the buffer held only in memory and zeroed on overwrite. A middle path: open the stream on hotkey-down as now, and add **150-300 ms of post-roll** after key-up plus a short lead-in of silence, which protects word endings without an always-on mic [E].

---

## 7. Local inference versus hosted APIs (for the report only; cloud is prohibited for LocalFlow)

Assumes a heavy dictation user: 60 minutes of speech per day, 22 days a month = 22 hours of audio per month.

| Option | Unit price | Cost at 22 h/month | Latency profile | Privacy |
|---|---|---|---|---|
| Local whisper.cpp + Ollama | 0 per minute | 0 marginal. Electricity on the order of cents per month [E] | No network; bounded by local hardware | Audio never leaves the device |
| OpenAI `gpt-4o-mini-transcribe` | $1.25 in / $5.00 out per 1M tokens, about $0.003/min [42] | about $3.96 | Network round trip plus inference | Audio sent to a third party |
| OpenAI `gpt-transcribe` | $0.0045/min [42] | about $5.94 | as above | as above |
| OpenAI Whisper / `gpt-4o-transcribe` | $0.006/min [42] | about $7.92 | as above | as above |
| Deepgram Nova-3 (monolingual) | $0.0043/min pre-recorded, $0.0048/min streaming [43] | about $5.68 / $6.34 | Streaming partials | as above |
| Groq `whisper-large-v3-turbo` | about $0.04/hour per secondary sources; I could not load Groq's own pricing table [44] | about $0.88 | Very fast inference | as above |
| Hosted cleanup LLM, e.g. OpenAI `gpt-5-nano` | $0.05 in / $0.40 out per 1M tokens [42] | Well under $1 for dictation volumes [E] | adds a second round trip | Transcript sent to a third party |

Takeaway: hosted speech-to-text is cheap per user (single-digit dollars a month), so the case for local is privacy, offline operation and zero marginal cost at scale, not price. The cost of local is engineering effort and hardware variance.

---

## 8. Ranked optimisations

Target: time from end of speech to pasted text for a 3-10 s utterance, plus background cleanup time. Gains marked [E] are estimates; [L] are from the noisy local runs; [P] are published.

| Rank | Optimisation | Expected gain | Complexity | Regression risk | Evidence |
|---|---|---|---|---|---|
| 1 | **Thread count = P-cores** (`-t 6` on the 165H; `clamp(P-cores, 2, 8)` generally) | Avoids a 2x-3.6x slowdown if the app currently passes a high count; 0 if it already passes 4-8 | Trivial | Very low | [L] t14 3.3 s, t22 6.2 s vs about 1.7 s; [P] Intel, Mozilla [12][13] |
| 2 | **Persistent `whisper-server`**, lazily spawned at record start, idle shutdown, in-memory WAV over loopback multipart | 250-400 ms per utterance, plus no temp files | Medium (process supervision, health poll, fallback to CLI) | Low-medium (decode defaults differ; set `beam_size`, `best_of`, `temperature_inc` explicitly) | [L]; server API [P] [1][2] |
| 3 | **Greedy decoding** (`-bs 1 -bo 1`; server default is already greedy) | 5-25% of ASR time, more on longer utterances [E] | Trivial | Low (slight WER increase on tiny) | CLI default is beam 5 [P] [5]; not cleanly visible in [L] |
| 4 | **Ollama request hygiene**: fixed `num_ctx: 2048`, `num_predict` cap, `think:false`, stable prefix, warm-up with identical options and system prompt, JSON schema in `format`, `temperature: 0` | Cleanup: removes reloads and system-prompt re-evaluation; about 1-4 s on CPU, tens of ms on GPU; bounds the tail [E] | Low | Low | [P] API fields and cache behaviour [20][31][35][36] |
| 5 | **Dynamic `audio_ctx`** per request: `min(1500, ceil(sec*50)+128)`, floor 512, with a repetition guard and WER gate | 2x-3x on encode for short clips | Medium (needs duration, guard, tests) | **High** on stock models | [P] 3x at equal WER [9]; [P] catastrophic WER when too tight [10]; [L] encode 1248 to 508 ms, punctuation changed |
| 6 | **Bound fallback**: `temperature_inc=0.0` (or `-nf` on CLI) plus Silero VAD skip for empty audio | Removes multi-pass latency spikes (up to 5 extra decodes) and silence hallucinations; no change to the median | Low | Medium (no recovery from a bad greedy decode) | [P] [2][5] |
| 7 | **q8_0 instead of q5_1** for tiny/base | 0-15% encode, small accuracy gain; +11-21 MB disk | Trivial (file swap) | Very low | [P] weak: INT8 fastest, INT5 slowest in a 10-file study [17]; otherwise [E] |
| 8 | **Proper resampler** (rubato FFT/sinc) and post-roll | No latency gain; possible WER gain | Low | Low | [P] poly/linear has no anti-alias filter [40]; WER effect unproven |
| 9 | **Bigger or different model made affordable by 1-5**: `base.en`/`small.en`, Parakeet GGUF q4 via the bundled runtime, or Moonshine v2 | Accuracy rather than latency; Moonshine v2 also removes the fixed 30 s encoder cost | Medium-high | Medium | [P] Parakeet 6.05-6.32% avg WER [22]; Moonshine v2 latency table [23] |
| 10 | **GPU ASR** (optional CUDA download or self-built Vulkan) | Large for small/turbo models, negligible for tiny | High (285-685 MB, detection, VRAM contention, fallback) | Medium-high | [P] asset sizes [3], GPU benchmark [15] |
| 11 | **In-process `whisper-rs`** | A few ms over the server (no HTTP) | High (native build chain) | Medium (no crash isolation) | [P] crate facts [8] |
| 12 | OpenVINO / NPU / DirectML / TensorRT / CTranslate2 | Unclear or small at this model size | High | High | See section 3 |

Suggested order of work: 1, 3 and 7 are configuration changes that can be validated in one benchmark session on the existing corpus. Then 2 with 6. Then 4. Then 5 behind a setting, gated on WER. Everything else only after measuring where time goes once those are in.

Measurement plan (all local, no content logged): log per utterance the audio duration, ASR wall time, and from Ollama `load_duration`, `prompt_eval_count`, `prompt_eval_cached_count`, `prompt_eval_duration`, `eval_count`, `eval_duration`. None of these contain dictated text.

---

## 9. Gaps and caveats

- No published cold-start comparison for whisper.cpp tiny models exists that I could find; the cold-start figures are local and noisy.
- The local machine was not idle. Tiny-model encode near 1 s is suspicious. Re-baseline before trusting any local number here.
- The `-nf` no-op in `whisper-server` was read from the `master` source on 2026-11-22, not tested against the 1.9.1 binary in the runtime.
- Ollama's current prompt-cache source could not be read (repository layout changed); the mechanism is cited from v0.5.13 source and corroborated by the current API's `prompt_eval_cached_count` field.
- The `num_ctx` reload behaviour is from secondary sources.
- Groq pricing is from secondary sources.
- ONNX Runtime's GitHub releases page returned 404 to my requests; versions and sizes are from PyPI and are wheel sizes, not bare DLL sizes.
- Whisper tiny.en/base.en/small.en Open ASR Leaderboard WERs were not retrievable from a primary source today, so they are not quoted.
- No dictation-cleanup benchmark exists for the small LLMs listed; the shortlist is a set of candidates, not a ranking.

---

## Sources

1. whisper.cpp server README: https://github.com/ggml-org/whisper.cpp/blob/master/examples/server/README.md
2. whisper.cpp `server.cpp` (request fields, defaults, endpoints, locking): https://github.com/ggml-org/whisper.cpp/blob/master/examples/server/server.cpp
3. whisper.cpp releases (v1.9.5, b5454, b5907 assets and changelogs): https://github.com/ggml-org/whisper.cpp/releases
4. whisper.cpp README (memory table, quantisation, OpenVINO, Vulkan, VAD): https://github.com/ggml-org/whisper.cpp/blob/master/README.md
5. whisper.cpp CLI README (flags, defaults, hallucination guidance): https://github.com/ggml-org/whisper.cpp/blob/master/examples/cli/README.md
6. whisper.cpp parakeet-cli README: https://github.com/ggml-org/whisper.cpp/blob/master/examples/parakeet-cli/README.md
7. Parakeet GGUF files: https://huggingface.co/ggml-org/parakeet-GGUF/tree/main
8. whisper-rs 0.16.0: https://docs.rs/crate/whisper-rs/latest and https://codeberg.org/tazz4843/whisper-rs
9. whisper.cpp issue 1855, audio_ctx speed-up measurements: https://github.com/ggml-org/whisper.cpp/issues/1855
10. FUTO whisper-acft: https://github.com/futo-org/whisper-acft
11. whisper.cpp bench README (mirror): https://sources.debian.org/src/whisper.cpp/1.8.3%2Bdfsg-2/examples/bench/README.md
12. Mozilla, best hardware concurrency for CPU inference: https://blog.mozilla.org/en/firefox/firefox-ai/what-is-the-best-hardware-concurrency-for-running-inference-on-cpu/
13. Intel oneMKL, managing performance with heterogeneous cores: https://www.intel.com/content/www/us/en/docs/onemkl/developer-guide-windows/2025-2/managing-performance-with-heterogeneous-cores.html
14. llama.cpp discussion 572, P-cores only: https://github.com/ggerganov/llama.cpp/discussions/572
15. faster-whisper README benchmarks: https://github.com/SYSTRAN/faster-whisper
16. ggml Whisper model files: https://huggingface.co/ggerganov/whisper.cpp/tree/main and Silero VAD: https://huggingface.co/ggml-org/whisper-vad/tree/main
17. Quantization for OpenAI's Whisper Models: A Comparative Analysis: https://arxiv.org/html/2503.09905v1
18. whisper.cpp discussion 3752, legacy-CPU quantisation benchmark: https://github.com/ggml-org/whisper.cpp/discussions/3752
19. Package versions and sizes: https://pypi.org/project/onnxruntime/ , https://pypi.org/project/onnxruntime-gpu/ , https://pypi.org/project/onnxruntime-directml/ , https://pypi.org/project/openvino/ , https://pypi.org/project/openvino-genai/ , https://pypi.org/project/ctranslate2/ , https://pypi.org/project/faster-whisper/ , https://pypi.org/project/tensorrt-cu12-libs/ , https://github.com/k2-fsa/sherpa-onnx/releases , https://crates.io/crates/ort , https://crates.io/crates/sherpa-onnx , https://crates.io/crates/ct2rs , https://crates.io/crates/openvino
20. Ollama FAQ: https://docs.ollama.com/faq
21. sherpa-onnx NeMo transducer models (Parakeet int8 sizes and RTF): https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html
22. Open ASR Leaderboard paper: https://arxiv.org/html/2510.06961v3
23. Moonshine v2 paper: https://arxiv.org/html/2602.12241
24. Ollama thinking: https://docs.ollama.com/capabilities/thinking ; 24a. Moonshine repository: https://github.com/moonshine-ai/moonshine
25. llama.cpp perplexity README (mirror): https://sources.debian.org/src/llama.cpp/7965%2Bdfsg-1/tools/perplexity/README.md
26. Ollama library tag pages: https://ollama.com/library/llama3.2/tags , https://ollama.com/library/gemma3/tags , https://ollama.com/library/gemma3n/tags , https://ollama.com/library/gemma4/tags , https://ollama.com/library/qwen3/tags , https://ollama.com/library/qwen3.5/tags , https://ollama.com/library/phi4-mini/tags , https://ollama.com/library/granite4/tags , https://ollama.com/library/lfm2/tags , https://ollama.com/library/lfm2.5/tags , https://ollama.com/library/lfm2.5-thinking/tags
27. Windows ML / DirectML status: https://github.com/microsoft/WindowsML and https://learn.microsoft.com/windows/ai/new-windows-ml/overview
28. OpenVINO GenAI speech recognition: https://openvinotoolkit.github.io/openvino.genai/docs/use-cases/speech-recognition
29. OpenVINO GenAI on NPU: https://docs.openvino.ai/2026/openvino-workflow-generative/inference-with-genai/inference-with-genai-on-npu.html
30. Ollama releases: https://github.com/ollama/ollama/releases
31. Ollama API, generate and chat: https://docs.ollama.com/api/generate and https://docs.ollama.com/api/chat
32. Ollama context length: https://docs.ollama.com/context-length
33. Ollama Modelfile reference: https://docs.ollama.com/modelfile
34. num_ctx reload reports (secondary): https://dev.to/pat9000/pin-your-context-window-or-pay-the-reload-tax-3nfo
35. Ollama structured outputs: https://docs.ollama.com/capabilities/structured-outputs
36. Ollama runner input cache source (v0.5.13): https://cdn.jsdelivr.net/gh/ollama/ollama@0.5.13/runner/llamarunner/cache.go
37. llama.cpp KV cache reuse tutorial: https://github.com/ggml-org/llama.cpp/discussions/13606
38. Ollama API reference (options example incl. num_thread, num_gpu, use_mmap): https://github.com/ollama/ollama/blob/main/docs/api.md
39. Ollama hardware support: https://docs.ollama.com/gpu
40. rubato 5.1.0: https://docs.rs/rubato/latest/rubato/
41. Factors affecting ASR performance (resampling, secondary evidence): https://arxiv.org/pdf/2606.09335
42. OpenAI API pricing: https://developers.openai.com/api/docs/pricing
43. Deepgram pricing: https://deepgram.com/pricing
44. Groq pricing (secondary): https://convertaudiototext.com/blog/speech-to-text-api-pricing-2026
