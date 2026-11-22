# State of text-to-speech models for LocalFlow (as of 2026-11-22)

> Research appendix produced on 2026-11-22 by an independent research agent for the voice pipeline review. It is kept as written, apart from removing machine-specific paths. Line references to `native_dictation.rs` describe the code before the changes in `../VOICE_PIPELINE_REVIEW.md`. Figures are labelled by the author as vendor, third-party, reproduced, or unverified; treat unlabelled numbers as unverified.

Scope: an optional, fully offline "read back my dictation / read selected text" feature for a Windows Tauri 2 / Rust app. Must run CPU-only on a 16-core laptop, with permissive licensing. Hosted APIs are listed for comparison only; they are incompatible with LocalFlow's privacy rules.

## How to read this report

Evidence labels used throughout:

- **[primary]**: read today from the official repo, model card, licence file, docs page, or the GitHub / Hugging Face / crates.io API.
- **[vendor claim]**: a number published by the model's own authors; not reproduced.
- **[third-party]**: measured by someone other than the vendor.
- **[secondary]**: taken from a search summary or news article, not the primary page.
- **[unverified]**: I could not confirm it against a primary source in this session.

No benchmark was run on LocalFlow's target hardware. Every speed figure below is someone else's number on someone else's machine.

## Key findings

1. **Fish Speech is not a candidate.** The current open models are Drama 3 (35B MoE, 3B active, about 41.5 GB of weights) and S2-Pro (4B). Both need a GPU-class server, and the licence is not permissive: the Fish Audio Community License (v1.0, 2026-10-13) allows commercial use only under USD 500k annual revenue, requires "Built with Fish Audio" attribution, and the S2-Pro weights stay under a research-only licence [1][2][3][4].
2. **Three vendors of small permissive models stopped or reduced maintenance in 2026.** Supertonic was archived (repo moved to `supertone-oss-archive`, "development and support have ended") [9]. Chatterbox was declared "retired research ... no longer maintained" as of August 2026 [12]. Piper's maintained fork is advertising for maintainers [16]. Weights and licences are unchanged, but fixes are unlikely.
3. **The practical integration route is sherpa-onnx.** It ships an official Rust crate (`sherpa-onnx` 2.0.3, published 2026-11-22, static linking by default, Apache-2.0) with TTS examples for Kokoro, Kitten, Piper/VITS, Matcha, Pocket TTS, Supertonic and ZipVoice [19][20]. Since v2.0.0 (2026-10-23) espeak-ng and piper-phonemize are no longer built in, so the library is GPL-free; English uses a built-in lexicon front end [21].
4. **Best-fit models:** Kokoro-82M (Apache-2.0 code and weights), Pocket TTS (MIT code, CC-BY-4.0 weights, official Rust CPU binary for Windows, streaming), and the new Kitten TTS 2 mini / nano / pico (Apache-2.0, ONNX int8/int4, published 2026-11-13). Piper voices run through sherpa-onnx are the smallest fallback.
5. **Hosted APIs moved on:** ElevenLabs v4 / v4 Turbo, Cartesia Sonic 3.6, and OpenAI has scheduled `gpt-4o-mini-tts`, `tts-1` and `tts-1-hd` for shutdown on 2027-01-06 [33][35][36].

## Comparison table: local open-weight models

Sizes are parameters unless a file size is given. "Clone" means zero-shot voice cloning from a reference clip.

