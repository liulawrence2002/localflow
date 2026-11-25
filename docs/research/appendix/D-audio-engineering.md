# Voice quality and audio engineering for LocalFlow

> **Research snapshot dated 2026-11-22.** Kept as written; model choices, measurements and plans may have changed since, and it may not match the current code. For what was decided and built, see [VOICE_ASSISTANT_PLAN.md](../../VOICE_ASSISTANT_PLAN.md) and [development/PLAN.md](../../development/PLAN.md).

> Research appendix produced on 2026-11-22 by an independent research agent for the voice pipeline review. It is kept as written, apart from removing machine-specific paths. Line references to `native_dictation.rs` describe the code before the changes in `../VOICE_PIPELINE_REVIEW.md`. Figures are labelled by the author as vendor, third-party, reproduced, or unverified; treat unlabelled numbers as unverified.

Research date: 2026-11-22. Scope: Part A covers the dictation input chain (to be implemented); Part B covers TTS output quality (report only).

Evidence labels used throughout:

- **Measured**: a published experiment or primary source reports numbers.
- **Anecdotal**: implementation reports, vendor claims, defaults chosen by maintainers, or engineering reasoning without a controlled ASR measurement.
- **None found**: I searched and found no usable evidence.

Numbers are cited to sources in the numbered list at the end. Where I state something from general DSP knowledge rather than a fetched source, it is marked "(not sourced)".

---

## 0. What the current code does (read-only inspection)

File: `src-tauri\src\native_dictation.rs`

| Stage                                                 | Current behaviour                                                                                | Issue                                                                                                                                                 |
| ----------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| Downmix (`downmix_input_callback`, `loudest_channel`) | Picks the highest-energy channel **per audio callback**                                          | The selected channel can change between consecutive callbacks (every few ms), splicing two different signals together and creating discontinuities    |
| End of speech (`EndOfSpeechDetector`)                 | RMS thresholds 0.008 start / 0.0045 continue, 120 ms confirm, 550 ms silence timeout, 55 ms poll | Fixed absolute thresholds: no noise-floor adaptation, sensitive to mic gain and DC offset                                                             |
| Resample (`resample_to_16khz`)                        | Two-point linear interpolation, no low-pass                                                      | For 48 kHz input the ratio is exactly 3, so every output sample lands on an input sample: this is plain decimation with **zero** anti-alias filtering |
| WAV (`write_wav`)                                     | 16-bit PCM, 16 kHz mono, `round(x * 32767)`                                                      | Fine                                                                                                                                                  |
| Whisper invocation                                    | `whisper-cli -m -f -oj -of -np -nt -l en -t`                                                     | No VAD flags, no minimum-length padding                                                                                                               |

The bundled `whisper-cli.exe` in `.localflow-runtime\whisper\Release` already exposes `--vad`, `-vm`, `-vt`, `-vspd`, `-vsd`, `-vmsd`, `-vp`, `-vo` (verified by running `--help`), and a `parakeet-cli.exe` is also present. `.localflow-runtime\bench-corpus` already holds `jfk.wav` and a LibriSpeech folder.

---

## Part A: input side

### A1. Resampler quality

**Question: does linear interpolation without a low-pass measurably hurt ASR when going 44.1/48 kHz to 16 kHz?**

**Evidence strength: none found for a controlled WER measurement. The signal-level defect is certain.**

What I found:

- No paper, whisper.cpp issue, or faster-whisper issue isolates "naive resampler vs windowed-sinc" and reports Whisper or Parakeet WER. Two extended searches returned nothing of that kind.
- Reference Whisper never sees naive resampling: `load_audio` shells out to ffmpeg with `-ac 1 -acodec pcm_s16le -ar 16000`, so the model was trained and evaluated on ffmpeg-quality (anti-aliased) resampling [1]. Naive decimation is therefore an out-of-distribution input.
- Other dictation projects have filed the same bug with signal-level proof but no WER: a 12 kHz tone captured at 48 kHz and converted to 16 kHz produced a dominant peak near 4 kHz [4]; a second project reports high-frequency microphone noise aliasing into Whisper's speech band [5]. Both state the WER effect is expected, not measured.
- A vendor blog claims "4-7% WER improvement" from switching to a high-quality resampler but gives no method; treat as unverified [6].
- Counter-evidence worth knowing: a 2025 study on very low sample rates (320-1600 Hz) found ASR models _trained on_ aliased audio exploit the aliased information and get lower WER than with filtered resampling [2]. That is a train-time effect at extreme rates and does not transfer to Whisper inference, but it shows aliasing is not automatically catastrophic.
- A commercial-engine test found identical accuracy (98.0%) for 48 kHz and 16 kHz 16-bit WAV input, i.e. nothing above 8 kHz is needed provided the downsampling is done properly [3].

Engineering analysis of the current resampler (calculated, not sourced):

- 48 kHz to 16 kHz: pure decimation by 3. Everything between 8 and 24 kHz folds into 0-8 kHz at full amplitude. Sibilants, keyboard clicks, fan and coil whine, and any ultrasonic interference land in the mel bins Whisper uses.
- 44.1 kHz to 16 kHz: linear interpolation has a sinc-squared response whose first null is at 44.1 kHz. Attenuation is about 1 dB at 8 kHz, 4 dB at 16 kHz and 8 dB at 22 kHz. That is effectively no anti-aliasing, plus about 0.7 dB of droop at 7 kHz.

**Recommendation: replace it.** The cost is about 80 lines and well under 100 ms per utterance; the risk is nil; the defect is real even though its WER impact is unmeasured. Then measure the effect with the benchmark in A6 so the PLAN.md claim is honest ("fixed aliasing; WER delta on N utterances = X").

Two implementation options:

