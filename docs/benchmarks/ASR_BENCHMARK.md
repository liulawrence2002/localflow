# Speech-to-Text Benchmark: Before and After

Date: 2026-11-22. This report compares LocalFlow's original speech-to-text path with the
persistent-engine path that replaced it, and records the measurements behind the default
settings. Everything here was run on one machine with the harness in
`src-tauri/examples/asr_bench.rs`. Nothing is estimated unless it says so, and the last
section lists what was not measured.

## Summary

On 136 clips of clean read speech, three passes each, CPU only:

| | Original path | New default | Change (95 % interval) |
|---|---|---|---|
| Configuration | one `whisper-cli` process per utterance, beam 5, 8 threads | persistent `whisper-server`, greedy, 6 threads | |
| Median transcription time | 780 ms | 437 ms | -44.0 % (-45.6 to -42.4) |
| 95th percentile | 1,137 ms | 549 ms | -51.7 % (-54.9 to -48.0) |
| Clips on which the new path was faster | | 136 of 136 | |
| Word error rate | 3.18 % | 3.31 % | +0.13 points (-0.09 to +0.35) |
| First request after 30 s idle | 837 ms | 447 ms | single observation each |
| Peak resident memory of the engine | 150 MB per run, 0 between dictations | 132 MB, held while the app runs | |
| Errors in 408 requests | 0 | 0 | |

Both use the same model, `ggml-tiny.en-q5_1.bin`. The accuracy difference is three words in
2,358 and its interval includes zero, so this corpus cannot tell the two apart. A small
penalty from greedy decoding cannot be excluded: the interval reaches +0.35 points.

About 31 points of the median improvement come from keeping the model loaded and the rest
from greedy decoding (see "Where the gain comes from").

These numbers cover the speech-to-text stage only. They are not end-to-end dictation
latency; see "Not measured".

## Update: Parakeet kept loaded (run 5)

After the comparison below, Parakeet TDT 0.6B v3 was integrated in-process through
transcribe.cpp so the model stays loaded, and it became the default speech model. Raw data:
`asr-parakeet-raw.json` (three passes, 136 clips, CPU).

| | tiny.en on the whisper server | Parakeet TDT 0.6B v3 (Q5_K_M), in-process | Change (95 % interval) |
|---|---|---|---|
| Word error rate | 3.31 % | 0.93 % | -2.37 points (-3.34 to -1.49) |
| Median transcription time | 453 ms | 534 ms | +18.0 % (-2.6 to +24.2) |
| 95th percentile | 588 ms | 1,280 ms | +118 % (+94 to +182) |
| Median, clips up to 5 s (57 clips) | 424 ms | 334 ms | |
| Median, clips over 10 s (36 clips) | 540 ms | 1,118 ms | |
| First request after 30 s idle | 666 ms | 955 ms | single observation each |
| Non-speech probes that produced text | 2 of 2 | 0 of 2 | |
| Peak resident memory | 132 MB | 1,241 MB (whole benchmark process) | |
| Model load | 236 ms | 533 ms | |

Readings:

- **Accuracy is the reason for the switch.** Errors fall from 78 to 22 in 2,358 words and the
  interval is well clear of zero. The caution from run 1 still applies: LibriSpeech-style
  text is probably in Parakeet's training data, so the margin on real dictation is likely
  smaller.
- **Latency depends on length.** Whisper pays a fixed 30-second encoder window; Parakeet's
  cost grows with the audio. Parakeet is faster on short dictations and about twice as slow
  beyond ten seconds. The median difference is not distinguishable from zero; the 95th
  percentile is clearly worse.
- **Against the original path** (process per utterance, 780 ms median, 1,137 ms p95, 3.18 %
  WER, measured in run 4 rather than in this run): the median is lower and the error rate is
  far lower, but the 95th percentile is higher.
- **Memory is the cost.** About 1.2 GB while loaded against 132 MB. The figure is the whole
  benchmark process, which also holds the corpus; the model's own share was not isolated.
- GPU inference through Vulkan was not built or measured.

The rest of this report describes the Whisper comparison that preceded this change.

## Method

**Hardware and software.** Intel Core Ultra 7 165H (6 performance, 8 efficiency, 2
low-power cores; 22 threads), 32 GB RAM, Windows 11, on AC power with the Balanced plan and
ordinary desktop applications open. whisper.cpp 1.9.1, CPU build. The RTX 4070 Laptop GPU
was not used.

**Corpus.** 135 utterances from LibriSpeech test-clean (chapters 1089-134686, 121-127105,
and 4507-16021: three speakers) plus whisper.cpp's `jfk.wav`. 136 clips, 1,002 s, 2,358
reference words, median clip 5.8 s, range 2.0 to 35.0 s. LibriSpeech is CC BY 4.0.