| Model (latest) | Released | Params / size | Code licence | Weights licence | Languages | Clone | CPU-realistic? | Rust / ONNX path | Maintenance |
|---|---|---|---|---|---|---|---|---|---|
| **Kokoro-82M v1.0** | 2025-01-27 | 82M; ONNX 86-326 MB | Apache-2.0 | Apache-2.0 | 8 (EN US/UK, JA, ZH, ES, FR, HI, IT, PT-BR) | No (54 preset voices) | Yes | sherpa-onnx, `ort`, kokoro-onnx, Kokoros | Dormant: last repo push 2025-08 |
| **Pocket TTS v4.0.1** | 2026-10-21 (first: 2026-01-13) | 100M; English variants 18.2M / 6.9M / 1.9M | MIT | CC-BY-4.0 | EN, FR, DE, PT, IT, ES, NL | Yes | Yes (designed for CPU) | Official Rust binary (Windows x86), sherpa-onnx, ONNX and Candle ports | Active (push 2026-11-18) |
| **Kitten TTS 2 mini / nano / pico** | 2026-11-13 | 82M / 17M / 7M; 83 / 17 / 8 MB int8 | Apache-2.0 | Apache-2.0 | 16 / 8 / 8 | No | Yes | ONNX Runtime (`ort`); sherpa-onnx bundles only v0.8 so far | Active (push 2026-11-19) |
| Kitten TTS 2 (1.7B "Ternary") | 2026-09-30 | 1.7B; 492 MB | Apache-2.0 | Stellon Labs Community License (**restricted**) | 20 | Yes | Claimed; ~6 GB RAM | llama.cpp fork (beta) | Active |
| **Piper (piper1-gpl v1.8.0)** | 2026-09-04 | VITS; per-voice size [unverified] | **GPL-3.0** | Per voice; varies, some non-commercial | 35 language tags on the voices repo | No | Yes | sherpa-onnx runs Piper VITS voices without the GPL engine | "Looking for maintainers" |
| **Supertonic 3** | 2026-04-29 | 99M | MIT (samples) | OpenRAIL-M (use restrictions) | 31 | Not in open weights | Yes | Own Rust example, sherpa-onnx | **Archived 2026-09** |
| **Chatterbox Nano** | 2026-07 | 110M | MIT | MIT | English | Yes | Claimed 3x real time on 8 cores | PyTorch; ONNX [unverified] | **Retired 2026-08** |
| Chatterbox Turbo / Multilingual V3 | 2025-12 / 2026-06 | 350M / 500M | MIT | MIT | EN / 23 | Yes | Marginal | PyTorch | Retired 2026-08 |
| NeuTTS Air / Nano / 2E | 2025-09 / 2025-11 / 2026-07 | ~552M / ~229M / ~236M total | Custom, see note | Apache-2.0 (Air); NeuTTS Open License 1.0 (Nano, 2E) | EN (+FR, DE, ES Nano variants) | Yes (2E: 4 fixed speakers) | Yes, via GGUF | llama.cpp + ONNX codec decoder | Last push 2026-07 |
| MOSS-TTS-Nano | 2026-04-13 | ~100M | Apache-2.0 | [unverified] | Multilingual | Yes | Yes ("4 CPU cores") | Standalone ONNX CPU build | Last push 2026-09 |
| **Fish Audio Drama 3** | 2026-10-13 (pre-release) | 35B MoE, 3B active; 41.5 GB (22.1 GB community int4) | Fish Audio Community License (**restricted**) | Same (see note) | "About a hundred" (S2 line) | Yes | **No** | None; SGLang-Omni / vLLM servers | Active (push 2026-11-14) |
| Fish Audio S2-Pro | 2026-03-10 (beta) | 4B | Community License | Fish Audio Research License (**non-commercial**) | ~100 | Yes | No | None | Superseded |
| Fish S1-mini / Fish Speech 1.5 | 2025-05 / 2024-11 | [unverified] | Earlier restrictive licences | **CC-BY-NC-SA-4.0** | 13 (1.5) | Yes | No | None | Legacy |
| F5-TTS v1 Base | 2025-03-12 (pkg 1.1.22, 2026-07-23) | [unverified] | MIT | **CC-BY-NC-4.0** | EN, ZH (+community) | Yes | No | Community ONNX export | Active |
| XTTS-v2 (idiap coqui-tts 0.28.0) | Model 2023-11; fork 2026-10-19 | [unverified] | MPL-2.0 | **Coqui Public Model License (non-commercial)** | 17 | Yes | Slow on CPU [unverified] | Python only | Fork active; original repo unmaintained |
| OpenVoice V2 | 2024-04 | [unverified] | MIT | MIT | EN, ES, FR, ZH, JA, KO | Yes (tone-colour converter) | [unverified] | Python | Dormant since 2025-04 |
| Fun-CosyVoice 3.0 (0.5B-2512) | 2025-12 | 0.5B | Apache-2.0 | Apache-2.0 | 9 + 18 Chinese dialects | Yes | No | Python / vLLM | Last push 2026-05 |
| StyleTTS 2 | 2023-11 | [unverified] | MIT (inference needs GPL phonemiser) | Custom disclosure condition | English | Yes | [unverified] | None official | Dormant since 2024-08 |
| IndexTTS-2.5 | 2026-08-10 | [unverified] | bilibili Model Use License (**restricted**) | Same | ZH, EN, JA, ES, AR | Yes | No | Python / vLLM | Active |
| VibeVoice Realtime 0.5B | 2025-12-03 | 0.5B | MIT | MIT | EN (+9 experimental) | [unverified] | No | Python | TTS-1.5B listed as "Disabled" |
| Higgs Audio v3 TTS | 2026-06 | 4B | Research / Non-Commercial (**restricted**) | Same | 100+ [vendor claim] | Yes | No | SGLang-Omni | Active |
| Qwen3-TTS | 2026-01-22 | 0.6B / 1.7B | Apache-2.0 | Apache-2.0 | 10 | Yes | No | Python | Last push 2026-03 |
| VoxCPM2 | 2026-04 | 2B | Apache-2.0 | Apache-2.0 | 30 | Yes | No | Python, community GGUF | Active |
| OmniVoice (k2-fsa) | 2026-03-31 | [unverified] | Apache-2.0 | [unverified] | 600+ | Yes | [unverified] | Python | Active |
| Voxtral TTS (Mistral) | 2026-03 | 4B (per repo name) | n/a | **CC-BY-NC-4.0** | 9 | [unverified] | No | n/a | n/a |
| Orpheus 3B / Sesame CSM-1B / Dia2 / Zonos v0.1 / Kyutai TTS 1.6B | 2025 | 3B / 1B / 1-2B / [unverified] / 1.6B | Apache-2.0 | Apache-2.0 (Kyutai: CC-BY-4.0) | Mostly English | Varies | No | None | Little or no activity since 2025 |

