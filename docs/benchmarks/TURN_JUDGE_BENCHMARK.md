# Turn Judge Benchmark: Text Judge vs Smart Turn

Measured 2026-11-23 on the development workstation: Intel Core Ultra 7 165H (22 logical
cores), Windows 11, CPU only. Every number here was produced by
`src-tauri/examples/turn_judge_bench.rs` on the clips described below. None is an estimate.
What was not measured is listed at the end.

The question: should conversation mode use Smart Turn, the audio end-of-turn model in
`src-tauri/src/audio/smart_turn.rs`, which is written but not compiled into the app? The
rule was to wire it in only if it clearly helps, and otherwise to delete it.

## Recommendation: wire Smart Turn in

On 1,600 English clips of human speech from Pipecat's held-out test set, the text judge that
conversation mode uses today calls **28.4 %** of unfinished turns finished. In the app, each
of those turns gets the short 400 ms wait, so a pause longer than that ends the turn while
the user is still talking. Smart Turn alone, at Pipecat's threshold of 0.5, calls **5.4 %**
of them finished. On finished turns its mean wait is the same, 431 ms. Smart Turn with the
text judge moving its answer one step does a little better on both counts:

| Human clips (800 complete, 800 incomplete)                  | Cut-off (95 % interval) | Complete turns delayed | Mean wait, complete turns | Mean wait, incomplete turns | F1 for "complete" |
| ----------------------------------------------------------- | ----------------------- | ---------------------- | ------------------------- | --------------------------- | ----------------- |
| Text judge (today)                                          | 28.4 % (25.4-31.6)      | 7.8 %                  | 431 ms                    | 1,014 ms                    | 0.836             |
| Smart Turn alone, complete if p > 0.5                       | 5.4 % (4.0-7.2)         | 3.1 %                  | 431 ms                    | 1,346 ms                    | 0.958             |
| **Smart Turn at 0.5, text judge moves it one step** (below) | **4.5 % (3.3-6.2)**     | **4.1 %**              | **415 ms**                | **1,179 ms**                | **0.957**         |

This meets the decision rule set before any results (see Method) by a wide margin. The
two intervals for the cut-off rate are far apart, and finished turns wait 16 ms less. Smart
Turn ranks complete clips above incomplete ones almost perfectly: the area under its ROC
curve is 0.990. The result holds for each of the four human sources and the synthetic
clips, and when 200 ms of silence is added to the end of each clip.

How to wire it in:

1. Compile `audio/smart_turn.rs` into the app. That means `pub mod smart_turn;` in
   `audio/mod.rs`, and moving `tract-onnx` (no default features) and `tract-linalg` (with
   `multithread-mm`) from `[dev-dependencies]` to `[dependencies]`. Without
   `multithread-mm`, `smart_turn.rs` does not compile. These add 40 packages to
   `Cargo.lock`, and PLAN.md estimated about 27 MB of binary (not re-measured here). The
   model comes from `scripts/Get-SmartTurnModel.ps1` (32 MB, SHA-256 pinned). `best_turn_model`
   already returns none, with a reason, when the model is missing.
2. At each pause, when the assembler asks for the last phrase to be transcribed, run Smart
   Turn on that turn's audio so far, from the start of the turn to now. It uses only the last
   8 s. Run it on its own thread, at the same time as Parakeet: it took a median of 120 ms on
   this machine under light load, while transcription takes longer. That overlap was not
   measured in the app.
3. Combine the two answers:
   - Smart Turn p > 0.5 means Ready (400 ms) and p <= 0.5 means NotReady (1,400 ms).
   - If the text judge says incomplete, move Ready down to Unsure (700 ms).
   - If the text judge says complete, move NotReady up to Unsure (700 ms).

   `ReadinessJudge` takes text only today (`Fn(&str) -> Readiness`), so it needs the
   probability as a second input. When Smart Turn is not installed or fails, the input is
   none and the text judge decides alone, exactly as now.

