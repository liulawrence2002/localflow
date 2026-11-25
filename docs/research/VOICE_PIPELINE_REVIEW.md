# Voice Pipeline Review

> **Research snapshot dated 2026-11-22.** Kept as written; model choices, measurements and plans may have changed since, and it may not match the current code. For what was decided and built, see [VOICE_ASSISTANT_PLAN.md](../VOICE_ASSISTANT_PLAN.md) and [development/PLAN.md](../development/PLAN.md).

Date: 2026-11-22. Scope: a technical review of LocalFlow's voice pipeline, a survey of
current speech models, the changes made as a result, and what remains.

Measurements are in [`../benchmarks/ASR_BENCHMARK.md`](../benchmarks/ASR_BENCHMARK.md). The
six research reports this document summarises are kept unedited in [`appendix/`](appendix/).
Every figure below is labelled as **measured here**, **third-party**, **vendor**, or
**estimate**. Nothing in this document was measured unless it says so.

> **Update, later on 2026-11-22.** Two decisions below were superseded at the owner's
> request. Parakeet TDT 0.6B v3, kept loaded in-process through transcribe.cpp, is now the
> default speech model (0.93 % WER against 3.31 %, similar median latency; see run 5 in the
> benchmark report), and the per-utterance Parakeet CLI engine was removed. A read-aloud
> path using Fish Audio S2 Pro through s2.cpp was wired in despite the reservations in
> section 4 (size, speed on CPU, research-only licence, alpha runtime); it is implemented
> and unit-tested but the engine has not been built or run, so nothing about its speed or
> quality here is measured. Sections 3, 4, 6, and 8 describe the state before that update.

## 1. What the system is

LocalFlow is a local-only Windows dictation app. It has one voice pipeline:

```
hotkey -> microphone capture (cpal) -> end-of-speech detection -> resample to 16 kHz
       -> speech-to-text (whisper.cpp) -> deterministic formatting -> paste
       -> background cleanup by a local LLM (Ollama)
```

It has **no text-to-speech, no audio playback, and no conversational or full-duplex
pipeline**. The review brief asked about those. They are covered as research in sections 4
and 5, because they could matter for a future read-back feature, but there was nothing to
audit, benchmark, or optimise on the output side. "Time to first audio" does not exist in
this product; the equivalent user-facing number is **time from end of speech to visible
text**.

The project rules in `AGENTS.md` rule out every hosted model: no cloud ASR, LLM, or TTS,
and no silent network fallback. Hosted services appear below only as reference points.

## 2. Audit of the pipeline before this change

Evidence is from reading the code at commit `eb17641` and from the architecture review in
[`appendix/E-architecture-review.md`](appendix/E-architecture-review.md).

| Area              | State before                                                                                                                                | Consequence                                                                                        |
| ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------- |
| Speech-to-text    | A new `whisper-cli.exe` process per utterance, model `ggml-tiny.en-q5_1`, beam 5, 8 threads, audio and transcript passed through temp files | Process start and model load on every dictation; no way to swap engines                            |
| Sidecar control   | `Command::output()` with no timeout and no kill                                                                                             | A hung whisper left the overlay on "processing" and the audio file on disk                         |
| Provider boundary | The existing `AsrProvider` trait took no audio and was used only by mock sessions                                                           | Engine, model, language, and flags were hardcoded in a 2,550-line module                           |
| Resampling        | Linear interpolation with no low-pass filter                                                                                                | At 48 kHz this is plain decimation by 3: everything from 8 to 24 kHz aliases into the speech band  |
| Channel selection | Loudest channel chosen per audio callback                                                                                                   | The chosen channel could flip mid-word on a multi-channel microphone                               |
| Voice activity    | RMS thresholds; the watchdog skipped all timeouts when no samples arrived                                                                   | An unplugged microphone left the session listening with Escape grabbed globally                    |
| Clipboard         | Dictated text written with no history or cloud-sync exclusion                                                                               | Transcripts entered Win+V history and cloud clipboard                                              |
| Ollama client     | Default HTTP client: honoured proxy variables, followed redirects, no context limit                                                         | Text could leave loopback through a proxy; on this machine the request failed outright (section 6) |
| Observability     | Per-stage latency shown for the last dictation only; no benchmark harness                                                                   | No baseline existed                                                                                |
| Health file       | Read and rewritten on every level-meter event, about 18 times a second                                                                      | Disk I/O on the capture thread during dictation                                                    |
| Packaging         | Installer bundled all 37 runtime executables                                                                                                | About 6 MB of unused binaries, including test programs                                             |