| Option                                                   | Pros                                                                                                        | Cons                                                                                                                                                                                                     |
| -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Hand-rolled polyphase Kaiser-windowed sinc (recommended) | Zero new dependencies, about 80 lines, fully unit-testable, deterministic                                   | You own the code                                                                                                                                                                                         |
| `rubato` 5.1.0, `Fft` resampler with `process_all()`     | MIT/Apache-2.0, maintained; docs call `Fft` the natural choice for a fixed ratio and offline conversion [7] | Pulls `realfft`, `audioadapter`, `audioadapter-buffers`, `num-integer`, `num-traits`, `visibility`, `windowfunctions` [7]. Do not use `Async::new_poly`: the docs say it has no anti-alias filtering [7] |

**Concrete filter design (hand-rolled):**

| Parameter                                                            | Value                                                                  | Reason                                                                                                                  |
| -------------------------------------------------------------------- | ---------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| Structure                                                            | Rational polyphase FIR, L = 16000 / gcd(fs_in, 16000), M = fs_in / gcd | 48 kHz: L=1, M=3. 44.1 kHz: L=160, M=441. 32 kHz: L=1, M=2. 96 kHz: L=1, M=6                                            |
| Passband edge                                                        | 7.0 kHz                                                                | Keeps almost all of Whisper's 0-8 kHz mel range                                                                         |
| Stopband edge                                                        | 8.0 kHz (new Nyquist)                                                  | Nothing above Nyquist survives, so no fold-back at all                                                                  |
| Cutoff (-6 dB)                                                       | 7.5 kHz                                                                | Midpoint of the transition band                                                                                         |
| Stopband attenuation                                                 | 80 dB                                                                  | Whisper's feature floor is 80 dB below the per-clip peak [1], so anything lower is invisible to the model               |
| Window                                                               | Kaiser, beta = 7.857                                                   | beta = 0.1102 x (80 - 8.7)                                                                                              |
| Taps at input rate                                                   | N = 2 x ceil(40 x fs_in / 16000) + 1                                   | 241 at 48 kHz, 223 at 44.1 kHz, 481 at 96 kHz (the Kaiser estimate for a 1 kHz transition at 80 dB gives 241 at 48 kHz) |
| Upsampling case (fs_in < 16 kHz, e.g. Bluetooth hands-free at 8 kHz) | Cutoff 0.47 x fs_in, 81 taps                                           | Suppresses images rather than aliases; same code path                                                                   |
| Per-phase gain                                                       | Normalise each phase's coefficients to sum to 1.0                      | Exact unity DC gain, no amplitude ripple between phases                                                                 |
| Edges                                                                | Zero-extend the input by (N-1)/2 on both sides                         | Output length = ceil(len x L / M), no delay to compensate                                                               |
| Arithmetic                                                           | f32 samples, f64 for coefficient generation                            |                                                                                                                         |

Kernel: `h(t) = (2 fc / fs_in) * sinc(2 fc t / fs_in) * kaiser(t / half_width)` evaluated at `t = k - frac` for each of the L phases, where output sample n sits at input position `n * M / L`.

Cost: 16 000 output samples/s x 241 multiply-adds is about 3.9 M MAC per second of audio, so a 30 s utterance is roughly 116 M MAC, tens of milliseconds single-threaded.

Unit tests to write (pure functions, no Tauri):

1. 1 kHz sine at 48 kHz and 44.1 kHz comes out within 0.1 dB.
2. 12 kHz sine at 48 kHz comes out at least 70 dB down (currently it comes out at 0 dB as a 4 kHz tone).
3. DC input of 0.5 comes out as 0.5 after the edge region.
4. Output length equals `ceil(len * 16000 / fs_in)`.
5. 16 kHz input is returned unchanged.

### A2. Input conditioning

**Summary: do very little. Fix what is objectively broken (DC, clipping, very low levels) and do not add adaptive processing or denoising.**