**Harness.** Each configuration runs through the same `SpeechToTextProvider` code the app
uses. The timed region is one `transcribe` call: WAV encoding, the request or process
launch, decoding, and parsing. Server start-up is reported separately because the app
starts the server in the background when recording begins. The first request is included.
Every configuration is also checked for crash recovery, four overlapping requests, and two
non-speech probes (three seconds of silence, three seconds of low noise).

**Accuracy.** Corpus-level word error rate: total word edits divided by total reference
words, after lower-casing and removing non-alphanumeric characters
(`src-tauri/src/asr/metrics.rs`). Decoding is deterministic, so accuracy is scored on the
first pass.

**Statistics.** `paired_stats.py` compares two configurations clip by clip, using each
clip's median latency across passes, and reports 95 % percentile bootstrap intervals from
10,000 resamples of the clips. Resampling whole clips accounts for errors within a clip
being correlated. It does not account for the corpus having only three speakers.

**Runs.** Four runs were made. Runs 1 to 3 were single-pass exploration. Run 4 is the
final comparison: three passes, the new path measured first, with per-clip data recorded.
Between runs 3 and 4 the server provider gained a listener ownership check and a start
backoff; neither is on the per-request path.

## Reproducing

```powershell
# 1. Runtime and models (explicit downloads; the app never downloads anything)
.\scripts\Get-AsrModel.ps1 -Model tiny.en
.\scripts\Get-AsrModel.ps1                    # Parakeet
#    (base.en, used in runs 1 to 3, is no longer in the script; fetch ggml-base.en-q5_1.bin
#    from https://huggingface.co/ggerganov/whisper.cpp to repeat those runs)

# 2. Corpus: .localflow-runtime\bench-corpus\manifest.tsv (id<TAB>reference) and wav\<id>.wav
#    Extract the three chapters from https://www.openslr.org/resources/12/test-clean.tar.gz
#    and convert each FLAC:  ffmpeg -i in.flac -ar 16000 -ac 1 -c:a pcm_s16le wav\<id>.wav

# 3. Run
cd src-tauri
cargo run --release --example asr_bench -- --passes 3 --idle-secs 30 `
  --config server:ggml-tiny.en-q5_1.bin:1:6 `
  --config cli:ggml-tiny.en-q5_1.bin:5:8 `
  --json ..\docs\benchmarks\my-run.json

# 4. Compare
python ..\docs\benchmarks\paired_stats.py ..\docs\benchmarks\my-run.json `
  cli:ggml-tiny.en-q5_1.bin:5:8 server:ggml-tiny.en-q5_1.bin:1:6
```

A configuration is `engine:model:beam:threads[:context]`. Engines: `inprocess` (a GGUF
model through transcribe.cpp), `server`, `cli`. Context applies to Whisper only: `full`
(default), `dyn`, `vad`, `dyn+vad`. The `parakeet` engine used in run 1 (one
`parakeet-cli.exe` process per utterance) has since been removed in favour of `inprocess`.

## Run 4: final comparison (three passes)

Raw data: `asr-final-raw.json`.

| Configuration | p50 ms | p95 ms | Mean ms | p50 ms, clips up to 5 s | Real-time factor | WER % | Peak RSS MB | After 30 s idle, ms |
|---|---|---|---|---|---|---|---|---|
| `server:tiny.en:1:6` (new default) | 441 | 562 | 450 | 418 | 0.061 | 3.31 | 132 | 447 |
| `cli:tiny.en:5:8` (original) | 784 | 1,163 | 832 | 697 | 0.113 | 3.18 | 150 | 837 |
| `server:tiny.en:5:6` | 571 | 964 | 625 | 499 | 0.085 | 2.88 | 174 | 750 |
| `cli:tiny.en:1:8` | 626 | 794 | 647 | 598 | 0.088 | 3.31 | 118 | 590 |
| `server:tiny.en:1:8` | 429 | 564 | 447 | 401 | 0.061 | 3.31 | 132 | 548 |

Percentiles in this table are over all 408 requests. The paired figures below use each
clip's median, which is why they differ by a few milliseconds.

### Where the gain comes from

| Comparison (same model) | Median change | p95 change | WER difference, points |
|---|---|---|---|
| Original to new default | -44.0 % (-45.6 to -42.4) | -51.7 % (-54.9 to -48.0) | +0.13 (-0.09 to +0.35) |
| Persistent server alone: `cli:1:8` to `server:1:8` | -31.0 % (-32.3 to -29.6) | -25.9 % (-30.2 to -21.5) | 0.00 |
| Greedy alone on the server: `server:5:6` to `server:1:6` | -22.9 % (-25.8 to -21.1) | -41.9 % (-45.3 to -35.9) | +0.42 (-0.08 to +1.00) |
| Original to server with beam 5 | -27.3 % (-28.9 to -25.0) | -17.0 % (-22.0 to -13.9) | -0.30 (-0.89 to +0.17) |
| Eight threads to six on the server | +1.6 % (0.0 to +3.7) | -0.8 % (-6.3 to +5.6) | 0.00 |