Things that were already sound and were kept: session identity with supersede and cancel,
target-window revalidation before paste, deterministic formatting, transcript-free logging,
and the fast-insert design in which the LLM never blocks visible text.

## 3. Speech recognition: what the research found

Full report: [`appendix/A-speech-recognition.md`](appendix/A-speech-recognition.md).
Verification of the numbers: [`appendix/F-benchmark-verification-and-economics.md`](appendix/F-benchmark-verification-and-economics.md).

- **The accuracy leaders that run locally are 0.6B transducers, not Whisper.** On the
  Hugging Face Open ASR Leaderboard (v1.16.2, 2026-11-20) Parakeet TDT 0.6B v2 and v3 score
  6.93 and 7.04 average WER against 8.24 for Whisper large-v3 and 8.75 for large-v3-turbo.
  The best hosted systems are near 4.9 to 5.5. (third-party, read from the leaderboard)
- **The leaderboard changed in 2026.** It dropped LibriSpeech from the average and changed
  its dataset mix, so averages quoted on older model cards are not comparable. Whisper tiny,
  base, small, and distil-whisper are no longer listed at all.
- **Whisper's cost does not shrink with short audio.** The encoder always processes a
  30-second window. That is why small.en needs about 5 s for a 5 s clip on this CPU
  (measured here), and why large-v3-turbo is unusable on CPU (35 s for a 5 s clip, measured
  here by the research agent).
- **Closest comparable app.** Handy 0.10.1 (Tauri 2 and Rust, like LocalFlow) runs every
  model in-process through the `transcribe-cpp` crate with Vulkan on Windows and recommends
  Parakeet Unified EN. (third-party, read from its `Cargo.toml` and README)
- **Whisper invents text for silence.** Every Whisper size tested produced words for three
  seconds of silence or low noise; Parakeet returned nothing. A Silero VAD gate fixes it.
  (measured here)

### Weighted comparison for this application

Weights reflect a single-user offline dictation tool: accuracy 35, CPU latency 25,
memory and disk 10, licence 10, integration cost 10, dictation features (vocabulary prompt,
punctuation) 10. Scores are 1 to 5 and are judgement informed by the measurements; the
total is out of 100.

| Candidate                                                | Accuracy | Latency | Footprint | Licence | Integration | Features | Total | Basis                                         |
| -------------------------------------------------------- | -------- | ------- | --------- | ------- | ----------- | -------- | ----- | --------------------------------------------- |
| Whisper tiny.en, persistent server (new default)         | 2        | 5       | 5         | 5       | 5           | 4        | 77    | measured here                                 |
| Whisper base.en, persistent server (opt-in)              | 3        | 3       | 5         | 5       | 5           | 4        | 74    | measured here                                 |
| Whisper small.en                                         | 3.5      | 1       | 4         | 5       | 5           | 4        | 66    | measured here (latency), third-party (WER)    |
| Whisper large-v3-turbo, CPU                              | 4.5      | 1       | 2         | 5       | 5           | 5        | 71    | measured here (latency), GPU only in practice |
| Parakeet TDT 0.6B v3, one process per utterance (opt-in) | 5        | 2       | 2         | 5       | 4           | 2        | 71    | measured here                                 |
| Parakeet 0.6B kept loaded in-process (roadmap)           | 5        | 4       | 2         | 4       | 2           | 3        | 77    | estimate; latency not measured                |

Reading: the shipped default and the roadmap engine tie for the best total. The roadmap
engine trades integration effort for a large accuracy gain, and its score depends on a
latency estimate that must be measured before it is adopted.

Best option by goal, speech recognition:

| Goal                             | Choice                                                                                                                                                         | Status                              |
| -------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------- |
| Lowest latency on CPU            | Whisper tiny.en on the persistent server, greedy decoding                                                                                                      | Shipped as the default              |
| Highest accuracy available today | Parakeet TDT 0.6B v3 (CC-BY-4.0)                                                                                                                               | Shipped as an opt-in engine         |
| Efficient local deployment       | tiny.en default (32 MB model, about 130 MB resident)                                                                                                           | Shipped                             |
| Lowest cost at scale             | Local inference. Electricity is roughly USD 0.002 to 0.012 per 1,000 dictations against USD 0.33 to 0.87 hosted (estimate with stated assumptions, appendix F) | Not applicable to a single-user app |
| Best overall balance             | A 0.6B Parakeet-class model kept loaded, with Vulkan when a GPU exists                                                                                         | Roadmap; needs measurement          |