4. A probability belongs to one pause. If the user speaks again, throw it away and compute a
   new one at the next pause.
5. Before making it the default, try it at the real microphone. This test set comes from
   the same pipeline as Smart Turn's training data. LocalFlow's echo-cancelled microphone
   audio has not been tested.

A higher threshold trades cut-offs for waiting. At 0.8, Smart Turn alone cuts off 3.6 % and
delays 6.2 % of finished turns (see the sweep below).

## Why the text judge cuts people off

The incomplete clips often end where the speaker took a breath mid-sentence, and Parakeet
puts a full stop there. It did so for 264 of the 800 incomplete human clips (33 %). The text
judge called 203 of those 264 complete. Of the 510 incomplete transcripts that ended with no
punctuation, it called only 13 complete. Together with 11 after a question mark, **214 of
the text judge's 227 cut-offs (94 %) follow punctuation that Parakeet placed where the
speaker had not finished.** PLAN.md already lists this as a known weakness ("can be fooled by
a full stop the recogniser put after a verb that wanted more"). The judge cannot hear the
voice, and the words before such a pause often do form a sentence.

Smart Turn hears the voice. It called 34 of those 264 full-stop clips complete (13 %), and
33 incomplete clips (4 % of 800) fooled both judges. No combination of the two can fix
those.

**On clips cut mid-word.** There are no reference transcripts, so whether Parakeet dropped
or "repaired" a cut-off last word cannot be checked directly. The clip endings were measured
instead, as the level of the last 100 ms against the loudest 100 ms of the clip. Only 128 of
the 800 incomplete human clips (16 %) stop within 15 dB of full loudness, the sign of a cut
in mid-sound. Most end on quieter audio, as at a pause. The text judge cut off 34 % of the
clips that stop on sound and 27-31 % of the others, so mid-word cuts are not what drive its
errors. The recogniser's full stops are. Smart Turn called none of the 128 that stop on
sound complete, and 4-7 % of the rest. It may partly be hearing the abrupt cut, which a real
pause would not have. Its 5-7 % on clips that end quietly is the fairer estimate.

Parakeet returned nothing for 11 of the 800 incomplete human clips, and the text judge said
unsure for all of them. In the padded run, 8 of 300 incomplete clips came back empty, against
2 of the same 300 unpadded.

## How to reproduce

```powershell
.\scripts\Get-SmartTurnModel.ps1   # Smart Turn v3.2, 32 MB
.\scripts\Get-AsrModel.ps1         # Parakeet, if not installed
.\scripts\Get-SmartTurnTestClips.ps1 -OutDir $env:TEMP\smart-turn-data   # 2,000 clips, 344 MB
cd src-tauri
cargo build --release --example turn_judge_bench
$bench = ".\target\release\examples\turn_judge_bench.exe"
& $bench run ..\.localflow-runtime $env:TEMP\smart-turn-data --out results.csv
& $bench run ..\.localflow-runtime $env:TEMP\smart-turn-data --out results-pad200.csv --pad-ms 200 --limit 600
& $bench report results.csv --md report.md
Remove-Item -Recurse $env:TEMP\smart-turn-data
```

`report` recomputes every table from the per-clip CSV without running the models again. The
CSV and the report hold:

- labels and verdicts;
- Smart Turn's probability;
- how the transcript ends (full stop, question mark, none and so on);
- word counts and timings.

No transcript text is printed or written anywhere. While a run is going, copy the exe
elsewhere first: a running exe in `target` blocks other builds on Windows.

## Method

**Data.** The data is Pipecat's Smart Turn v3.2 test set, `pipecat-ai/smart-turn-data-v3.2-test`
on Hugging Face, licensed CC-BY-4.0 (attribution at the end). It has 31,527 clips. Each clip
is the audio of a turn up to a pause, labelled `endpoint_bool`: complete or not. There are no
transcripts, because `spoken_text` is always empty. The clips were read through the
datasets-server `filter` API, which gives one signed FLAC link per row (16 kHz mono, 16 bit),
instead of downloading the whole 4.8 GB set. The only config is `default`, and the only split
is `train`.

English has 7,820 rows:

- 5,367 human: 2,649 complete and 2,718 incomplete.
- 2,453 synthetic (text-to-speech).

The filler flags exist only on synthetic rows. `midfiller` (a filler such as "um" in the
middle of the turn) is set on 1,069 of them. `endfiller` (the turn ends on a filler) is set
on 502, all of them incomplete.

Within each group and label, the rows were sorted by their id (a random UUID) and the first
N were kept: 800 + 800 human and 200 + 200 synthetic. That is 2,000 clips, 344 MB, 5.1 hours
of audio. Human clips average 9.4-10.0 s and synthetic clips 7.1 s. Synthetic clips were
added only because the filler breakdown needs them. They are reported separately, and the
headline numbers are human speech only.

| Human source          | Complete | Incomplete |
| --------------------- | -------- | ---------- |
| liva_1                | 597      | 564        |
| midcentury_1          | 144      | 157        |
| human_5               | 47       | 61         |
| human_convcollector_1 | 12       | 18         |

**Per clip:**

1. Decode the FLAC (`claxon`) and mix it to mono. It would be resampled to 16 kHz with
   `audio::resample::resample_to_16khz` if needed, but every clip was already 16 kHz.
2. Transcribe the whole clip with the app's Parakeet provider,
   `asr::transcribe_cpp::TranscribeCppProvider`, using `parakeet-tdt-0.6b-v3-Q5_K_M.gguf` on the
   CPU. This is the same request conversation mode makes. Output that is only the blank
   marker counts as empty, as in the engine.
3. Judge the transcript with `conversation::default_readiness()`, the function conversation
   mode itself uses. It wraps `turn_completion::judge_transcript`.
4. Run Smart Turn on the clip. `audio/smart_turn.rs` is compiled into the example with
   `#[path]` and runs the fp32 model `smart-turn-v3.2-gpu.onnx` through `tract`. It uses the
   last 8 s of the clip.

The answers become end-of-turn waits from the app's own `engine::Limits`: complete 400 ms,
unsure 700 ms, incomplete 1,400 ms. Smart Turn alone gives only two answers: complete
(400 ms) above the threshold and incomplete (1,400 ms) otherwise.

A second run, on 600 of the human clips (300 + 300), added 200 ms of digital silence to the
end of each clip before both models. The app hands Parakeet each phrase with a 200 ms tail
in the same way. This run checks that neither judge depends on exactly where a clip stops.

**Metrics.** Each policy is scored on the clips labelled incomplete and on those labelled
complete:

- **Cut-off rate**: the share of incomplete clips called complete. This is the error that
  makes LocalFlow answer while the user is still talking, so it matters most. It is shown
  with a Wilson 95 % interval.
- **Complete delayed**: the share of complete clips called unsure or incomplete, which adds
  300 or 1,000 ms of wait. "Of which 1,400 ms" is the part called incomplete.
- **Mean wait, complete**: what the user waits for an answer. Lower is better.
- **Mean wait, incomplete**: how long a pause in the middle of a turn can last before the
  turn is cut. Higher is safer.
- **Precision, recall and F1** for "complete", and **accuracy**, on the binary split
  "complete" against "not complete". Unsure and incomplete both count as not complete.
- **Time per decision** for Smart Turn (log-mel features plus the model) and for Parakeet.

**Decision rule, written down before any results.** Smart Turn "clearly helps" if, on human
clips, a policy that uses it does either of these:

- (a) lowers the cut-off rate against the text judge with 95 % intervals that do not
  overlap, while adding no more than 50 ms of mean wait on complete turns;
- (b) cuts the mean wait on complete turns by at least 100 ms without a worse cut-off rate.

Against that weigh the costs: `tract` in the binary, a 32 MB model download, and CPU time at
every pause. Rule (a) is met. Rule (b) is not, because the saving is only 16 ms.

## Results: human clips, every policy

The policies:

- "Text" is today's judge.
- "p" is Smart Turn's probability that the turn is complete.
- "Unsure -> complete if p > 0.9" means that a text "unsure" becomes "complete" when
  p > 0.9, and every other verdict is unchanged.

| Policy                                                                                                         | Cut-off (95 % CI)  | Complete delayed | of which 1,400 ms | Wait, complete | Wait, incomplete | Precision | Recall | F1    | Accuracy |
| -------------------------------------------------------------------------------------------------------------- | ------------------ | ---------------- | ----------------- | -------------- | ---------------- | --------- | ------ | ----- | -------- |
| Fixed 700 ms, no judge                                                                                         | 0.0 % (0.0-0.5)    | 100.0 %          | 0.0 %             | 700            | 700              | -         | 0.000  | -     | 0.500    |
| Text judge (today)                                                                                             | 28.4 % (25.4-31.6) | 7.8 %            | 1.1 %             | 431            | 1014             | 0.765     | 0.922  | 0.836 | 0.819    |
| Smart Turn alone, complete if p > 0.3                                                                          | 6.5 % (5.0-8.4)    | 2.4 %            | 2.4 %             | 424            | 1335             | 0.938     | 0.976  | 0.957 | 0.956    |
| Smart Turn alone, complete if p > 0.5                                                                          | 5.4 % (4.0-7.2)    | 3.1 %            | 3.1 %             | 431            | 1346             | 0.947     | 0.969  | 0.958 | 0.958    |
| Smart Turn alone, complete if p > 0.7                                                                          | 4.0 % (2.8-5.6)    | 4.8 %            | 4.8 %             | 448            | 1360             | 0.960     | 0.953  | 0.956 | 0.956    |
| Smart Turn alone, complete if p > 0.8                                                                          | 3.6 % (2.5-5.2)    | 6.2 %            | 6.2 %             | 462            | 1364             | 0.963     | 0.938  | 0.950 | 0.951    |
| Smart Turn alone, complete if p > 0.9                                                                          | 3.1 % (2.1-4.6)    | 9.1 %            | 9.1 %             | 491            | 1369             | 0.967     | 0.909  | 0.937 | 0.939    |
| Smart Turn alone, complete if p > 0.95                                                                         | 2.4 % (1.5-3.7)    | 11.2 %           | 11.2 %            | 512            | 1376             | 0.974     | 0.887  | 0.929 | 0.932    |
| Smart Turn 3-way: complete if p > 0.8, incomplete if p < 0.2, else unsure                                      | 3.6 % (2.5-5.2)    | 6.2 %            | 2.0 %             | 433            | 1333             | 0.963     | 0.938  | 0.950 | 0.951    |
| Smart Turn 3-way: complete if p > 0.9, incomplete if p < 0.5, else unsure                                      | 3.1 % (2.1-4.6)    | 9.1 %            | 3.1 %             | 449            | 1353             | 0.967     | 0.909  | 0.937 | 0.939    |
| Text; unsure -> complete if p > 0.5                                                                            | 28.7 % (25.7-32.0) | 1.4 %            | 1.1 %             | 412            | 1013             | 0.774     | 0.986  | 0.868 | 0.849    |
| Text; unsure -> complete if p > 0.9                                                                            | 28.5 % (25.5-31.7) | 1.5 %            | 1.1 %             | 412            | 1014             | 0.776     | 0.985  | 0.868 | 0.850    |
| Text; unsure -> incomplete if p < 0.2                                                                          | 28.4 % (25.4-31.6) | 7.8 %            | 1.2 %             | 432            | 1110             | 0.765     | 0.922  | 0.836 | 0.819    |
| Text; unsure -> incomplete if p < 0.5                                                                          | 28.4 % (25.4-31.6) | 7.8 %            | 1.4 %             | 433            | 1114             | 0.765     | 0.922  | 0.836 | 0.819    |
| Text; complete -> unsure if p < 0.1                                                                            | 6.8 % (5.2-8.7)    | 8.8 %            | 1.1 %             | 434            | 1079             | 0.931     | 0.912  | 0.922 | 0.922    |
| Text; complete -> unsure if p < 0.2                                                                            | 6.1 % (4.7-8.0)    | 9.5 %            | 1.1 %             | 436            | 1081             | 0.937     | 0.905  | 0.921 | 0.922    |
| Text; complete -> unsure if p < 0.5                                                                            | 4.1 % (3.0-5.7)    | 10.5 %           | 1.1 %             | 439            | 1087             | 0.956     | 0.895  | 0.924 | 0.927    |
| Text; complete -> unsure if p < 0.2, unsure -> complete if p > 0.9                                             | 6.2 % (4.8-8.1)    | 3.2 %            | 1.1 %             | 418            | 1080             | 0.939     | 0.968  | 0.953 | 0.953    |
| Text; as above, and unsure -> incomplete if p < 0.2                                                            | 6.2 % (4.8-8.1)    | 3.2 %            | 1.2 %             | 418            | 1176             | 0.939     | 0.968  | 0.953 | 0.953    |
| Both agree: complete if text complete and p > 0.5; incomplete if text incomplete and p < 0.5; else unsure      | 4.1 % (3.0-5.7)    | 10.5 %           | 0.1 %             | 432            | 1080             | 0.956     | 0.895  | 0.924 | 0.927    |
| Smart Turn when sure (p > 0.8 complete, p < 0.2 incomplete); text judge otherwise                              | 6.8 % (5.2-8.7)    | 2.2 %            | 2.0 %             | 421            | 1328             | 0.935     | 0.978  | 0.956 | 0.955    |
| Smart Turn at 0.5; text incomplete moves complete to unsure                                                    | 4.5 % (3.3-6.2)    | 4.1 %            | 3.1 %             | 434            | 1349             | 0.955     | 0.959  | 0.957 | 0.957    |
| **Smart Turn at 0.5; text moves it one step (incomplete: complete -> unsure; complete: incomplete -> unsure)** | 4.5 % (3.3-6.2)    | 4.1 %            | 0.4 %             | 415            | 1179             | 0.955     | 0.959  | 0.957 | 0.957    |
| Smart Turn at 0.5, else unsure (never 1,400 ms)                                                                | 5.4 % (4.0-7.2)    | 3.1 %            | 0.0 %             | 409            | 684              | 0.947     | 0.969  | 0.958 | 0.958    |

What the table shows:

- **Using Smart Turn only to promote the text judge's "unsure"** (the cheap combination)
  cuts the wait on finished turns from 431 to 412 ms. It leaves the cut-off rate at 28.5 %,
  because the text judge's mistakes are confident "complete" verdicts, not "unsure" ones.
- **Using it to veto the text judge's "complete"** fixes most cut-offs. At p < 0.5 the
  cut-off rate falls to 4.1 %, but 10.5 % of finished turns are delayed.
- **Letting Smart Turn decide, with the text judge moving the answer one step**, keeps the
  cut-off rate at 4.5 %. Only 4.1 % of finished turns are delayed, and only 0.4 % by the full
  1,400 ms. The difference from Smart Turn alone (5.4 %) is within the intervals. The
  one-step rule is preferred because it falls back gracefully: if Smart Turn misjudges a
  microphone, a turn the text judge reads as finished still waits only 700 ms.
- "Never 1,400 ms" has the lowest wait on finished turns, 409 ms. But it gives mid-turn
  pauses only 700 ms, half as much room as today's "incomplete".

## Synthetic clips, and the filler flags

These 400 clips are text-to-speech, so they are reported apart from the human clips.
Smart Turn's area under the curve is 0.980 here.

| Synthetic clips (200 complete, 200 incomplete)                     | Cut-off (95 % CI)  | Complete delayed | of which 1,400 ms | Wait, complete | Wait, incomplete | F1    |
| ------------------------------------------------------------------ | ------------------ | ---------------- | ----------------- | -------------- | ---------------- | ----- |
| Text judge (today)                                                 | 23.5 % (18.2-29.8) | 12.5 %           | 1.0 %             | 444            | 1112             | 0.829 |
| Smart Turn alone, complete if p > 0.5                              | 6.5 % (3.8-10.8)   | 9.5 %            | 9.5 %             | 495            | 1335             | 0.919 |
| Smart Turn alone, complete if p > 0.9                              | 3.0 % (1.4-6.4)    | 17.5 %           | 17.5 %            | 575            | 1370             | 0.889 |
| Text; complete -> unsure if p < 0.2, unsure -> complete if p > 0.9 | 5.0 % (2.7-9.0)    | 6.0 %            | 1.0 %             | 425            | 1168             | 0.945 |
| Smart Turn at 0.5; text moves it one step                          | 4.0 % (2.0-7.7)    | 10.0 %           | 1.5 %             | 440            | 1199             | 0.928 |

By filler flag. These flags exist only on synthetic rows:

| Subset (complete / incomplete clips)   | Text judge: cut-off | delayed | Smart Turn 0.5: cut-off | delayed | One-step rule: cut-off | delayed | wait, complete |
| -------------------------------------- | ------------------- | ------- | ----------------------- | ------- | ---------------------- | ------- | -------------- |
| Filler mid-turn, `midfiller` (89 / 86) | 26.7 %              | 7.9 %   | 9.3 %                   | 14.6 %  | 7.0 %                  | 15.7 %  | 455 ms         |
| `midfiller` = false (39 / 39)          | 20.5 %              | 17.9 %  | 2.6 %                   | 2.6 %   | 2.6 %                  | 2.6 %   | 426 ms         |
| Ends on a filler, `endfiller` (0 / 75) | 14.7 %              | -       | 1.3 %                   | -       | 1.3 %                  | -       | -              |
| Flags absent (72 / 75)                 | 21.3 %              | 15.3 %  | 5.3 %                   | 6.9 %   | 1.3 %                  | 6.9 %   | 431 ms         |

Fillers in the middle of a turn are where Smart Turn is weakest. It cuts off 9.3 % of those
turns and delays 14.6 % of the finished ones (mean wait 546 ms for Smart Turn alone), and it
is still well ahead of the text judge on cut-offs. A turn that ends on "um" is nearly always
recognised as unfinished: Smart Turn cut off 1 of 75 such clips, the text judge 11 of 75.

## Human clips by source

| Source (complete / incomplete)  | Text judge: cut-off | delayed | Smart Turn 0.5: cut-off | delayed | One-step rule: cut-off | delayed | wait, complete |
| ------------------------------- | ------------------- | ------- | ----------------------- | ------- | ---------------------- | ------- | -------------- |
| liva_1 (597 / 564)              | 28.9 %              | 5.5 %   | 7.1 %                   | 3.5 %   | 6.0 %                  | 4.4 %   | 417 ms         |
| midcentury_1 (144 / 157)        | 35.7 %              | 9.7 %   | 0.6 %                   | 1.4 %   | 0.6 %                  | 2.8 %   | 408 ms         |
| human_5 (47 / 61)               | 8.2 %               | 25.5 %  | 0.0 %                   | 4.3 %   | 0.0 %                  | 6.4 %   | 419 ms         |
| human_convcollector_1 (12 / 18) | 16.7 %              | 25.0 %  | 11.1 %                  | 0.0 %   | 5.6 %                  | 0.0 %   | 400 ms         |

Smart Turn is better on every source. The text judge's behaviour varies far more from
source to source: it cuts off 8-36 % and delays 6-26 %.

By length, on human clips: Smart Turn looks only at the last 8 s. On clips longer than that
(523 incomplete, 546 complete) it cut off 6.3 % and delayed 3.7 %. On shorter clips (277
incomplete, 254 complete) it cut off 3.6 % and delayed 2.0 %. The text judge cut off 29.3 %
and 26.7 % on the same two groups.

## Smart Turn threshold sweep (human clips)

| Threshold | Cut-off | Complete called incomplete | F1    |
| --------- | ------- | -------------------------- | ----- |
| 0.1       | 8.8 %   | 1.2 %                      | 0.952 |
| 0.3       | 6.5 %   | 2.4 %                      | 0.957 |
| 0.5       | 5.4 %   | 3.1 %                      | 0.958 |
| 0.7       | 4.0 %   | 4.8 %                      | 0.956 |
| 0.8       | 3.6 %   | 6.2 %                      | 0.950 |
| 0.9       | 3.1 %   | 9.1 %                      | 0.937 |
| 0.95      | 2.4 %   | 11.2 %                     | 0.929 |
| 0.98      | 1.2 %   | 21.2 %                     | 0.875 |

The probabilities are sharply split. The median is 0.989 on complete clips and 0.006 on
incomplete ones, so the threshold matters little between 0.3 and 0.8.

## Where the two judges agree (human clips)

| Label      | Text judge | Clips | Smart Turn p > 0.5 | Median p |
| ---------- | ---------- | ----- | ------------------ | -------- |
| complete   | complete   | 738   | 97.0 %             | 0.989    |
| complete   | unsure     | 53    | 96.2 %             | 0.989    |
| complete   | incomplete | 9     | 88.9 %             | 0.987    |
| incomplete | complete   | 227   | 14.5 %             | 0.007    |
| incomplete | unsure     | 117   | 2.6 %              | 0.006    |
| incomplete | incomplete | 456   | 1.5 %              | 0.006    |

How Parakeet ended the transcript, and what the text judge did with it (human clips):

| Label      | Transcript ends with | Clips | Judge: complete | unsure | incomplete |
| ---------- | -------------------- | ----- | --------------- | ------ | ---------- |
| complete   | full stop            | 533   | 486             | 41     | 6          |
| complete   | question mark        | 254   | 246             | 6      | 2          |
| complete   | no punctuation       | 13    | 6               | 6      | 1          |
| incomplete | full stop            | 264   | 203             | 28     | 33         |
| incomplete | question mark        | 15    | 11              | 2      | 2          |
| incomplete | no punctuation       | 510   | 13              | 76     | 421        |
| incomplete | nothing transcribed  | 11    | 0               | 11     | 0          |

How each clip ends, measured as the level of its last 100 ms against its loudest 100 ms
(human clips):

| Label      | Last 100 ms   | Clips | Text judge called complete | Smart Turn p > 0.5 |
| ---------- | ------------- | ----- | -------------------------- | ------------------ |
| complete   | above -15 dB  | 59    | 86.4 %                     | 96.6 %             |
| complete   | -15 to -30 dB | 317   | 91.8 %                     | 96.5 %             |
| complete   | -30 to -45 dB | 356   | 93.0 %                     | 97.2 %             |
| complete   | below -45 dB  | 68    | 95.6 %                     | 97.1 %             |
| incomplete | above -15 dB  | 128   | 34.4 %                     | 0.0 %              |
| incomplete | -15 to -30 dB | 315   | 27.3 %                     | 6.3 %              |
| incomplete | -30 to -45 dB | 312   | 26.6 %                     | 6.7 %              |
| incomplete | below -45 dB  | 45    | 31.1 %                     | 4.4 %              |

## With 200 ms of silence added (600 human clips)

These are the same 300 complete and 300 incomplete clips, unpadded and padded:

| Policy                                    | Cut-off, as recorded | Cut-off, +200 ms   | Delayed, as recorded | Delayed, +200 ms | Wait, complete (+200 ms) |
| ----------------------------------------- | -------------------- | ------------------ | -------------------- | ---------------- | ------------------------ |
| Text judge (today)                        | 30.0 % (25.1-35.4)   | 31.0 % (26.0-36.4) | 8.3 %                | 7.0 %            | 428 ms                   |
| Smart Turn alone, complete if p > 0.5     | 5.3 % (3.3-8.5)      | 6.3 % (4.1-9.7)    | 2.7 %                | 3.3 %            | 433 ms                   |
| Smart Turn at 0.5; text moves it one step | 5.0 % (3.1-8.1)      | 6.0 % (3.8-9.3)    | 3.3 %                | 4.0 %            | 414 ms                   |

The padding changed:

- the text judge's verdict on 37 of the 600 clips;
- Parakeet's final punctuation on 56;
- Smart Turn's side of 0.5 on 15, with a mean change in probability of 0.027.

Smart Turn's area under the curve went from 0.994 to 0.991. Neither judge depends on
exactly where the clip stops, and the gap between them is unchanged.

## Time per decision

Smart Turn's time covers the log-mel features and the model, on a private pool of 4
threads. It ran after Parakeet for each clip, not at the same time.

| Run                                                 | Clips | Smart Turn median | 90th pct | 99th pct | Parakeet median (whole clip, mean 9.2-9.7 s) |
| --------------------------------------------------- | ----- | ----------------- | -------- | -------- | -------------------------------------------- |
| Main run, with other builds and a benchmark running | 2,000 | 157 ms            | 388 ms   | 1,026 ms | 1,304 ms                                     |
| Padded run, lighter load                            | 600   | 120 ms            | 156 ms   | 223 ms   | 1,275 ms                                     |

Three other agents were compiling and benchmarking on the same machine throughout, and
during the main run another process held about 8 of the 22 cores. Both rows are therefore
upper bounds. The earlier isolated measurement in PLAN.md was 62-80 ms per decision. Either
way, Smart Turn costs less than transcribing the last phrase, so running the two side by
side should hide it.

## Limitations and what was not measured

- **Same distribution as the training data.** The test set is held out, but it comes from
  the same sources and pipeline as Smart Turn's training data. The text judge was written
  without ever seeing these clips. On LocalFlow's own microphone audio (echo-cancelled,
  different microphones and rooms), Smart Turn's accuracy is unknown until it is tried.