## Comparison table: hosted APIs (not usable in LocalFlow)

| Service | Current model | Date | Languages | Latency | Notes |
|---|---|---|---|---|---|
| ElevenLabs | Eleven v4, Eleven v4 Turbo (`eleven_v4`, `eleven_v4_turbo`) | 2026-09-28 [secondary] | 90+ | v4 Turbo ~100 ms median model latency [vendor claim] | v3 is now "previous generation"; Turbo v2 / v2.5 deprecated [33][34] |
| Cartesia | Sonic 3.6 (`sonic-3.6-2026-08-27`) | 2026-08-27 | 44 | "Sub-90 ms" [vendor claim, secondary]; 128 ms median including network for Sonic 3.5 [third-party, Vapi, secondary] | Instant voice cloning [35] |
| OpenAI | `gpt-4o-mini-tts`, `tts-1`, `tts-1-hd` | n/a | ~60 (follows Whisper) | Not published | All three shut down 2027-01-06; OpenAI points to `gpt-realtime-2.1-mini` on the Realtime API [36] |
| Fish Audio | S2.1 Pro, Drama 3 Preview | 2026-06-23 / 2026-09-23 [secondary] | ~100 | ~100 ms to first audio on H200 for S2-Pro [vendor claim] | S1 API retires 2026-12-31 [secondary] |

## Fish Speech versus the strongest alternatives

Fish Audio leads on expressiveness and control. Its own benchmarks put S2 at 0.99% English WER on Seed-TTS Eval, with Drama 3 reported above S2 [vendor claim] [1]. Those results come from 4B and 35B-parameter models served on NVIDIA H200 GPUs (S2-Pro: RTF 0.195 and about 100 ms to first audio on one H200 [vendor claim]).