## 4. Text-to-speech: what the research found

Full report: [`appendix/B-text-to-speech.md`](appendix/B-text-to-speech.md). LocalFlow has
no TTS, so this section is a recommendation for a possible read-back feature, not a change.

- **Fish Speech is not a candidate here.** Its current open models are Drama 3 (35B
  mixture-of-experts, about 41.5 GB) and S2-Pro (4B), served on datacentre GPUs. The Fish
  Audio Community License permits commercial use only under USD 500k annual revenue with
  attribution, and the S2-Pro weights are research-only. (vendor and licence text)
- **Several small permissive projects reduced maintenance in 2026.** Supertonic was
  archived, Chatterbox was declared retired, and Piper's maintained fork is GPL-3.0 and
  seeking maintainers.
- **The practical Rust route is sherpa-onnx** (Apache-2.0, official crate, GPL-free since
  v2.0.0), which runs Kokoro, Kitten, Piper voices, and Pocket TTS.
- **Vendor latency claims often failed independent checks.** CosyVoice's "150 ms" measured
  2,566 ms median on an A100; Fish S2-Pro's "about 100 ms" measured 265 ms. (third-party)

### Fish Speech against the strongest alternatives

| Constraint                        | Fish Drama 3 / S2-Pro                                 | Kokoro-82M                | Pocket TTS                                                  | Kitten TTS 2 mini              |
| --------------------------------- | ----------------------------------------------------- | ------------------------- | ----------------------------------------------------------- | ------------------------------ |
| Download size                     | 41.5 GB / multi-GB                                    | 86 to 326 MB              | 100M parameters                                             | 47 to 83 MB                    |
| Runs on CPU only                  | No                                                    | Yes                       | Yes                                                         | Yes                            |
| Commercial use                    | Under USD 500k revenue, with attribution; S2-Pro none | Unrestricted (Apache-2.0) | Unrestricted with attribution (MIT code, CC-BY-4.0 weights) | Unrestricted (Apache-2.0)      |
| Rust or ONNX runtime              | None                                                  | Yes                       | Yes                                                         | Yes                            |
| Voice cloning and emotion control | Yes                                                   | No                        | Cloning only                                                | No                             |
| Independent speed data            | Partly reproduced                                     | Yes                       | Yes                                                         | None yet (released 2026-11-13) |

Fish leads on expressiveness, cloning, and emotional control. A read-back feature needs
none of those; it needs intelligible speech from a preset voice, on CPU, under a permissive
licence. Voice cloning is also a liability for a privacy-first product.

Weighted comparison (local CPU feasibility 25, licence 20, quality 20, footprint 10, Rust
integration 15, maintenance 10; judgement scores, total out of 100):

| Candidate                       | CPU | Licence | Quality | Footprint | Rust | Maintenance | Total |
| ------------------------------- | --- | ------- | ------- | --------- | ---- | ----------- | ----- |
| Kokoro-82M                      | 5   | 5       | 4       | 5         | 5    | 2           | 90    |
| Pocket TTS                      | 5   | 4       | 3.5     | 5         | 5    | 5           | 90    |
| Kitten TTS 2 mini               | 5   | 5       | 3       | 5         | 3    | 4           | 84    |
| Piper voice through sherpa-onnx | 5   | 3       | 2.5     | 5         | 5    | 2           | 76    |
| Fish Drama 3 / S2-Pro           | 1   | 1       | 5       | 1         | 1    | 5           | 44    |
| F5-TTS, XTTS-v2                 | 1   | 1       | 4       | 2         | 1    | 3           | 38    |

Best option by goal, text-to-speech:

| Goal                                 | Choice                                                                                                                     |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| Highest quality, any deployment      | Hosted leaders (ElevenLabs v4, Cartesia Sonic 3.6) or Fish Drama 3 on a datacentre GPU. All prohibited or impractical here |
| Lowest-latency conversation          | Hosted streaming APIs. Not applicable: LocalFlow has no conversational mode                                                |
| Efficient local deployment           | Kokoro-82M or Pocket TTS through sherpa-onnx                                                                               |
| Lowest cost at scale                 | Local CPU: about USD 0.011 to 0.019 per 1,000 requests against USD 4.50 to 24 hosted (estimate, appendix F)                |
| Best balance for a read-back feature | Pocket TTS (actively maintained, streaming) with Kokoro as the alternative                                                 |

## 5. Inference and audio engineering findings

Reports: [`appendix/C-inference-optimisation.md`](appendix/C-inference-optimisation.md) and
[`appendix/D-audio-engineering.md`](appendix/D-audio-engineering.md).

| Technique                                             | Finding                                                               | Decision                                                       |
| ----------------------------------------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------- |
| Persistent sidecar instead of a process per utterance | Removes process start, model load, and temp files                     | Adopted; measured                                              |
| Greedy decoding instead of beam 5                     | Faster; accuracy difference not detectable on our corpus              | Adopted; measured                                              |
| Threads limited to performance cores                  | Published guidance; here six threads were no slower than eight        | Adopted (cap of six)                                           |
| Dynamic Whisper audio context                         | Large speed-up but an accuracy cost that grows as the margin tightens | Not enabled; measured                                          |
| Silero VAD gate in whisper.cpp                        | Removes hallucinated text on silence; small latency cost              | Not enabled by default; measured, sample too small to conclude |
| Quantisation q5_1 versus q8_0                         | Evidence of benefit is weak                                           | No change                                                      |
| GPU build (CUDA)                                      | Prebuilt zips are 285 to 766 MB; no Vulkan prebuilt for whisper.cpp   | Roadmap; not measured                                          |
| ONNX Runtime with DirectML                            | Microsoft now lists DirectML as legacy                                | Rejected                                                       |
| Band-limited resampler                                | Fixes a certain signal defect; no published WER evidence either way   | Adopted; WER effect not measured                               |
| Noise suppression before ASR                          | Three studies show it raises WER for Whisper-class models             | Rejected                                                       |
| Bounded Ollama context                                | Prevents a 10 GB allocation                                           | Adopted; measured                                              |

## 6. What was changed

All changes are local-only and keep the existing behaviour as the fallback.

**Provider abstraction.** `src-tauri/src/asr/provider.rs` defines `SpeechToTextProvider`
(prepare, transcribe with a deadline, shutdown), a typed `AsrError`, and `FallbackProvider`.
The dictation coordinator now depends only on that trait. Three engines implement it:

- `whisper_server.rs`: a supervised persistent `whisper-server.exe` on a loopback ephemeral
  port. Audio is sent as an in-memory WAV. It is started while the user is still speaking,
  restarted if it exits, killed if a request passes its deadline, stopped on app exit, and
  tied to a Windows kill-on-close job object. A failed start is not retried for a minute.
  The listener's owning process is verified before any audio is sent.
- `whisper_cli.rs`: the previous one-process-per-utterance path, now with a deadline and a
  kill. It is the fallback and the low-resource engine.
- `parakeet_cli.rs`: an opt-in accuracy engine using the `parakeet-cli.exe` already in the
  runtime, selected by pointing the model path at a `ggml-parakeet-*.bin` file.

**Audio.** A dependency-free polyphase windowed-sinc resampler replaces linear
interpolation. The capture channel is locked once a channel carries signal.

**Reliability.** Timeouts now run when the microphone stops delivering audio. Every engine
has a deadline. Fallback is always to another local engine.

**Privacy and security.** Both loopback HTTP clients ignore proxies and refuse redirects.
Cloud-tagged Ollama models are refused. Dictated text is excluded from clipboard history
and cloud sync. Error messages carry file names, not full paths, and no longer echo model
output.

**Efficiency.** Ollama requests carry a fixed context window and an output cap. The health
file is written on phase changes only. The installer bundles three sidecar executables
instead of 37.

**Settings now honoured.** Whisper model path and low-resource mode affect the engine.
English-only models always use English; other models use the configured language.

**Tooling.** `src-tauri/examples/asr_bench.rs` is a reproducible benchmark that runs the
app's own provider code. `scripts/Get-AsrModel.ps1` is an explicit model download helper.
`scripts/Measure-OllamaCleanup.ps1` measures the cleanup request.