**How the models see level.** Whisper computes log-mel, clamps to `max - 8.0` in log10 power units (an 80 dB window under the clip's loudest bin), then applies `(x + 4) / 4` [1]. The clamp is relative to the clip peak; the final offset is absolute, so features do shift with gain, but the model was trained on audio with uncontrolled levels. Parakeet-class NeMo models take 16 kHz mono [15]; their default feature pipeline uses per-feature normalisation, which makes them largely gain-invariant (not sourced: from the NeMo default config, verify before relying on it).

| Topic                                                            | Recommendation                                                                             | Value                                                                            | Evidence                                                                                                                                                                                                                                                         |
| ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| DC offset / high-pass                                            | One 2nd-order Butterworth high-pass (biquad) before the RMS detector and before resampling | 80 Hz cutoff at the capture rate                                                 | **None found** for a WER effect. Justification is engineering: DC and rumble bias the RMS endpoint thresholds and waste headroom. Cheap, linear, harmless                                                                                                        |
| Level normalisation                                              | Static per-utterance peak gain, applied once after capture                                 | If peak < -12 dBFS, scale so peak = -3 dBFS; cap gain at +30 dB; never attenuate | **None found**: no published Whisper gain sweep. Rationale: keeps the signal well above the 16-bit quantisation floor. 10-12 bit effective resolution is reported as still stable for ASR, degrading at 8 bits and below [14]                                    |
| LUFS normalisation                                               | Not needed                                                                                 | n/a                                                                              | **None found**. LUFS is a perceptual playback measure; peak or RMS is sufficient for a recogniser                                                                                                                                                                |
| AGC (time-varying)                                               | Do not add                                                                                 | n/a                                                                              | **Anecdotal**. Time-varying gain pumps the noise floor during pauses, which is the kind of input that triggers Whisper hallucinations (A3). Static gain gets the benefit without the pumping                                                                     |
| Clipping                                                         | Detect and warn; do not try to repair                                                      | Flag when more than 0.1% of samples have abs >= 0.99                             | **Measured that clipping is harmful, weakly attributed**: a 2026 robustness benchmark reports large WER increases under hard clipping [13] (I confirmed the paper and its seven-system scope, not the per-model figure). No evidence found that declipping helps |
| Noise suppression (RNNoise, DeepFilterNet 3, NSNet, Windows APO) | **Do not add a denoiser by default**                                                       | n/a                                                                              | **Measured**, see below                                                                                                                                                                                                                                          |

**Denoising evidence (measured, consistent across three independent studies):**

1. "When De-noising Hurts" (2025): MetricGAN+ in front of Whisper, Parakeet, Gemini Flash 2.0 and Parrotlet-a on 500 medical recordings. Noisy audio beat enhanced audio in all 40 configurations; degradation 1.1 to 46.6 points absolute semantic WER; Whisper was the most sensitive [8].
2. SAM-Audio in front of Whisper (2026): WER and CER rose in every model and dataset combination; Whisper base English WER went from 10.53% to 21.66%; the effect was worse for larger Whisper models [9].
3. Polar projection diagnosis (2026): six enhancers in front of Whisper-large-v3 on VoiceBank+DEMAND. Average WER: noisy 6.6%; MossFormerGAN 6.0 (the only one that helped); FRCRN 6.7; MeanFlowSE 8.2; FlowSE 8.9; MetricGAN+ 10.6; simple magnitude mask 11.4. All improved PESQ, STOI and SI-SDR. At 12.5 dB SNR and above the differences are within about half a point either way. Whisper's response to enhancement strength is U-shaped: a mild setting (alpha = 0.25) is best, full enhancement is statistically indistinguishable from noisy input (p = 0.92) [10].

Gaps: I found **no direct RNNoise, DeepFilterNet or NSNet into Whisper WER benchmark**. DeepFilterNet's own paper reports PESQ and STOI only (PESQ 2.81 vs 2.33 for RNNoise) [11], which the studies above show does not predict WER. Parakeet's own card shows it is already robust: average WER 6.34% clean, 7.12% at 10 dB SNR, 8.23% at 5 dB, 11.66% at 0 dB [15].

If a noisy-environment mode is ever wanted: make it opt-in, apply the enhancer at roughly quarter strength (following [10]), and gate the feature on a measured WER win in the A6 benchmark.

**Windows capture path.** cpal opens a shared-mode WASAPI stream without setting an audio category, so the stream is "Other" and gets whatever the OEM driver implements for the Default signal-processing mode; applications cannot choose the mode or discover it, only raw vs non-raw [12]. On many laptops that already includes beamforming, noise suppression and AGC. Raw mode forbids all adaptive processing [12]. Consequences:

- Do not stack your own denoiser or AGC on top of unknown OEM processing.
- The Speech category exists for speech input such as a personal assistant [12], but cpal does not expose stream categories, so using it means a small `windows`-crate WASAPI path in `src-tauri/src/platform`. Treat it as a later experiment, not a first step. **Evidence that Speech mode beats Default for Whisper: none found.**
- Avoid the Communications category: it is the VOIP mode [12], tuned for calls.

### A3. VAD, endpointing, pre-roll, trimming, padding

**Current versions and defaults (measured from primary sources):**

| Engine                   | Version / status                                                                                                                            | Defaults                                                                                                                                                                                                                      |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Silero VAD               | v6.2.3 is latest; v6.0 claims 16% fewer errors on noisy real-life data; v6.2 improves edge cases and out-of-distribution stability [17]     | threshold 0.5, neg_threshold = max(threshold - 0.15, 0.01), min_speech 250 ms, min_silence 100 ms, speech_pad 30 ms, 512-sample (32 ms) window at 16 kHz [16]                                                                 |
| whisper.cpp built-in VAD | Silero v6.2.0 as ggml (`ggml-silero-v6.2.0.bin`, fetched by `models/download-vad-model.cmd`) [18]                                           | Same as Silero: 0.50 / 250 / 100 / 30 ms, overlap 0.10 s [18]                                                                                                                                                                 |
| faster-whisper           | Silero                                                                                                                                      | threshold 0.5, min_speech 0, **min_silence 2000 ms, speech_pad 400 ms** [19]                                                                                                                                                  |
| LiveKit agents (Silero)  | Real-time turn detection                                                                                                                    | min_speech 50 ms, **min_silence 550 ms, prefix padding 500 ms**, activation 0.5 [20]                                                                                                                                          |
| TEN VAD                  | Apache 2.0 "with additional conditions"; 16 kHz, 10 or 16 ms hops; 508 KB on Windows x64; ONNX model available; no Rust binding listed [21] | Vendor claims better precision than Silero and WebRTC and faster speech-to-silence transitions [21]. Unverified: one community test on a single file shows the two disagreeing wildly (Silero 13.0% speech vs TEN 78.8%) [22] |
| WebRTC VAD               | Legacy                                                                                                                                      | In the hallucination study below it was far worse than Silero as a Whisper front-end [23]                                                                                                                                     |

Note the spread: the "transcription" defaults (faster-whisper: 2000 ms / 400 ms) are an order of magnitude more conservative than the raw Silero defaults (100 ms / 30 ms). The raw defaults are for detection, not for cutting audio you intend to transcribe.

**Why trimming matters (measured).** Whisper large-v3 hallucinated on 40.3% of 301 317 non-speech clips; "thank you" was 24.8% of those outputs and "thanks for watching" 10.3%. Appending non-speech to real speech gave detected hallucination rates of 0.3% with 1 s appended, 11.9% with 10 s and 44.5% with 30 s. As a pre-processor on speech plus non-speech, WER was 104.8% with nothing, 68.3% with WebRTC VAD, 8.0% with Silero VAD, and 6.5% with Silero plus de-looping plus a "bag of hallucinations" filter [23]. Calm-Whisper notes VAD alone cannot remove the problem completely [24]. Practical reading: up to about 1 s of surrounding silence is harmless; multi-second silences are the risk.

**Minimum length (measured, with a version caveat).** Current whisper.cpp source rejects input shorter than **100 ms** with "input is too short ... consider padding the input audio with silence" [18]. The widely quoted 1 s figure comes from issue #1603, where clips under 1 s silently returned nothing and padding past 1 s fixed it; that issue was closed without a recorded fix [25]. Because the bundled build's exact behaviour is unknown, pad anyway: it is free.

**Pre-roll (anecdotal, one small measurement).** Microphone open latency of 0.4 to 1.1 s on some devices is reported as the root cause of clipped first words. Projects use 300 to 500 ms ring buffers [26][27]. One PR measured it: with a 500 ms pre-roll, first-word recognition went from 0/4 to 4/4 for speech starting between 300 ms before and at key-down, and from 1/4 to 4/4 at +100 ms; the measured request-to-first-buffer delay without it was about 130 ms [26].

**Recommended parameters:**

| Item                                     | Value                                                                                                                                                           | Evidence                                                                                                                                       |
| ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| Speech probability threshold             | 0.5 on, 0.35 off                                                                                                                                                | Measured defaults [16]                                                                                                                         |
| Live auto-stop silence (toggle mode)     | 700 ms (range 550 to 800)                                                                                                                                       | Anecdotal: LiveKit uses 550 ms [20]; current code uses 550 ms. Dictation tolerates slightly longer pauses than conversation. Make it a setting |
| Minimum speech to count as started       | 100 to 250 ms                                                                                                                                                   | Defaults [16][20]; the current 120 ms is fine                                                                                                  |
| Pre-roll                                 | 400 ms (range 300 to 500)                                                                                                                                       | Anecdotal plus one measurement [26]                                                                                                            |
| Padding kept around speech when trimming | 200 ms each side (range 200 to 400)                                                                                                                             | Anecdotal: faster-whisper uses 400 ms [19]. Never use the 30 ms default for audio that will be transcribed                                     |
| Internal silence                         | Leave pauses under 2 s untouched; shorten longer ones to about 500 ms                                                                                           | Derived from [19] and [23]                                                                                                                     |
| Leading/trailing silence                 | Trim to the padding above                                                                                                                                       | Measured benefit [23]                                                                                                                          |
| Minimum clip length                      | Zero-pad the tail to at least 1.1 s                                                                                                                             | Measured symptom [25]; Whisper zero-pads to 30 s internally anyway [1]                                                                         |
| Post-filter                              | If the whole transcript is one of the top hallucination strings ("thank you", "thanks for watching", "you", "so") and the clip was short or low-energy, discard | Measured [23]                                                                                                                                  |

**Pre-roll and the privacy posture.** A true pre-roll needs the microphone open before the hotkey, which conflicts with a privacy-first default. Suggested order:

1. Default: open the stream on key-down as now, but measure and log (as a number, not audio) the key-down-to-first-buffer latency so the problem is visible in diagnostics.
2. Opt-in "instant start": keep the stream warm with a 400 ms in-memory ring buffer, never written to disk, overwritten continuously, with a clear indicator that the mic is open. This is what the measured project did [26].

**Implementation path with the least new code:**

1. Phase 1 (no new dependencies): high-pass before the RMS detector; replace fixed thresholds with a noise-floor-relative threshold (for example, track a low percentile of RMS over the last 2 s and trigger about 10 dB above it; these two values are my suggestion, not sourced); trim leading and trailing silence to 200 ms; pad to 1.1 s.
2. Phase 2: enable the bundled whisper.cpp VAD for trimming: `--vad -vm ggml-silero-v6.2.0.bin -vt 0.5 -vspd 100 -vsd 1000 -vp 200`. Verify the `-vsd` flag in the bundled build first: one mirrored copy of the 1.8.3 example source appears to assign `--vad-min-silence-duration-ms` to the min-speech field (seen in a single secondary snippet, unconfirmed). A/B this in the benchmark, because VAD stitching can change punctuation.
3. Phase 3 (optional): Silero for live endpointing through whisper.cpp's C API or an ONNX runtime. Only worth it if the adaptive RMS endpointer fails in noisy rooms. TEN VAD is not recommended now: the licence has extra conditions, there is no Rust binding, and its accuracy claims are unverified [21][22].

### A4. Bit depth, sample rate, container

- **16 kHz mono is not a choice.** Whisper's front-end is hard-coded to 16 kHz (N_FFT 400, hop 160, 30 s = 480 000 samples) [1]; Parakeet requires 16 kHz mono [15]. Feeding more gains nothing: 48 kHz and 16 kHz input scored identically (98.0%) on a commercial engine [3].
- **16-bit PCM is exactly what reference Whisper uses**: ffmpeg `pcm_s16le`, then `int16 / 32768.0` [1]. Float WAV gives no benefit unless the signal is very quiet, and the static gain in A2 covers that case. ASR is reported stable down to 10-12 bits [14]. **Measured.**
- **The WAV path is lossless.** The only losses in the chain are the resampler (A1) and clipping at capture. Minor nit: the writer scales by 32767 while Whisper divides by 32768, a 0.0003 dB difference, irrelevant.
- Dither is unnecessary at 16 bits for ASR (not sourced; follows from [14]).
- Lossy codecs are irrelevant for a local pipeline, but for reference Opus did not reduce accuracy down to 16 kbps (97.9 to 98.2%) and dropped at 6 kbps (95.5%) [3].
- Possible simplification: pipe PCM to whisper over stdin or link whisper.cpp as a library to skip the temp WAV. That is a latency and privacy improvement (no dictated audio on disk), not a quality one.

### A5. Multi-channel and array microphones

**Evidence strength: measured for "channels differ, and selection or weighting matters" in distant-speech research; none found specifically for laptop arrays with Whisper.**

- Multi-microphone ASR research finds per-channel WER differs substantially (the microphone facing away is worst), that quality-weighted combination beats selection, and that simple energy-based selection is less reliable than recogniser-informed selection [28]. Those results are for distant, distributed microphones, not a laptop lid.
- Phase cancellation when averaging (calculated, not sourced): two capsules spaced d apart see a delay of `d * sin(theta) / 343 m/s`. For d = 10 cm and a talker 90 degrees off-axis the delay is 0.29 ms and the first null of the averaged signal is at about 1.7 kHz, inside the speech band. For a talker directly in front (the normal laptop case) the delay is near zero and averaging is benign, giving up to 3 dB SNR improvement against uncorrelated noise.
- Many Windows laptop arrays are already processed by the driver in Default mode, which may include beamforming [12]; in that case any strategy gives much the same result.

**Bug in the current strategy:** selection is per callback. If two channels have similar energy the choice flips between callbacks, splicing different signals (different delay and gain) every few milliseconds. This is the one downmix behaviour that is definitely wrong.

**Recommendation:**

| Rule                                                 | Detail                                                                                                                                      |
| ---------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| Decide once per utterance, not per callback          | Store interleaved frames (or per-channel buffers) during capture; choose at `finish_recording`                                              |
| Default: best single channel for the whole utterance | Highest RMS over the speech region. No comb-filter risk; handles the common "one channel is dead or near-silent" device                     |
| Ignore dead channels                                 | Drop any channel more than 20 dB below the loudest                                                                                          |
| Optional refinement                                  | If the top two channels are within 3 dB and their zero-lag normalised correlation exceeds 0.9, average them; otherwise keep the single best |
| Never                                                | Sum without halving (clips), or take channel 0 blindly (the microphone is not always on channel 0)                                          |
| Live level meter                                     | Can keep using the per-callback loudest channel, since it only drives the overlay                                                           |

The thresholds in this table (20 dB, 3 dB, 0.9) are my engineering suggestions, not sourced values.

### A6. Objective evaluation for dictation

**Metrics:**

| Metric                            | Use                                                                      | Tooling                                                                                                                                                                                                                                                                                               |
| --------------------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| WER (normalised)                  | Headline accuracy, comparable with published numbers                     | `jiwer.process_words` gives the alignment and error counts in one call [30]. The Open ASR Leaderboard removes punctuation and casing and applies a normaliser that closely follows Whisper's (numbers, spellings, filler removal) [29]; Whisper's English normaliser lives in the openai/whisper repo |
| CER                               | Sensitive to small spelling and name errors; useful for short utterances | `jiwer.process_characters` [30]                                                                                                                                                                                                                                                                       |
| Formatted WER (WER_pc)            | Dictation users see punctuation and case, so score them                  | Same alignment without lowercasing or punctuation stripping. LibriSpeech-PC defines WER, WER_c and WER_pc plus a Punctuation Error Rate that scores only the punctuation tokens [31]. An extended-Levenshtein method reports orthographic errors separately from word errors [32]                     |
| Hallucination rate                | Fraction of silence or noise clips yielding any text                     | Count non-empty outputs on non-speech clips, following [23]                                                                                                                                                                                                                                           |
| First-word and last-word accuracy | Directly measures pre-roll and endpointing                               | Compare first and last reference word with the hypothesis                                                                                                                                                                                                                                             |
| Latency                           | Already captured by `NativeLatencySnapshot`                              | RTFx = audio seconds / processing seconds [29]                                                                                                                                                                                                                                                        |

Report both normalised WER (for comparability) and WER_pc (for user-perceived quality). Always report the absolute number of utterances and errors; on a few hundred utterances a change of a few tenths of a WER point is noise.

**Test sets:**

| Set                           | Licence                                                                                        | Sample rate                                     | Size / URL                                                                      | Notes                                                                                     |
| ----------------------------- | ---------------------------------------------------------------------------------------------- | ----------------------------------------------- | ------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| LibriSpeech test-clean        | CC BY 4.0 [33]                                                                                 | 16 kHz                                          | 346 MB, https://openslr.trmal.net/resources/12/test-clean.tar.gz [33]           | Standard; read speech; already 16 kHz so it **cannot** test the resampler                 |
| LibriSpeech test-other        | CC BY 4.0 [33]                                                                                 | 16 kHz                                          | 328 MB, https://openslr.trmal.net/resources/12/test-other.tar.gz [33]           | Harder recordings                                                                         |
| LibriSpeech dev-clean         | CC BY 4.0 [33]                                                                                 | 16 kHz                                          | 337 MB, https://openslr.trmal.net/resources/12/dev-clean.tar.gz [33]            | Use for tuning so the test sets stay clean                                                |
| LibriSpeech-PC                | CC BY 4.0 [31]                                                                                 | 16 kHz                                          | Paper states code, data and models are public; locate via arXiv 2310.02943 [31] | Same audio with punctuation and capitalisation restored; the right reference for WER_pc   |
| TED-LIUM 3                    | CC BY-NC-ND 3.0 [29][34]                                                                       | not verified                                    | See [34]                                                                        | Non-commercial, no derivatives: fine for internal benchmarking, do not redistribute clips |
| Earnings-22                   | CC BY-SA 4.0 [29]                                                                              | not verified                                    | Via the Open ASR Leaderboard datasets on Hugging Face [29]                      | Long-form earnings calls with jargon; less like dictation                                 |
| Common Voice (English)        | CC0 [34]                                                                                       | not verified (commonly 48 kHz MP3, not sourced) | Mozilla / Hugging Face                                                          | Real consumer microphones; a good proxy for dictation acoustics                           |
| VCTK                          | CC BY 4.0 [34]                                                                                 | 48 kHz (not sourced, verify)                    | Edinburgh DataShare                                                             | Clean native high-rate speech: the best candidate for the resampler A/B                   |
| whisper.cpp `samples/jfk.wav` | US government speech, generally treated as public domain (no explicit licence statement found) | 16 kHz                                          | In the whisper.cpp repo [18]; already in `.localflow-runtime\bench-corpus`      | Smoke test only; one utterance says nothing about WER                                     |

**Reproducible local protocol:**

1. Fixed manifest: 200 utterances from LibriSpeech test-clean and 200 from test-other, selected by a seeded shuffle and committed as a list of IDs (not audio).
2. Resampler A/B: 100 native 48 kHz utterances (VCTK or Common Voice), each converted to 16 kHz by (a) the current linear code, (b) the new polyphase filter, (c) ffmpeg as the reference. Same model, same flags. Report WER for each and the count of utterances whose transcript changed.
3. Conditioning A/B: the same clips at 0, -20 and -40 dB gain; with and without the high-pass; with and without static gain.
4. Endpointing: prepend and append 0, 1, 5 and 10 s of silence and of recorded room noise; report hallucination rate and WER with and without trimming, mirroring [23].
5. Short clips: single words of 300 to 900 ms with and without the 1.1 s pad.
6. Scoring: jiwer with the Whisper English normaliser for WER; unnormalised for WER_pc against LibriSpeech-PC references.
7. Record model file, quantisation, whisper.cpp build and flags with every result.

---

## Part B: output side (report only)

Caveat: a local read-back feature plays audio on the same machine, so network codecs and streaming bitrates are largely irrelevant; several levers below matter only if audio is ever exported.

### B7. TTS quality levers

| Lever                                   | Practice                                                                                                                                                                                                                                                                                                                                 | Evidence                                                                                       |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Text normalisation                      | The biggest intelligibility lever in practice: expand numbers, dates, units, abbreviations and URLs before G2P. Keep a map from normalised text back to source character offsets so playback can highlight text [46]                                                                                                                     | Anecdotal                                                                                      |
| Pronunciation                           | Layer: user lexicon, then dictionary G2P, then rule fallback. Kokoro uses `misaki`, which falls back to espeak-ng for out-of-dictionary words and supports inline overrides of the form `[Misaki](/phonemes/)` [45]. LocalFlow already has a custom dictionary for ASR; the same entries could carry pronunciations                      | Measured (documented behaviour)                                                                |
| SSML                                    | Useful as an input convention (`break`, `say-as`, `phoneme`, `sub`), but small local neural models generally do not parse it; translate tags into pauses and phoneme overrides yourself                                                                                                                                                  | Anecdotal (not sourced)                                                                        |
| Prosody                                 | In local models prosody is driven mainly by punctuation, sentence length and the voice or style embedding. Chunk at sentence and paragraph boundaries; vendor guidance suggests 200 to 400 characters per chunk to preserve intonation [46]                                                                                              | Anecdotal (vendor)                                                                             |
| Emotion and style tags                  | Available in larger models; small models such as Kokoro offer fixed voices instead. For reading back dictated text a neutral voice is the right default                                                                                                                                                                                  | Anecdotal (not sourced)                                                                        |
| Speaker consistency and long form       | Open autoregressive models deteriorate on long prompts; research attributes this to attention misalignment, and inference-time methods target prosodic drift, speaker inconsistency and sentence-boundary artefacts [46]. Mitigations: fixed voice embedding and seed for every chunk, sentence-level chunks, generate ahead of playback | Measured in the cited research; mitigations anecdotal                                          |
| Joins                                   | Trim leading and trailing silence per chunk, insert a deliberate pause (about 150 to 250 ms after sentences, 400 to 600 ms after paragraphs), and apply a 5 to 20 ms fade or equal-power crossfade so joins do not click                                                                                                                 | **None found** as a published recommendation; values are standard audio practice (not sourced) |
| Loudness                                | EBU R128 specifies -23 LUFS and -1 dBTP for broadcast; speech for mobile and podcast listening is normally targeted at about -16 LUFS with true peak at or below -1 dBTP (Apple's podcast guidance; ITU-R BS.2434 short-form figure) [44]. Normalise per document rather than per chunk, otherwise chunk-to-chunk level jumps appear     | Measured (standards)                                                                           |
| Sample rate                             | Use the model's native rate (Kokoro examples write 24 kHz [45]) and resample once to the device rate with the same polyphase resampler from A1. Do not resample twice                                                                                                                                                                    | Documented                                                                                     |
| Codec                                   | Local playback: uncompressed PCM straight to the output device. Export or network streaming: Opus is the usual choice; MP3 adds encoder delay and padding that complicate gapless chunk joins                                                                                                                                            | Anecdotal (not sourced)                                                                        |
| Artefacts                               | Common ones: clicks at joins, metallic buzz, swallowed final words, repeated or skipped words in autoregressive models. A cheap automatic guard is to transcribe the output with the local ASR and regenerate any chunk whose WER against the input text exceeds a threshold (LocalFlow already ships the recogniser)                    | Anecdotal; the ASR check mirrors standard WER-via-ASR evaluation [42]                          |
| Model choice for an offline Windows app | Kokoro-82M: Apache-licensed weights, 82 M parameters [45]; competitive in arenas despite its size (see B8)                                                                                                                                                                                                                               | Measured (model card and leaderboard snapshot)                                                 |

### B8. TTS evaluation

| Method                        | What it measures                                                      | Notes                                                                                                                                                                                                  |
| ----------------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| MOS (crowd, absolute 1 to 5)  | Overall naturalness                                                   | Expensive, noisy, not comparable across tests; surrounding context changes ratings [46]                                                                                                                |
| MUSHRA                        | Fine differences among systems against a hidden reference and anchor  | Needs a reference and careful listeners; best for codec or vocoder comparisons (not sourced)                                                                                                           |
| Preference / arena (pairwise) | Which of two outputs listeners prefer                                 | Most sample-efficient for small teams; this is how the public leaderboards work [39]                                                                                                                   |
| UTMOS / UTMOSv2               | Predicted naturalness MOS, no reference needed                        | UTMOSv2 was the University of Tokyo system for VoiceMOS Challenge 2024 Track 1 (high-quality synthetic speech); MIT-licensed, pip-installable [41]                                                     |
| NISQA                         | Predicted MOS plus noisiness, coloration and discontinuity sub-scores | Used alongside UTMOS as a non-intrusive predictor in recent TTS papers [42]. Discontinuity is useful for catching bad joins                                                                            |
| DNSMOS                        | Predicted signal quality and cleanliness                              | Used as a clarity proxy in TTS papers [42]; it does not measure naturalness                                                                                                                            |
| WER via ASR                   | Intelligibility and robustness (skips, repeats)                       | Seed-TTS-eval protocol: Whisper-large-v3 for English [42]                                                                                                                                              |
| Speaker similarity            | Voice consistency or cloning fidelity                                 | Seed-TTS-eval: cosine similarity of WavLM-large speaker-verification embeddings between output and reference [42]. For single-voice read-back, compare each chunk with the first chunk to detect drift |
| TTSDS2                        | Distributional benchmark for human-quality TTS                        | [43]                                                                                                                                                                                                   |

**Public leaderboards (snapshot 2026-11-22, from a secondary aggregator; the official TTS Arena V2 leaderboard rendered empty when fetched, so verify live) [39]:**

- TTS Arena V2 top entries: Aurora 1694 (+/-41, 380 votes), star-june-2026 1624 (+/-38), Gradium TTS 2026.10 1604 (+/-54, preliminary), MiniMax Speech-02-HD 1585 (+/-19, 2049 votes), Inworld TTS 1.5 MAX 1579, Inworld TTS MAX 1564, then a tie at 1554 among Smallest Lightning v3.1 Pro, Fish Audio OpenAudio S2 and ElevenLabs Eleven v3. Confidence intervals overlap heavily at the top, and the aggregator page contradicts itself about which model is first.
- Open-weight models in the same snapshot: Chatterbox 1490 (+/-18, 2555 votes) and Kokoro 82M v1.0 1477 (+/-21, 1850 votes), roughly rank 27 and 31 of 48.
- The official TTS Arena V2 site now describes its ranking as a Bradley-Terry fit with a conservative lower bound, and V1 votes do not carry over [39].
- Artificial Analysis runs a separate Speech Arena whose scores are on a different scale and must not be mixed with TTS Arena numbers [40].

**Lightweight protocol for a small team (thresholds are suggestions, not sourced):**

1. Text set: 40 sentences (10 plain, 10 with numbers, dates and units, 10 with names and jargon from the custom dictionary, 10 long or multi-clause) plus 3 passages of about 300 words for long-form.
2. Automatic gate on every build: WER via the local Whisper model against the input text, UTMOSv2 mean and 5th percentile, integrated loudness within 1 LU of -16 LUFS, true peak at or below -1 dBTP, and chunk-to-first-chunk speaker similarity for the passages. Track these against the previous build rather than against absolute targets.
3. Human check only when the automatic numbers move: blind A/B preference, 5 listeners x 20 pairs, randomised order, report the win rate with a binomial confidence interval. Listen to the long passages end to end at least once, since per-chunk scoring misses drift [46].
4. Keep the generated audio for failed items so regressions are audible, and store only test-set audio, never user dictation.

---

## Recommended input chain (summary)

Order of operations after capture:

1. Capture all channels interleaved at the device rate (pre-roll optional and opt-in, 400 ms).
2. Choose one channel for the whole utterance (A5).
3. High-pass, 2nd-order Butterworth, 80 Hz (A2).
4. Endpoint and trim: 200 ms padding each side, internal pauses under 2 s untouched (A3).
5. Static gain to -3 dBFS peak when peak is below -12 dBFS, capped at +30 dB; flag clipping (A2).
6. Polyphase Kaiser-sinc resample to 16 kHz: 7.0 kHz passband, 8.0 kHz stopband, 80 dB, 241 taps at 48 kHz (A1).
7. Zero-pad to at least 1.1 s (A3).
8. 16-bit PCM mono WAV, unchanged (A4).
9. No denoiser, no AGC (A2).

| Change                                                   | Priority            | Evidence                                                          |
| -------------------------------------------------------- | ------------------- | ----------------------------------------------------------------- |
| Anti-aliased resampler                                   | High                | Signal defect certain; WER effect none found, measure it          |
| Per-utterance channel selection                          | High                | Defect certain by inspection; WER effect none found               |
| Trim silence, pad short clips, hallucination post-filter | High                | Measured [23][25]                                                 |
| No denoiser                                              | High (a non-change) | Measured, three studies [8][9][10]                                |
| Pre-roll                                                 | Medium, opt-in      | Anecdotal plus one small measurement [26]                         |
| High-pass and static gain                                | Medium              | None found; low risk                                              |
| Silero VAD through bundled whisper.cpp                   | Medium              | Measured as a hallucination mitigation [23]; parameters anecdotal |
| Speech-category WASAPI stream, TEN VAD                   | Low, experiment     | None found / unverified vendor claims                             |

---

## Sources

1. OpenAI Whisper `audio.py` (ffmpeg load, constants, log-mel normalisation): https://github.com/openai/whisper/blob/main/whisper/audio.py
2. "Revisiting the Privacy of Low-Frequency Speech Signals: Exploring Resampling Methods, Evaluation Scenarios, and Speaker Characteristics": https://arxiv.org/html/2508.02483
3. AmiVoice Techblog, sampling rate and compression vs recognition accuracy: https://acp.amivoice.com/en/blog/2025-07-29/
4. GitHub issue, downsampling without anti-aliasing feeds false tones to VAD and Whisper: https://github.com/kurealnum/marceline/issues/196
5. GitHub issue, downsampling aliases high-frequency microphone noise into Whisper's speech band: https://github.com/laceyp99/murmur/issues/80
6. sipsip.ai blog (unverified resampling WER claim): https://sipsip.ai/blog/how-ai-transcribes-voice-recordings-to-text
7. rubato crate documentation (5.1.0): https://docs.rs/crate/rubato/latest
8. "When De-noising Hurts: A Systematic Study of Speech Enhancement Effects on Modern Medical ASR Systems": https://arxiv.org/abs/2512.17562
9. "When Audio Separation Hurts Zero-Shot ASR: Evaluating SAM-Audio with Whisper": https://arxiv.org/pdf/2603.04710
10. "Where Speech Enhancement Hurts Recognition: An Inference Time Polar Projection Diagnosis": https://arxiv.org/html/2607.11157v1
11. DeepFilterNet2 paper: https://arxiv.org/pdf/2205.05474
12. Microsoft Learn, Audio Signal Processing Modes: https://learn.microsoft.com/en-us/windows-hardware/drivers/audio/audio-signal-processing-modes
13. "Back to Basics: Revisiting ASR in the Age of Voice Agents": https://arxiv.org/abs/2603.25727
14. "Factors affecting ASR performance: A study using state of the art ASR models in Indic Languages": https://arxiv.org/pdf/2606.09335
15. NVIDIA Parakeet TDT 0.6B v3 model card: https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
16. Silero VAD `utils_vad.py`: https://github.com/snakers4/silero-vad/blob/master/src/silero_vad/utils_vad.py
17. Silero VAD releases: https://github.com/snakers4/silero-vad/releases
18. whisper.cpp README (VAD section), `src/whisper.cpp`, `samples/jfk.wav`: https://github.com/ggml-org/whisper.cpp
19. faster-whisper `vad.py`: https://github.com/SYSTRAN/faster-whisper/blob/master/faster_whisper/vad.py
20. LiveKit Silero plugin reference: https://docs.livekit.io/python/livekit/plugins/silero/index.html
21. TEN VAD repository: https://github.com/TEN-framework/ten-vad
22. livekit-plugins-tenvad (community comparison on one file): https://pypi.org/project/livekit-plugins-tenvad/
23. "Investigation of Whisper ASR Hallucinations Induced by Non-Speech Audio": https://arxiv.org/html/2501.11378v1
24. Calm-Whisper: https://arxiv.org/html/2505.12969v1
25. whisper.cpp issue #1603, audio under 1 s returns nothing: https://github.com/ggml-org/whisper.cpp/issues/1603
26. TypeWhisper PR #1497, opt-in microphone pre-roll with measurements: https://github.com/TypeWhisper/typewhisper-mac/pull/1497
27. Further pre-roll reports: https://github.com/altintasutku/hece/issues/106 and https://github.com/PeeeBrain/shuddhalekhan/pull/293
28. "Noise Robust Speech Recognition Using Multi-Channel Based Channel Selection And Channel Weighting": https://arxiv.org/html/1604.03276
29. "Open ASR Leaderboard: Towards Reproducible and Transparent Multilingual Speech Recognition Evaluation": https://arxiv.org/html/2510.06961v3 (repo: https://github.com/huggingface/open_asr_leaderboard)
30. jiwer documentation: https://jitsi.github.io/jiwer/usage
31. "LibriSpeech-PC: Benchmark for Evaluation of Punctuation and Capitalization Capabilities of end-to-end ASR Models": https://arxiv.org/abs/2310.02943
32. "Beyond Levenshtein: Leveraging Multiple Algorithms for Robust Word Error Rate Computations And Granular Error Classifications": https://arxiv.org/pdf/2408.15616
33. OpenSLR 12, LibriSpeech: https://www.openslr.org/12
34. Dataset licence catalogue (TED-LIUM 3, Common Voice, VCTK): https://voxkitchen.readthedocs.io/en/latest/datasets/
35. to 38. (numbers not used)
36. TTS Arena V2 about page: https://tts-agi-tts-arena-v2.hf.space/about ; snapshot aggregator used for scores: https://17nas.com/tts-leaderboard.php?lang=en-US ; original arena write-up: https://huggingface.co/blog/arena-tts
37. Artificial Analysis Speech Arena: https://artificialanalysis.ai/text-to-speech/leaderboard
38. UTMOSv2 (T05 system, VoiceMOS Challenge 2024): https://arxiv.org/abs/2409.09305 ; code: https://github.com/sarulab-speech/UTMOSv2
39. Seed-TTS-eval protocol: https://github.com/BytedanceSpeech/seed-tts-eval
40. TTSDS2: https://arxiv.org/html/2506.19441v1
41. EBU R 128: https://tech.ebu.ch/docs/r/r128v4_0.pdf ; ITU-R BS.2434: https://www.itu.int/dms_pub/itu-r/opb/rep/R-REP-BS.2434-2018-PDF-E.pdf ; Auphonic loudness targets: https://auphonic.com/blog/2013/01/07/loudness-targets-mobile-audio-podcasts-radio-tv/
42. Kokoro-82M model card: https://huggingface.co/hexgrad/Kokoro-82M ; misaki G2P: https://github.com/hexgrad/misaki
43. Long-form TTS: NVIDIA Magpie-TTS long-form docs https://docs.nvidia.com/nemo/speech/latest/tts/magpietts-longform.html ; Deepgram chunking guidance https://developers.deepgram.com/docs/tts-text-chunking ; Inworld long text input https://docs.inworld.ai/tts/capabilities/long-text-input ; long-form consistency write-up https://smallest.ai/blog/long-form-tts-how-to-generate-consistent-speech-at-scale ; long-form degradation research https://arxiv.org/html/2603.06444v1

## Known gaps

- No controlled WER measurement of resampler quality for Whisper or Parakeet exists that I could find; the recommendation rests on signal analysis and on matching the reference pipeline.
- No direct RNNoise, DeepFilterNet 3 or NSNet into Whisper WER benchmark found; the "denoising hurts" conclusion comes from MetricGAN+, SAM-Audio, FRCRN, MossFormerGAN and flow-based enhancers.
- No published Whisper input-gain sweep, and no evidence on high-pass filtering and WER.
- No laptop-array-specific downmix study.
- The clipping figure in [13] and the bit-depth figures in [14] were seen only in search summaries, not confirmed in the papers themselves.
- Several TTS sources in [46] and the dataset catalogue [34] were seen only through search summaries, not fetched in full.
- TTS Arena V2 scores come from a secondary aggregator dated today; the official leaderboard could not be read programmatically.
- Statements marked "(not sourced)" are from general DSP or audio knowledge and should be verified before being quoted as fact.
- Nothing in this report was measured locally; no LocalFlow files were modified.