For LocalFlow the comparison is one-sided on every constraint that matters:

| Constraint | Fish (Drama 3 / S2-Pro) | Kokoro-82M | Pocket TTS | Kitten TTS 2 mini |
|---|---|---|---|---|
| Download size | 41.5 GB / multi-GB | 86-326 MB | 100M params (file size not checked) | 47-83 MB |
| CPU-only | No | Yes | Yes | Yes |
| Fits an 8 GB laptop GPU | Drama 3 no; S2-Pro [unverified] | Not needed | Not needed | Not needed |
| Commercial use | Under USD 500k revenue only, with attribution; S2-Pro none | Unrestricted | Unrestricted, with attribution | Unrestricted |
| Rust / ONNX runtime | None | Yes | Yes | Yes (ONNX) |
| Voice cloning, emotion tags | Yes | No | Cloning only | No |

A read-back feature needs intelligible, pleasant, fast speech from preset voices. It does not need cloning, multi-speaker scenes or emotion direction, which is where Fish's advantage lies. Voice cloning is also a liability for a privacy-first product.

Among the larger open models, the permissively licensed ones closest to Fish in capability are VoxCPM2, Qwen3-TTS and Fun-CosyVoice 3 (all Apache-2.0 for code and weights). They still need a GPU and a Python stack, so they do not fit either.

## Per-model notes

### Kokoro-82M
- v1.0, 2025-01-27; StyleTTS 2-derived (the model card lists `yl4579/StyleTTS2-LJSpeech` as base model); 82M parameters; 8 languages, 54 voices [primary] [5][6].
- Apache-2.0 for both code and weights [primary]. No successor exists: hexgrad's newest model is Kokoro-82M-v1.1-zh (2025-02-27), and the GitHub repo was last pushed 2025-08-06 [primary].
- ONNX files: 325.5 MB fp32, 163.2 MB fp16, 92.4 MB int8, 86.0 MB q8f16 [primary] [7].
- Speed: mean RTF 0.64 (ONNX fp32) on a 4-core Xeon Platinum 8272CL, UTMOS 4.44 [third-party, a small unreviewed repo] [8]. "Near real-time on macOS M1" per kokoro-onnx [vendor claim]. No published time-to-first-audio; the model is not streaming, so latency is managed by synthesising sentence by sentence.
- Phonemisation: the reference Python pipeline uses misaki with an espeak-ng fallback (GPL). The sherpa-onnx lexicon front end avoids espeak for English and Chinese [primary] [20][21]. Other languages need an external phonemiser.
- No cloning, no emotion control.

### Pocket TTS (Kyutai)
- First released 2026-01-13; current v4.0.1 (2026-10-21). 100M parameters, plus small English models at 18.2M, 6.9M and 1.9M selectable since October 2026 [primary] [10].
- Code MIT, weights CC-BY-4.0 (attribution required) [primary]. The cloning-capable repo `kyutai/pocket-tts` is gated on Hugging Face; `kyutai/pocket-tts-without-voice-cloning` is not [primary]. I did not read the gate terms [unverified]. Preset voices carry their own licences; the Expresso-derived voices are CC-BY-NC [primary] [11].
- Vendor figures: about 200 ms to the first audio chunk and about 6x real time on a MacBook Air M4 using 2 cores; on a 4-vCPU cloud VM they report 2.3-2.5x real time on CPU [vendor claim].
- Third-party: mean RTF 0.71 (PyTorch, 4-core Xeon), UTMOS 4.10 [8].
- Each release includes a standalone `pocket-tts` Rust binary for Windows x86 (AVX2, with an SSE4.2 variant) that needs neither Python nor PyTorch. sherpa-onnx, a single-file C++ ONNX runtime and a Candle port also exist [primary] [10].
- Streaming audio output is native. Cloning from a WAV file is supported. No emotion control.