### Decisions on the existing components

| Component                | Decision                 | Reason                                                                           |
| ------------------------ | ------------------------ | -------------------------------------------------------------------------------- |
| whisper.cpp runtime      | Retain, used differently | The persistent server was already in the runtime folder                          |
| tiny.en model as default | Retain                   | Only model that meets the latency target on CPU; accuracy options are now opt-in |
| One-shot CLI             | Retain as fallback       | Zero idle memory; known-good path                                                |
| Deterministic formatter  | Retain                   | Not part of the bottleneck                                                       |
| Ollama cleanup           | Retain, bounded          | Background only; the unbounded request was failing                               |
| Linear resampler         | Replace                  | Signal defect                                                                    |
| Parakeet                 | Combine, as opt-in       | Large accuracy gain, but slow to start in this runtime                           |
| TTS                      | Not added                | No product requirement yet; shortlist recorded above                             |

## 7. Results in brief

Detail, method, and limits are in the benchmark report. On 136 clean read-speech clips
(1,002 s of audio) on an Intel Core Ultra 7 165H, CPU only:

- Moving tiny.en from a process per utterance with beam 5 to the persistent server with
  greedy decoding cut median speech-to-text time from 780 ms to 437 ms (-44 %, 95 %
  interval -45.6 to -42.4) and the 95th percentile from 1,137 ms to 549 ms (-52 %). The new
  path was faster on all 136 clips. Word error rate was 3.31 % against 3.18 %, three words
  in 2,358; the interval on the difference (-0.09 to +0.35 points) includes zero, though a
  small greedy-decoding penalty cannot be excluded.
- Keeping the model loaded accounts for about 31 points of that at identical output; greedy
  decoding accounts for the rest.
- base.en on the server gave 2.42 % WER at roughly twice the latency of the new default.
- Parakeet TDT 0.6B v3 gave 0.93 % WER, a clear improvement, at about 1.5 s per clip
  because its model loads on every call in this runtime.
- The unbounded Ollama cleanup request failed 9 of 9 times on this machine with an
  out-of-memory error; the bounded request succeeded 9 of 9 times.

Not measured: end-to-end latency in the running app with a real microphone, GPU inference,
the resampler's effect on accuracy, battery impact, and any TTS metric.

## 8. Roadmap

Ordered by expected value for effort.

1. **Measure in the real app.** Record 30 or more real 48 kHz dictations (names, commands,
   dictionary terms) and compare old and new builds with the existing stage timers. This is
   the only way to confirm end-to-end latency and the resampler's effect.
2. **Keep a Parakeet-class model loaded.** Evaluate `transcribe-cpp` (in-process, Vulkan,
   built-in Silero VAD) behind `SpeechToTextProvider`, or a newer whisper.cpp runtime if it
   gains a Parakeet server. Expected to bring Parakeet accuracy at under a second on CPU
   (estimate). Check the NVIDIA Open Model License before bundling Unified EN; TDT v3 is
   CC-BY-4.0.
3. **Update the whisper.cpp runtime** from 1.9.1 to 1.9.5 or later. Fixes since 1.9.1
   include a heap out-of-bounds read on very short audio, VAD flag parsing, and Parakeet
   decoding.
4. **GPU option.** Measure the CUDA build as an optional download; it would make
   large-v3-turbo practical on the 8 GB GPU.
5. **Re-test the VAD gate and wide-margin dynamic context** on a larger and more realistic
   corpus with per-utterance paired statistics.
6. **Streaming partials** on the existing `StreamingAsrProvider` foundation, once an engine
   with native streaming is in place.
7. **Remaining review findings**: idle shutdown of the sidecar, a shared deadline across
   the fallback chain, rich clipboard preservation, serde defaults on settings, a startup
   sweep of the temp folder, restricting executable overrides in release builds, CSP.
8. **Read-back TTS**, if wanted: Pocket TTS or Kokoro-82M through sherpa-onnx, evaluated
   with UTMOS, ASR-based intelligibility, and a small listening test (appendix D).

Models worth watching: Granite Speech 5.0 TurboCTC 470M (Apache-2.0, fast, recently gained
punctuation), Moonshine v2 streaming, Qwen3-ASR 1.7B for GPU, Kitten TTS 2 (very new, no
independent measurements yet).
