# Conversation Mode Benchmark

Measured 2026-11-22 on the development workstation: Intel Core Ultra 7 165H (22 logical
cores), NVIDIA RTX 4070 Laptop (8 GB), Windows 11, "Balanced" power plan, on mains power.
Ollama serving `llama3.2:3b` fully on the GPU; speech recognition and the voice on the CPU.

Every number here was produced by a harness in `src-tauri/examples/`. None is an estimate.
What was not measured is listed at the end.

## How to reproduce

```powershell
.\scripts\Get-PhononModel.ps1      # the conversation voice, about 65 MB
.\scripts\Get-VadModel.ps1         # the voice-activity model, 1.2 MB
cd src-tauri
cargo run --release --example tts_bench -- ..\.localflow-runtime\models\phonon --passes 3
cargo run --release --example chat_bench
cargo run --release --example conversation_bench -- ..\.localflow-runtime --passes 2
```

Ollama must be running with the model installed. Close anything else that is using the CPU
or GPU: timings on this machine move with background load. Only timings and character
counts are printed; nothing that is said is written anywhere.

## Result

End of the user's speech to the first audio of the reply, eight turns (four questions, two
passes), measured from the moment the last sample of the question was delivered:

|                                                          | Median   | Slowest  |
| -------------------------------------------------------- | -------- | -------- |
| First working build                                      | 1,860 ms | 2,851 ms |
| With the language model kept awake while the user speaks | 1,090 ms | 1,859 ms |

Stages of the improved build:

| Stage                                             | Short questions (2-3 s)        | Long question (8.5 s) |
| ------------------------------------------------- | ------------------------------ | --------------------- |
| Silence waited for before deciding the turn ended | 700 ms                         | 700 ms                |
| Transcription (Parakeet, CPU)                     | 282-453 ms                     | 887-943 ms            |
| Language model, first words                       | 46-82 ms typical, up to 305 ms | 61-212 ms             |
| Voice, first audio after the first piece of text  | 63-147 ms                      | 122-130 ms            |

Stopping a reply: the speaker is told to stop 5 ms after the hotkey event reaches the
conversation loop, and then fades out over 6 ms.

The 700 ms wait and the transcription time are now most of the delay. Both are the subject
of the next conversation milestone (a turn-completion model so the wait can be shorter, and
transcribing each phrase while the user is still speaking so long turns cost no more than
short ones).

## Finding 1: the voice must not use every core

`tts_bench`, Phonon 31m (q8), warm, by number of threads given to its tensor backend:

| Threads                                          | Speed (times real time) | First audio, 27-character sentence | First audio, 159 characters |
| ------------------------------------------------ | ----------------------- | ---------------------------------- | --------------------------- |
| 22 (the library's default: one per logical core) | 1.0-1.2                 | 310-313 ms                         | 1,072-1,501 ms              |
| 2                                                | 2.5-3.5                 | 119 ms                             | 398 ms                      |
| 4                                                | 4.2-4.5                 | 67 ms                              | 247 ms                      |
| 6                                                | 4.2-4.4                 | 76 ms                              | 302 ms                      |
| 8                                                | 4.6-4.7                 | 73 ms                              | 294 ms                      |
| 12                                               | 3.9-4.1                 | 69 ms                              | 305 ms                      |

With its default thread count the voice barely kept up with real time. LocalFlow gives it
at most four threads (`tts/phonon.rs`), which is within a few percent of the best setting
and leaves the rest of the machine for speech recognition.

First audio grows with the length of the text handed over, because the model reads the
whole piece before it starts. That is why replies are cut into sentences, and why the first
piece may be cut at a clause (`conversation/chunker.rs`).

Model load: 204 ms. Cancelling mid-sentence returned in 229 ms.

## Finding 2: an idle laptop GPU is slow to answer

`chat_bench`, six turns back to back: first words after 49-83 ms on every turn but the
first.

The same requests with a pause before each one, as when a user is speaking
(time the server spent reading the prompt, per turn):

| Pause before each request | Prompt time per turn |
| ------------------------- | -------------------- |
| 0.5 s                     | 20-23 ms             |
| 2 s                       | 49-83 ms             |
| 5 s                       | 90-343 ms            |
| 9 s                       | 202-322 ms           |

Inside the full pipeline, where transcription has just used the CPU, the same step took
807-1,145 ms. `nvidia-smi` showed the GPU at its lowest power state (P8, 210 MHz, 3 W)
between requests. A few idle seconds, which is how long a spoken question lasts, are enough
for it to drop there.

LocalFlow therefore sends the model a one-token request about every 400 ms from the moment
the user starts speaking until their question is sent (`ChatProvider::keep_warm`). This is
what took the median response from 1,860 ms to 1,090 ms. It does not remove the effect
entirely: two of the eight turns still spent 167-190 ms reading the prompt.

Remembering replies as they were spoken (cut into sentences and cleaned of markup) rather
than exactly as the model wrote them did not slow the next request: 49-62 ms either way.

Cancelling a reply by closing the connection frees the server at once: the next request got
its first words in 46-66 ms.

## After milestone 4: transcribing during the turn

Measured 2026-11-23 with the same harness, twelve turns (four questions, three passes).
Phrases are now transcribed at each 250 ms pause while the user is still speaking, and the
silence that ends a turn depends on how finished the transcript reads.

|                                            | Median   | Slowest  |
| ------------------------------------------ | -------- | -------- |
| Milestone 2 (700 ms wait, then transcribe) | 1,090 ms | 1,859 ms |
| Milestone 4                                | 663 ms   | 786 ms   |

| Stage                                            | Short questions (2-3 s) | Long question (8.4 s) |
| ------------------------------------------------ | ----------------------- | --------------------- |
| Silence when the turn was ended                  | 576-736 ms              | 640-672 ms            |
| Transcription, total across the turn's phrases   | 398-622 ms              | 1,159-1,255 ms        |
| Language model, first words                      | 40-163 ms               | 179-190 ms            |
| Voice, first audio after the first piece of text | 43-136 ms               | 69-85 ms              |

The long question spent over a second in the recogniser in total, but almost all of it
while the speaker was still talking: its response time (652-756 ms) is the same as a short
question's. The silence when the turn ended is longer than the 400 ms a finished question
needs because the transcript of the last phrase arrives about 350-500 ms after the pause is
noticed; the turn ends the moment it does. That makes the recogniser's speed on the last
phrase the remaining limit.

An earlier run of the same build, with eight turns and a placeholder rule in place of the
completeness judge, gave a median of 484 ms. Recogniser times were lower in that run
(275-425 ms against 398-622 ms), which is the difference; timings on this machine move
with background load.

Echo cancellation costs 31-77 microseconds per 10 ms of audio, measured in its own
harness.

### The microphone while the assistant speaks

Measured by playing a test signal through the laptop's own speakers at full volume and
recording its own microphone, four runs:

| Run | Microphone, nothing playing | Microphone during playback | Loss from speaker to microphone |
| --- | --------------------------- | -------------------------- | ------------------------------- |
| 1   | -69 dBFS                    | -50 dBFS                   | 29 dB                           |
| 2   | -86 dBFS                    | -77 dBFS                   | 56 dB                           |
| 3   | -78 dBFS                    | -67 dBFS                   | 46 dB                           |
| 4   | -87 dBFS                    | -69 dBFS                   | 48 dB                           |

In runs 2 to 4 what reached the microphone had no linear relation to what was played
(correlation 0.01-0.02), which means this laptop's audio driver already removes the
speaker from the microphone. Run 1, on another day, was louder and contained an outside
sound; why its level differed by about 27 dB is not known. On this machine, then, talking
over the assistant does not depend on LocalFlow's canceller, which detects the situation
and passes the microphone through. On synthetic rooms with a real echo path it removed
29-33 dB after the first second.

## After the upgrades: GPU recognition and Smart Turn

Measured 2026-11-24 with the same harness, plugged in, all runs back to back in one session
so they can be compared with each other: three passes of the four questions each. These
builds include the slice 24 turn-taking changes and Smart Turn.

|                                                 | Median | Slowest  | Recognition per turn, short questions |
| ----------------------------------------------- | ------ | -------- | ------------------------------------- |
| Speech recognition on the GPU (Vulkan)          | 568 ms | 853 ms   | 432-591 ms                            |
| Speech recognition on the CPU, with Smart Turn  | 993 ms | 1,380 ms | 549-762 ms                            |
| Speech recognition on the CPU, no Smart Turn    | 965 ms | 1,256 ms | 636-840 ms                            |

Smart Turn costs nothing measurable here: it runs on its own threads while the recogniser
works. The GPU takes about 40 % off the response time.

The CPU figures are slower than the 663 ms measured after milestone 4, and the cause is not
known. The same build without Smart Turn is no faster, and timings on this machine have
moved by this much between runs of one build before (484 ms against 663 ms, above), so
compare only within a table.

The first run of a freshly built GPU program stalled twice on its first pass: one turn
answered after 9.9 s and another after 3.8 s, with the recogniser taking 9.7 s and 5.0 s.
Later passes, and a second run of the same program, never stalled. The cause is Vulkan
preparing kernels for larger inputs the first time they are needed; the 2 s warm-up clip
did not reach them. The warm-up now also runs 4 s and 10 s clips on a GPU (see
`ASR_SPEED_BENCHMARK.md`). A freshly built program with that warm-up, two passes, did not
stall: median 533 ms, slowest 956 ms.

## Not measured

- A person speaking into a real microphone, in a real room, with a real speaker. The
  questions here were spoken by the conversation voice itself and delivered digitally, so
  recognition is easier than with a human voice, and there is no echo or background noise.
- The output device's own latency, which adds to what the user perceives.
- Speech-detection accuracy: missed speech, false starts on noise, and turns cut off
  mid-thought.
- Behaviour on battery power or under another power plan.
- Any model other than `llama3.2:3b`.
- How the voice sounds. Nobody has listened to it for naturalness yet.
- A person talking over the assistant. Interruption is tested with scripted audio only,
  and the final echo canceller settings were never run against the real devices.
- How often the completeness judge makes the assistant answer too early or wait too long
  in real conversation. On its own test sets it never called an unfinished sentence
  finished (215 and 52 utterances), but those were written, not spoken.