### Kitten TTS (KittenML / Stellon Labs)
- Kitten TTS 2 mini / nano / pico were published 2026-11-13: 82M / 17M / 7M parameters, 16 / 8 / 8 languages, 24 kHz, ONNX int8 and int4, Apache-2.0 weights [primary] [13][14].
- English text uses a lexicon plus a small BART grapheme-to-phoneme model; other languages use espeak-ng, which is GPL [primary] [15].
- The vendor claims pico runs faster than real time on one smartphone CPU thread with 40-50 MB peak memory. I found no independent measurement, and the models are nine days old.
- The 1.7B "Kitten TTS 2" flagship is under the Stellon Labs Community License: commercial use requires registration, ends above USD 1M in revenue or funding, and requires "Powered by KittenML" [primary] [13]. Avoid it.
- The older v0.8 models (15M-80M, Apache-2.0, English) are bundled by sherpa-onnx [primary].

### Piper
- `rhasspy/piper` (MIT) is archived [primary]. Development continues at `OHF-Voice/piper1-gpl`, v1.8.0 (2026-09-04), **GPL-3.0**, embedding espeak-ng [primary] [16].
- The project grants no licence over voices; each depends on its training data. A commercial-use list is maintained in the repo. For example `en_US-hfc_female-medium` is CC-BY-NC-SA, and `en_US-amy-medium` has undocumented recordings [primary] [17].
- sherpa-onnx distributes 536 converted Piper VITS voices and can run English ones with its lexicon front end, with no GPL code linked [primary] [19][20].
- No cloning, no emotion control. RTF figures were not verified in this session.

### Supertonic (Supertone)
- Supertonic 3, 2026-04-29: 99M parameters, 31 languages, 44.1 kHz, 10 inline expression tags, ONNX Runtime only [primary] [9].
- Sample code MIT; weights under BigScience OpenRAIL-M, which carries use-based restrictions that must be passed downstream [primary].
- **Archived**: "Development and support have ended ... No updates, bug fixes, security patches" [primary]. Weights moved to the `supertone-oss-archive` namespace.
- Speed: mean RTF 0.24 at the default 5 steps (UTMOS 4.32), and 0.12 at 2 steps with UTMOS 1.53, which is not usable quality; 4-core Xeon [third-party] [8].
- The open weights have no voice cloning.

### Chatterbox (Resemble AI)
- Family: Chatterbox (500M, EN), Multilingual V3 (500M, 23 languages, 2026-06), Turbo (350M, EN, 2025-12), Nano (110M, EN, 2026-07). MIT for code and weights [primary] [12].
- Nano: "3x faster than realtime on 8 CPU cores" [vendor claim]. Paralinguistic tags such as `[laugh]`. Zero-shot cloning.
- All output carries the PerTh neural watermark [primary].
- Retired as of August 2026; the README says Resemble no longer sells voice products [primary]. PyTorch-based; I did not find an official ONNX path [unverified].

### NeuTTS (Neuphonic)
- Air (~360M active), Nano (~120M), 2E (~125M, six emotions, four fixed speakers). GGUF quantisations run on llama.cpp with an ONNX codec decoder [primary] [18].
- Air weights are Apache-2.0; Nano and 2E are under "NeuTTS Open License 1.0", whose terms I did not read [unverified]. Hugging Face metadata for `neutts-2e` says apache-2.0, which conflicts with the README table. Repos are gated. Outputs are watermarked.
- Vendor benchmark: 119 tokens/s (Air) and 221 tokens/s (Nano) on a Ryzen 9 HX 370, backbone only, codec excluded.