- **"Called complete" is not "cut off".** Whether a turn is actually cut depends on how long
  the user pauses: more than 400 ms after a "complete" verdict, more than 700 or 1,400 ms
  otherwise. The dataset has no pause lengths, so the cut-off rate is the rate of exposure
  to the short wait, not of actual interruptions.
- **Whole clips, not phrases.** Each clip was transcribed in one piece. The app transcribes
  phrase by phrase and joins the text, so the judge sees the same words but Parakeet's
  punctuation may differ.
- **No reference transcripts.** How often Parakeet dropped or "repaired" a word cut off at
  the end could not be measured. Only the clips' final loudness and the transcripts' final
  punctuation were.
- **English only.** The text judge is English-only. Smart Turn is multilingual, but other
  languages were not tested here.
- **Not measured:** the end-to-end latency with Smart Turn running alongside Parakeet in
  the app, memory use, and the app's binary size with `tract` linked in.
- **Two-part main run.** The main run was stopped after 1,400 clips, because a competing
  load slowed it down, and the remaining 600 were run separately. The run code and settings
  were the same; only the report code differed between the two builds. The earlier rough
  check in PLAN.md (18 of 60 cut-off clips called complete) used a different, smaller set of
  clips. On these 800 human incomplete clips the rate is 5.4 %.

## Data attribution

The clips are from "smart-turn-data-v3.2-test" by pipecat-ai,
<https://huggingface.co/datasets/pipecat-ai/smart-turn-data-v3.2-test>, licensed under
CC BY 4.0 (<https://creativecommons.org/licenses/by/4.0/>). They were used only for this
evaluation: decoded, and in one run padded with 200 ms of silence. Nothing was
redistributed, and the downloaded audio was deleted after the runs. The Smart Turn v3.2
model is BSD-2-Clause.
