# Speech Recognition Speed: CPU Threads and the GPU

Measured 2026-11-23/24 on the development laptop (Core Ultra 7 165H, 22 logical CPUs; RTX
4070 Laptop, 8 GB; Windows 11) with Parakeet TDT 0.6B v3 (`Q5_K_M` GGUF) through
transcribe-cpp 0.4.2. Corpus: the 136 clips in `.localflow-runtime/bench-corpus` (1,002 s of
audio, 25 clips of 1-3 s). No transcript text is printed or stored by the harness.

## CPU thread count

`examples/asr_bench.rs --sweep-threads`, one pass. "Quiet" means nothing else was building
or benchmarking; the laptop was on battery (Balanced power plan).

| Threads | Median / p95 per clip, quiet | Median / p95 per clip, while other builds ran |
| --- | --- | --- |
| 4 | 625 / 1,598 ms | 1,759 / 7,671 ms |
| 6 | 590 / 1,556 ms | 1,140 / 3,065 ms |
| 8 | 635 / 1,949 ms | 1,036 / 2,564 ms |
| 12 | 570 / 1,487 ms | 1,167 / 2,871 ms |
| 22 | no result after 15 min (stopped) | no result after 43 min (stopped) |

Word error rate was 0.93 % in every configuration. transcribe.cpp's default is not "every
CPU": it uses min(8, usable CPUs), so 8 on this laptop. Between 6 and 12 threads the
differences are within one pass's noise; 22 threads (every logical CPU, including the slow
low-power cores) never finished. LocalFlow keeps the library default;
`LOCALFLOW_ASR_THREADS` overrides it.

## GPU through Vulkan

Built with `--features gpu-vulkan`. The discrete NVIDIA GPU is chosen explicitly (the Intel
Arc integrated GPU is never used), with the CPU as the fallback if the GPU is missing or a
load or run fails. Three runs of the corpus test
(`real_parakeet_gpu_agrees_with_the_cpu_on_the_bench_corpus`); the first two while other
work was running, the third quieter.

|                                         | CPU                | GPU             |
| --------------------------------------- | ------------------ | --------------- |
| Word error rate                         | 0.93 %             | 0.85 %          |
| Warm median, 1-3 s clips                | 734 / 680 / 383 ms | 199 / 190 / 135 ms |
| Warm median, all clips                  | 1,486 / 1,304 / 646 ms | 385 / 320 / 211 ms |
| Model load                              | 489-546 ms         | 1,204-1,439 ms  |
| First 1-3 s clip in a fresh process     | 516 ms             | 132 ms          |

127 of 136 transcripts were identical between CPU and GPU (5 word edits in 2,358 words).
With `llama3.2:3b` loaded in Ollama (2.6 GB), the GPU's memory use went from 2,581 MiB to a
peak of 3,396 MiB of 8,188 MiB, so Parakeet adds about 0.8 GB. The voice detector (Silero)
stays on the CPU. The NVIDIA driver's shader cache was already warm, so the very first
compile on a new machine was not measured; a short warm-up transcription at the start of
each conversation keeps that cost off the user's first turn.

## End to end

See "After the upgrades" in `CONVERSATION_BENCHMARK.md`.

## Building and running

The GPU build needs the Vulkan SDK (`VULKAN_SDK` set and `%VULKAN_SDK%\Bin` on `PATH`;
`build.rs` adds `%VULKAN_SDK%\Lib` to the linker path). `npm run tauri:build:gpu` builds the
app with it; `LOCALFLOW_ASR_DEVICE=cpu` switches a GPU build back to the CPU without a
rebuild. The GPU build imports `vulkan-1.dll`, so it does not start on a computer without a
Vulkan loader; that is why the feature is off by default.

```text
.\scripts\Bench-AsrSpeed.ps1 -SweepThreads "4,6,8,10,12"   # thread sweep, 3 passes, then GPU (quote the list)
.\scripts\Bench-AsrSpeed.ps1 -Part Gpu                   # GPU against CPU only
```

Not measured: three passes on a quiet machine on mains power, the first-ever shader compile,
and behaviour when Ollama has filled the GPU's memory.