### Fish Speech / Fish Audio
- Lineage: Fish Speech 1.5 (2024-12) → S1-mini / v1.5.1 (2025-05) → S2-Pro (beta 2026-03-10) → Drama 3 (pre-release 2026-10-13) [primary] [1][2]. Hosted-only S2.1 Pro (2026-06-23) [secondary] [22].
- Drama 3: 35B MoE with 3B active per token, RVQ codec, inline tags, multi-speaker scenes, infilling. Under 100 ms to first audio for a single stream on H200 [vendor claim] [3].
- Licence history from the LICENSE commit log: CC-BY-NC-SA (2024-05) → Fish Audio Research License (2026-03) → Fish Audio Community License v1.0 (2026-10-13) [primary] [4].
- The Drama 3 model card's metadata still says `fish-audio-research-license` while its text says Community License [primary]. Treat as unresolved.
- S1-mini weights: CC-BY-NC-SA-4.0, gated [primary]. Details of the full OpenAudio S1 were not checked [unverified].

### F5-TTS
- v1 Base released 2025-03-12; release 1.1.22 on 2026-07-23 [primary] [23].
- Code MIT; pretrained weights **CC-BY-NC** because of the Emilia training data [primary]. Not usable commercially.

### XTTS-v2 and Coqui forks
- Model published 2023-11; weights under the Coqui Public Model License [primary for the licence tag] [24]. Its non-commercial scope and Coqui's shutdown are from prior knowledge, not re-read today [unverified].
- `idiap/coqui-ai-TTS` is the maintained fork (v0.28.0, 2026-10-19, MPL-2.0, PyPI `coqui-tts`) and describes the original repo as unmaintained [primary] [25]. 17 languages; "<200 ms" streaming latency [vendor claim].

### OpenVoice V2, CosyVoice, StyleTTS 2
- OpenVoice V2: April 2024, MIT, six native languages, tone-colour converter; repo untouched since 2025-04 [primary] [26].
- Fun-CosyVoice 3.0 (0.5B, 2025-12): Apache-2.0, LLM-based, bi-streaming with latency "as low as 150 ms" [vendor claim], instruct control for emotion, speed and dialect [primary] [27]. GPU and Python.
- StyleTTS 2: MIT code, but inference depends on a GPL-licensed package, and the pretrained weights require telling listeners the speech is synthetic unless you have the speaker's permission [primary] [28]. Kokoro is its practical successor.

### Others checked briefly
- **MOSS-TTS-Nano** (2026-04-13): ~100M, multilingual cloning, 48 kHz, standalone ONNX CPU build; code Apache-2.0 [primary] [29]. Worth a second look; weights licence not confirmed.
- **VoxCPM2** (2026-04): 2B, 30 languages, 48 kHz, Apache-2.0; RTF ~0.3 on an RTX 4090 [vendor claim] [30].
- **Qwen3-TTS** (2026-01-22): 0.6B / 1.7B, 10 languages, Apache-2.0, streaming [primary] [31].
- **Higgs Audio v3** (4B): research / non-commercial licence [primary] [32]. **IndexTTS-2.5**: bilibili custom licence [primary]. **Voxtral TTS**: CC-BY-NC-4.0 [primary].
- **VibeVoice**: the README lists TTS-1.5B as "Disabled" after reported misuse; Realtime-0.5B remains under MIT [primary].
- **Orpheus, Sesame CSM, Dia / Dia2, Zonos, Kyutai TTS 1.6B**: Apache-2.0 (Kyutai weights CC-BY-4.0), GPU-oriented, with little or no repo activity since 2025 [primary for licence and dates].

## Recommendation for LocalFlow

**Runtime:** use the `sherpa-onnx` Rust crate behind a typed `TtsProvider` trait, in-process or as a small sidecar. One Apache-2.0 dependency covers Kokoro, Kitten v0.8, Piper voices, Pocket TTS and Supertonic, so the model can be swapped without touching workflow code. Models are plain files that can live in `.localflow-runtime/`.

**Shortlist, in order:**