Readings:

- **The persistent server is a pure win.** At identical decoding settings it is 31 % faster
  at the median with identical output.
- **Greedy decoding buys most of the tail improvement** and may cost a little accuracy. Beam
  5 on the server had the lowest tiny.en error rate (2.88 %), and the interval for greedy
  against it runs from -0.08 to +1.00 points. The default is greedy because LocalFlow's
  target is visible text within about a second of the end of speech, of which 550 ms is the
  end-of-speech wait. Beam 5 on the server remains faster than the original path with no
  detectable accuracy loss, and is the setting to choose if accuracy matters more.
- **Six threads and eight are equivalent here.** The default cap is six because it leaves
  two more cores free at the same speed; it is not faster.
- **The CLI and the server are not decode-identical at beam 5** (3.18 % against 2.88 %).
  The server does not carry previous-text context between segments; that is the likely
  reason but it was not isolated.

## Run 1: models and engines (single pass)

Raw data: `asr-matrix-raw.json`.

| Configuration | p50 ms | p95 ms | WER % | Peak RSS MB | Server ready ms | Crash recovery ms |
|---|---|---|---|---|---|---|
| `cli:tiny.en:5:8` (original) | 884 | 1,333 | 3.18 | 151 | | |
| `cli:tiny.en:1:8` | 741 | 979 | 3.31 | 118 | | |
| `server:tiny.en:1:8` | 522 | 699 | 3.31 | 132 | 451 | 909 |
| `server:tiny.en:1:6` | 484 | 618 | 3.31 | 131 | 237 | 715 |
| `server:tiny.en:5:6` | 636 | 1,042 | 2.88 | 169 | 229 | 1,169 |
| `server:base.en:1:6` | 1,044 | 1,273 | 2.42 | 180 | 240 | 1,304 |
| `server:base.en:1:8` | 1,051 | 1,371 | 2.42 | 181 | 441 | 1,613 |
| `server:base.en:5:6` | 1,320 | 1,961 | 2.21 | 236 | 456 | 2,219 |
| `cli:base.en:5:8` | 1,507 | 2,117 | 2.29 | 217 | | |
| `parakeet:tdt-0.6b-v3-q8_0` (process per utterance) | 1,483 | 2,939 | 0.93 | 727 | | |

Run-to-run variation is visible: the original path measured 884 ms here and 784 to 820 ms
in later runs. Single-pass differences under about 10 % should not be read as real.

Approximate 95 % intervals for these error rates, treating words as independent (which
understates the width): 3.31 % is plus or minus 0.72 points, 2.42 % is plus or minus 0.62,
0.93 % is plus or minus 0.39.

- **base.en** is more accurate than tiny.en on this corpus (2.42 % against 3.31 %, 21 fewer
  errors), but the difference is at the edge of what the corpus can resolve, and it costs
  roughly 2.2 times the latency. It is therefore opt-in.
- **Parakeet TDT 0.6B v3** is clearly more accurate (22 errors against 78). Its 1.5 s
  median is dominated by loading a 669 MB model on every call, because this runtime has no
  persistent Parakeet server. Two cautions: LibriSpeech-style text is likely to be in its
  training data, and the simple normaliser penalises Whisper for hyphenation and British
  spellings in the references, so the margin on real dictation is probably smaller.
- **Crash recovery works.** Killing the server and sending the next request returned text
  in 0.5 to 2.2 s, depending on model size, in every configuration.
- **Overlapping requests work.** Four concurrent requests succeeded in every configuration
  (the server serialises them).

## Runs 2 and 3: dynamic audio context and VAD gate (single pass)

Raw data: `asr-variants-raw.json` (run 2) and `asr-widectx-raw.json` (run 3).

Whisper's encoder always processes a 30-second window. "Dynamic audio context" shrinks that
window to the clip length plus a margin. Run 2 used a tight margin (audio + 5.1 s, floor
10.2 s); run 3 used a wide one (audio + 10 s, floor 15 s), which is what the code now
contains. The margin is recorded in the JSON from run 3 onward; run 2's is recorded only
here.