1. **Kokoro-82M v1.0 (int8, ~90 MB)** as the default. Fully Apache-2.0, widely deployed, and the best measured quality among the small models. Risks: upstream is dormant, and it is English-only without an external phonemiser.
2. **Pocket TTS** if low time-to-first-audio or streaming matters. Use the non-cloning weights. It needs CC-BY-4.0 attribution and care over which preset voices are bundled.
3. **Kitten TTS 2 nano or mini** as the small-footprint option, once it has been listened to and timed locally. It is too new to trust on vendor claims alone, and it needs direct ONNX Runtime integration until sherpa-onnx adds it.
4. **A Piper voice via sherpa-onnx** as a minimal fallback, chosen from Piper's commercial-use list.

**Use with caution:** Supertonic 3 (fast and good, but archived and OpenRAIL-M) and Chatterbox Nano (MIT, but retired, PyTorch-based and watermarked).

**Rule out:** Fish Speech (all generations), F5-TTS, XTTS-v2, Kitten TTS 2 1.7B, IndexTTS, Higgs v3 and Voxtral TTS on licence; every 0.5B+ LLM-style model on size; every hosted API on privacy.

**Before committing:** time Kokoro int8, Pocket TTS and Kitten 2 on the actual 16-core laptop (RTF, time to first audio, peak memory). None of the published numbers were taken on comparable hardware.

## Unverified items

- Real-time factors and memory use on LocalFlow's target hardware (nothing was run).
- Parameter counts for F5-TTS, XTTS-v2, OpenVoice V2, StyleTTS 2, IndexTTS-2.5, OmniVoice, Zonos, Piper voices and S1-mini.
- OpenAudio S1 (full model) details and whether it was ever open-weight.
- Terms of the Pocket TTS Hugging Face gate, the NeuTTS Open License 1.0, the Coqui Public Model License text, and the weights licences of MOSS-TTS-Nano and OmniVoice.
- Which licence actually governs the Drama 3 weights (card metadata and card text disagree).
- ElevenLabs v4 launch date (2026-09-28 per a TechCrunch URL and a search summary; the official blog was not opened) and Cartesia's "sub-90 ms" figure (secondary sources only).
- Fish Audio API changelog dates for S2.1 Pro, Drama 3 Preview and S1 retirement (search summary only).
- VRAM requirements for every GPU-class model.
- Whether Chatterbox Nano has a working ONNX export.

## Sources

1. Fish Speech README: https://github.com/fishaudio/fish-speech
2. Fish Speech releases: https://github.com/fishaudio/fish-speech/releases
3. Drama 3 model card: https://huggingface.co/fishaudio/drama-3 (community int4: https://huggingface.co/fishaudio/drama-3-int4)
4. Fish Audio Community License: https://github.com/fishaudio/fish-speech/blob/main/LICENSE ; S2-Pro licence: https://huggingface.co/fishaudio/s2-pro/blob/main/LICENSE.md ; S2 report: https://arxiv.org/abs/2603.08823 ; Drama 3 report: https://arxiv.org/abs/2610.18010
5. Kokoro-82M model card: https://huggingface.co/hexgrad/Kokoro-82M (voices: https://huggingface.co/hexgrad/Kokoro-82M/blob/main/VOICES.md)
6. Kokoro repo: https://github.com/hexgrad/kokoro
7. Kokoro ONNX export: https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX ; kokoro-onnx: https://github.com/thewh1teagle/kokoro-onnx ; Kokoros (Rust): https://github.com/lucasjinreal/Kokoros
8. Third-party CPU benchmark: https://github.com/gauravvij/kokoro-tts-vs-supertonic-3-tts/blob/main/results/benchmark_report.md
9. Supertonic (archived): https://github.com/supertone-oss-archive/supertonic ; weights: https://huggingface.co/supertone-oss-archive/supertonic-3
10. Pocket TTS repo: https://github.com/kyutai-labs/pocket-tts ; blog: https://kyutai.org/blog/2026-01-13-pocket-tts
11. Pocket TTS weights: https://huggingface.co/kyutai/pocket-tts ; https://huggingface.co/kyutai/pocket-tts-without-voice-cloning ; voices: https://huggingface.co/kyutai/tts-voices
12. Chatterbox repo: https://github.com/resemble-ai/chatterbox ; Nano: https://huggingface.co/ResembleAI/chatterbox-nano ; Turbo: https://huggingface.co/ResembleAI/chatterbox-turbo
13. KittenTTS repo: https://github.com/KittenML/KittenTTS ; Stellon licence: https://huggingface.co/KittenML/kitten-tts-2/blob/main/LICENSE.md
14. Kitten TTS 2 mini: https://huggingface.co/KittenML/kitten-tts-2-mini ; nano: https://huggingface.co/KittenML/kitten-tts-2-nano ; pico: https://huggingface.co/KittenML/kitten-tts-2-pico
15. Kitten TTS 2 small-model docs: https://github.com/KittenML/KittenTTS/blob/main/docs/kitten-tts-2-tiny.md
16. Piper (maintained): https://github.com/OHF-Voice/piper1-gpl ; original: https://github.com/rhasspy/piper
17. Piper commercial-use list: https://github.com/OHF-Voice/piper1-gpl/blob/main/docs/COMMERCIAL_USE.md ; voices: https://huggingface.co/rhasspy/piper-voices
18. NeuTTS: https://github.com/neuphonic/neutts
19. sherpa-onnx: https://github.com/k2-fsa/sherpa-onnx ; TTS models: https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models
20. sherpa-onnx Rust examples: https://github.com/k2-fsa/sherpa-onnx/tree/master/rust-api-examples ; crate: https://crates.io/crates/sherpa-onnx
21. sherpa-onnx PR removing espeak-ng: https://github.com/k2-fsa/sherpa-onnx/pull/4055
22. Fish Audio changelog: https://docs.fish.audio/changelog ; S1-mini: https://huggingface.co/fishaudio/s1-mini ; Fish Speech 1.5: https://huggingface.co/fishaudio/fish-speech-1.5
23. F5-TTS: https://github.com/SWivid/F5-TTS ; weights: https://huggingface.co/SWivid/F5-TTS
24. XTTS-v2: https://huggingface.co/coqui/XTTS-v2
25. idiap coqui-ai-TTS: https://github.com/idiap/coqui-ai-TTS
26. OpenVoice: https://github.com/myshell-ai/OpenVoice ; https://huggingface.co/myshell-ai/OpenVoiceV2
27. CosyVoice: https://github.com/FunAudioLLM/CosyVoice ; https://huggingface.co/FunAudioLLM/Fun-CosyVoice3-0.5B-2512
28. StyleTTS 2: https://github.com/yl4579/StyleTTS2
29. MOSS-TTS: https://github.com/OpenMOSS/MOSS-TTS ; Nano: https://github.com/OpenMOSS/MOSS-TTS-Nano
30. VoxCPM: https://github.com/OpenBMB/VoxCPM
31. Qwen3-TTS: https://github.com/QwenLM/Qwen3-TTS
32. Higgs Audio: https://github.com/boson-ai/higgs-audio ; IndexTTS: https://github.com/index-tts/index-tts ; VibeVoice: https://github.com/microsoft/VibeVoice ; Voxtral TTS: https://huggingface.co/mistralai/Voxtral-4B-TTS-2603 ; OmniVoice: https://github.com/k2-fsa/OmniVoice
33. ElevenLabs models: https://elevenlabs.io/docs/models
34. ElevenLabs v4: https://elevenlabs.io/blog/eleven-v4 ; https://techcrunch.com/2026/09/28/elevenlabs-new-v4-speech-model-supports-more-expression-control-and-90-languages/
35. Cartesia Sonic 3.6: https://docs.cartesia.ai/build-with-cartesia/tts-models/latest ; https://cartesia.ai/blog/sonic-3.6/ ; Vapi measurement: https://humannessindex.vapi.ai/models/cartesia-sonic-3-5
36. OpenAI text-to-speech guide: https://developers.openai.com/api/docs/guides/text-to-speech
37. `ort` (ONNX Runtime for Rust, v2.0.0-rc.15, 2026-10-19): https://github.com/pykeio/ort