| Configuration | p50 ms | p95 ms | WER % | Non-speech probes that produced text |
|---|---|---|---|---|
| `server:tiny.en:1:6`, full window | 472 | 638 | 3.31 | 2 of 2 |
| `server:tiny.en:1:6`, dynamic, tight | 204 | 516 | 3.77 | 2 of 2 |
| `server:tiny.en:1:6`, dynamic, wide | 250 | 475 | 3.77 | 1 of 2 |
| `server:base.en:1:6`, full window | 1,104 | 1,358 | 2.42 | 2 of 2 |
| `server:base.en:1:6`, dynamic, tight | 388 | 906 | 5.98 | 2 of 2 |
| `server:base.en:1:6`, dynamic, wide | 521 | 1,007 | 2.97 | 1 of 2 |
| `server:small.en:1:6`, dynamic, tight | 1,345 | 2,921 | 14.84 | 2 of 2 |
| `server:base.en:1:6`, full window, Silero VAD | 1,171 | 1,447 | 2.71 | 0 of 2 |
| `server:base.en:1:6`, dynamic tight, Silero VAD | 497 | 1,066 | 6.11 | 0 of 2 |

- **Tight dynamic context is harmful.** base.en more than doubled its error rate and
  small.en reached 14.8 %. That is a real degradation.
- **Wide dynamic context is unproven, not rejected.** It roughly halves the median for both
  models, and its error rates (3.77 % against 3.31 %, 2.97 % against 2.42 %) are within
  what this corpus can distinguish from no change. base.en with the wide margin would give
  about the default's latency with possibly better accuracy. The failure mode when it goes
  wrong is severe (repeated or dropped phrases), so it stays off until it is tested on a
  larger and more realistic corpus. `AudioContext::Dynamic` remains in the code for that.
- **The VAD gate stops invented text on silence** in both probes, at about 67 ms and a
  possibly higher error rate (2.71 % against 2.42 %, within noise). Two probes are too few
  to conclude much, and LocalFlow already rejects near-silent recordings before
  transcription, so the gate is supported by the server provider but not enabled.

## Resampler

The polyphase windowed-sinc resampler was timed in a release build with
`cargo test --release --lib resampler_speed -- --ignored --nocapture`: 30 s of audio takes
19 ms at both 44.1 kHz and 48 kHz. A first, direct-form version took 139 ms for 5 s and
was replaced after review. Unit tests show tones above 8 kHz are attenuated by more than
60 dB and speech-band tones keep their level within 3 %.

Its effect on recognition accuracy was **not measured**. The benchmark corpus is already
16 kHz, so the resampler is a pass-through there. No published measurement of resampler
quality against word error rate was found either.

## Background cleanup (Ollama)

`scripts/Measure-OllamaCleanup.ps1`, model `llama3.2:3b`, nine requests per variant after
one warm-up, on the RTX 4070 Laptop GPU. Raw data: `ollama-cleanup-raw.json`.

| Request options | Succeeded | Median wall time | Median output tokens |
|---|---|---|---|
| Before: temperature only | 0 of 9 | | |
| After: `num_ctx` 4096, `num_predict` 1024 | 9 of 9 | 3,363 ms | 221 |

Every "before" request failed with `llama-server reported out-of-memory during startup:
failed to allocate buffer of size 10737418240` (a 10 GB context cache). With 8.5 GB of RAM
free at the time, the unbounded request could not load the model at all, which means the
app's background cleanup was failing on this machine in that state. Bounding the context
fixes it. This is one machine in one memory state; with more free memory the unbounded
request may succeed and merely use more memory.

Cleanup runs after the text is already inserted, so its 3.4 s does not delay visible text.

## Not measured

- **End-to-end latency in the running app.** Speech end to visible text also includes the
  550 ms end-of-speech wait, resampling, formatting, and the paste. The app records these
  per dictation in Diagnostics; no real-microphone session was run for this report.
- **Real dictation audio.** The corpus is read audiobook speech at 16 kHz from three
  speakers. It has no background noise, spontaneous speech, spoken punctuation, short
  commands, names, or jargon, and no dictionary prompt was sent.
- **The resampler's effect on accuracy**, and the channel-locking change, both of which
  need real 48 kHz multi-channel capture.
- **GPU inference**, battery impact, and thermal behaviour over long sessions.
- **Idle cost of the sidecar** beyond its 132 MB peak resident memory.
- **Latency after long idle periods.** One 30-second gap per configuration was measured;
  minutes of idle were not.
- **The installer.** The bundle configuration changed but the installer was not rebuilt.
- **Any text-to-speech metric.** LocalFlow has no TTS.
- **Cost per 1,000 requests.** There is no hosted service; marginal cost is electricity.
  An estimate with its assumptions is in
  `../research/appendix/F-benchmark-verification-and-economics.md`.

## What would raise confidence most

1. Thirty or more real 48 kHz dictations through the old and new builds, using the app's
   stage timers.
2. The decisive configurations on all of LibriSpeech test-clean and test-other with a
   Whisper-style text normaliser.
3. Wide dynamic context and the VAD gate on that larger set, with paired statistics.
4. A Parakeet-class model kept loaded, to measure its latency without the per-call load.
