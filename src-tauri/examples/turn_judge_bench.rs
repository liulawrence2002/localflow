//! Compares LocalFlow's end-of-turn judges on labelled clips: the text judge that reads the
//! transcript (`conversation::turn_completion`), the Smart Turn audio model
//! (`audio::smart_turn`), and simple combinations of the two.
//!
//! ```powershell
//! .\scripts\Get-SmartTurnTestClips.ps1 -OutDir $env:TEMP\smart-turn-data
//! cd src-tauri
//! cargo run --release --example turn_judge_bench -- run ..\.localflow-runtime `
//!   $env:TEMP\smart-turn-data --out results.csv [--pad-ms 200] [--limit 50]
//! cargo run --release --example turn_judge_bench -- report results.csv [--md report.md]
//! ```
//!
//! `run` decodes each clip of the folder's `manifest.csv`, transcribes it with Parakeet
//! (the in-process provider the app uses), judges the transcript exactly as conversation mode
//! does (`default_readiness`), and asks Smart Turn for the probability that the turn is
//! complete. `--pad-ms` appends that much digital silence first, the way the app hands the
//! recogniser each phrase with a short tail of silence. One CSV row is written per clip;
//! `report` turns rows into tables. The CSV and the report hold labels, verdicts, numbers and
//! timings only: no transcript is printed or stored.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::{BufWriter, Write as _},
    path::{Path, PathBuf},
    time::Instant,
};

use localflow_lib::{
    asr::{
        provider::{SpeechToTextProvider, TranscriptionRequest},
        transcribe_cpp::{TranscribeCppConfig, TranscribeCppProvider},
        whisper_text::is_blank_transcript,
    },
    audio::{
        resample::resample_to_16khz,
        smart_turn::{
            SmartTurn, TurnCompletionModel, SMART_TURN_MODEL_FILE, SMART_TURN_SAMPLE_RATE,
        },
    },
    conversation::{assembler::Readiness, default_readiness, engine::Limits},
};

const DEFAULT_ASR_MODEL: &str = "parakeet-tdt-0.6b-v3-Q5_K_M.gguf";
const CSV_HEADER: &str = "file,row_idx,synthetic,endpoint,midfiller,endfiller,source,seconds,tail_db,words,end_mark,judge,smart_turn,smart_turn_ms,asr_ms";

/// The three answers conversation mode acts on, as waits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Complete,
    Unsure,
    Incomplete,
}

impl Verdict {
    fn code(self) -> &'static str {
        match self {
            Verdict::Complete => "complete",
            Verdict::Unsure => "unsure",
            Verdict::Incomplete => "incomplete",
        }
    }

    fn parse(code: &str) -> Result<Self, String> {
        match code {
            "complete" => Ok(Verdict::Complete),
            "unsure" => Ok(Verdict::Unsure),
            "incomplete" => Ok(Verdict::Incomplete),
            other => Err(format!("unknown verdict {other:?}")),
        }
    }
}

/// End-of-turn waits for each verdict, taken from the app's own limits.
#[derive(Debug, Clone, Copy)]
struct Waits {
    complete: f64,
    unsure: f64,
    incomplete: f64,
}

impl Waits {
    fn from_app() -> Self {
        let limits = Limits::default();
        Self {
            complete: limits.ready_wait.as_secs_f64() * 1000.0,
            unsure: limits.unsure_wait.as_secs_f64() * 1000.0,
            incomplete: limits.not_ready_wait.as_secs_f64() * 1000.0,
        }
    }

    fn of(&self, verdict: Verdict) -> f64 {
        match verdict {
            Verdict::Complete => self.complete,
            Verdict::Unsure => self.unsure,
            Verdict::Incomplete => self.incomplete,
        }
    }
}

/// One clip: its labels from the manifest and what each judge made of it.
#[derive(Debug, Clone)]
struct Clip {
    file: String,
    row_idx: u64,
    synthetic: bool,
    complete: bool,
    midfiller: Option<bool>,
    endfiller: Option<bool>,
    source: String,
    seconds: f64,
    tail_db: f64,
    words: usize,
    end_mark: String,
    judge: Verdict,
    smart_turn: f64,
    smart_turn_ms: f64,
    asr_ms: f64,
}

fn flag(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "1",
        Some(false) => "0",
        None => "",
    }
}

fn parse_flag(value: &str) -> Result<Option<bool>, String> {
    match value {
        "1" => Ok(Some(true)),
        "0" => Ok(Some(false)),
        "" => Ok(None),
        other => Err(format!("unexpected flag {other:?}")),
    }
}

impl Clip {
    fn to_csv(&self) -> String {
        format!(
            "{},{},{},{},{},{},{},{:.3},{:.1},{},{},{},{:.5},{:.1},{:.1}",
            self.file,
            self.row_idx,
            u8::from(self.synthetic),
            u8::from(self.complete),
            flag(self.midfiller),
            flag(self.endfiller),
            self.source,
            self.seconds,
            self.tail_db,
            self.words,
            self.end_mark,
            self.judge.code(),
            self.smart_turn,
            self.smart_turn_ms,
            self.asr_ms
        )
    }

    fn from_csv(line: &str) -> Result<Self, String> {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() != 15 {
            return Err(format!("expected 15 fields, got {}", fields.len()));
        }
        let number = |index: usize| -> Result<f64, String> {
            fields[index]
                .parse::<f64>()
                .map_err(|_| format!("field {index} is not a number"))
        };
        Ok(Self {
            file: fields[0].to_string(),
            row_idx: fields[1].parse().map_err(|_| "bad row_idx".to_string())?,
            synthetic: fields[2] == "1",
            complete: fields[3] == "1",
            midfiller: parse_flag(fields[4])?,
            endfiller: parse_flag(fields[5])?,
            source: fields[6].to_string(),
            seconds: number(7)?,
            tail_db: number(8)?,
            words: fields[9].parse().map_err(|_| "bad words".to_string())?,
            end_mark: fields[10].to_string(),
            judge: Verdict::parse(fields[11])?,
            smart_turn: number(12)?,
            smart_turn_ms: number(13)?,
            asr_ms: number(14)?,
        })
    }
}

/// A manifest row. The manifest was written from downloaded data, so every field is checked.
struct ManifestRow {
    file: String,
    row_idx: u64,
    complete: bool,
    midfiller: Option<bool>,
    endfiller: Option<bool>,
    synthetic: bool,
    source: String,
}

fn read_manifest(clip_dir: &Path) -> Result<Vec<ManifestRow>, String> {
    let text = fs::read_to_string(clip_dir.join("manifest.csv"))
        .map_err(|error| format!("cannot read manifest.csv: {error}"))?;
    let mut rows = Vec::new();
    for (number, line) in text.lines().enumerate().skip(1) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let fields: Vec<&str> = line.split(',').collect();
        let [file, row_idx, _id, endpoint, midfiller, endfiller, synthetic, source] =
            fields.as_slice()
        else {
            return Err(format!("manifest line {}: expected 8 fields", number + 1));
        };
        let row_idx: u64 = row_idx
            .parse()
            .map_err(|_| format!("manifest line {}: bad row", number + 1))?;
        if *file != format!("r{row_idx}.flac") {
            return Err(format!(
                "manifest line {}: unexpected file name",
                number + 1
            ));
        }
        let valid_source = !source.is_empty()
            && source.len() <= 40
            && source
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        rows.push(ManifestRow {
            file: file.to_string(),
            row_idx,
            complete: parse_flag(endpoint)?.ok_or("missing endpoint label")?,
            midfiller: parse_flag(midfiller)?,
            endfiller: parse_flag(endfiller)?,
            synthetic: parse_flag(synthetic)?.unwrap_or(false),
            source: if valid_source {
                source.to_string()
            } else {
                "other".to_string()
            },
        });
    }
    Ok(rows)
}

/// Mono samples in [-1, 1] and the sample rate.
fn decode_flac(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = claxon::FlacReader::open(path).map_err(|_| "not a readable FLAC file")?;
    let info = reader.streaminfo();
    if !(1..=32).contains(&info.bits_per_sample) || info.channels == 0 || info.sample_rate == 0 {
        return Err("unsupported FLAC stream".to_string());
    }
    let channels = info.channels as usize;
    let scale = 1.0 / (1u64 << (info.bits_per_sample - 1)) as f32;
    let mut mono = Vec::new();
    let (mut sum, mut in_frame) = (0.0f32, 0usize);
    for sample in reader.samples() {
        sum += sample.map_err(|_| "corrupt FLAC data")? as f32 * scale;
        in_frame += 1;
        if in_frame == channels {
            mono.push(sum / channels as f32);
            sum = 0.0;
            in_frame = 0;
        }
    }
    Ok((mono, info.sample_rate))
}

fn rms(samples: &[f32]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| f64::from(*s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
}

/// Loudness of the clip's last 100 ms relative to its loudest 100 ms, in dB. Near 0 means
/// the clip stops while someone is still speaking; -30 or lower means it ends in silence.
fn tail_db(samples: &[f32]) -> f64 {
    let window = SMART_TURN_SAMPLE_RATE as usize / 10;
    if samples.len() < window {
        return 0.0;
    }
    let loudest = samples.chunks(window).map(rms).fold(0.0, f64::max);
    if loudest <= 0.0 {
        return 0.0;
    }
    let tail = rms(&samples[samples.len() - window..]).max(1e-9);
    20.0 * (tail / loudest).log10()
}

/// How the transcript ends, as a category: kept instead of the text itself.
fn end_mark(text: &str) -> &'static str {
    match text.trim_end().chars().last() {
        None => "empty",
        Some('.') => "period",
        Some('?') => "question",
        Some('!') => "exclamation",
        Some(',') => "comma",
        Some(c) if c.is_alphanumeric() => "none",
        Some(_) => "other",
    }
}

struct RunArgs {
    runtime: PathBuf,
    clips: PathBuf,
    out: PathBuf,
    pad_ms: u32,
    limit: Option<usize>,
    asr_model: String,
}

fn run(args: RunArgs) -> Result<Vec<Clip>, String> {
    let manifest = read_manifest(&args.clips)?;
    let manifest: Vec<ManifestRow> = match args.limit {
        // Alternate labels so a short trial run still sees both.
        Some(limit) => {
            let (complete, incomplete): (Vec<_>, Vec<_>) =
                manifest.into_iter().partition(|row| row.complete);
            let mut mixed = Vec::new();
            let mut a = complete.into_iter();
            let mut b = incomplete.into_iter();
            while mixed.len() < limit {
                match (a.next(), b.next()) {
                    (None, None) => break,
                    (x, y) => mixed.extend(x.into_iter().chain(y)),
                }
            }
            mixed.truncate(limit);
            mixed
        }
        None => manifest,
    };

    let asr = TranscribeCppProvider::new(TranscribeCppConfig {
        model_path: args.runtime.join("models").join(&args.asr_model),
    });
    asr.prepare().map_err(|error| error.to_string())?;
    let mut smart_turn = SmartTurn::load(&args.runtime.join("models").join(SMART_TURN_MODEL_FILE))?;
    let readiness = default_readiness();
    let pad = vec![0.0f32; (SMART_TURN_SAMPLE_RATE * args.pad_ms / 1000) as usize];
    eprintln!(
        "{} clips; asr {} ({}); turn model {}; pad {} ms",
        manifest.len(),
        asr.describe(),
        asr.backend().unwrap_or_default(),
        smart_turn.id(),
        args.pad_ms
    );

    let mut out = BufWriter::new(
        fs::File::create(&args.out).map_err(|error| format!("cannot create output: {error}"))?,
    );
    writeln!(out, "{CSV_HEADER}").map_err(|error| error.to_string())?;

    let mut clips = Vec::new();
    let mut warmed = false;
    let mut skipped = 0usize;
    for (index, row) in manifest.iter().enumerate() {
        let path = args.clips.join(&row.file);
        let (samples, rate) = match decode_flac(&path) {
            Ok(decoded) => decoded,
            Err(reason) => {
                eprintln!("skip {}: {reason}", row.file);
                skipped += 1;
                continue;
            }
        };
        let mut samples = if rate == SMART_TURN_SAMPLE_RATE {
            samples
        } else {
            resample_to_16khz(&samples, rate)
        };
        if samples.is_empty() {
            skipped += 1;
            continue;
        }
        let seconds = samples.len() as f64 / f64::from(SMART_TURN_SAMPLE_RATE);
        let tail = tail_db(&samples);
        samples.extend_from_slice(&pad);

        let request = TranscriptionRequest {
            samples: &samples,
            language: "en",
            initial_prompt: None,
        };
        if !warmed {
            // The first run of each model pays one-off costs; keep them out of the timings.
            asr.transcribe(&request)
                .map_err(|error| error.to_string())?;
            smart_turn.probability_complete(&samples)?;
            warmed = true;
        }
        let started = Instant::now();
        let text = match asr.transcribe(&request) {
            Ok(transcription) if is_blank_transcript(&transcription.text) => String::new(),
            Ok(transcription) => transcription.text,
            Err(error) => {
                eprintln!("skip {}: {error}", row.file);
                skipped += 1;
                continue;
            }
        };
        let asr_ms = started.elapsed().as_secs_f64() * 1000.0;
        let text = text.trim();
        let judge = match readiness(text) {
            Readiness::Ready => Verdict::Complete,
            Readiness::Unsure => Verdict::Unsure,
            Readiness::NotReady => Verdict::Incomplete,
        };

        let started = Instant::now();
        let probability = smart_turn.probability_complete(&samples)?;
        let smart_turn_ms = started.elapsed().as_secs_f64() * 1000.0;

        let clip = Clip {
            file: row.file.clone(),
            row_idx: row.row_idx,
            synthetic: row.synthetic,
            complete: row.complete,
            midfiller: row.midfiller,
            endfiller: row.endfiller,
            source: row.source.clone(),
            seconds,
            tail_db: tail,
            words: text.split_whitespace().count(),
            end_mark: end_mark(text).to_string(),
            judge,
            smart_turn: f64::from(probability),
            smart_turn_ms,
            asr_ms,
        };
        writeln!(out, "{}", clip.to_csv()).map_err(|error| error.to_string())?;
        clips.push(clip);
        if (index + 1) % 100 == 0 {
            out.flush().map_err(|error| error.to_string())?;
            eprintln!("{} / {}", index + 1, manifest.len());
        }
    }
    out.flush().map_err(|error| error.to_string())?;
    eprintln!("done: {} clips, {} skipped", clips.len(), skipped);
    Ok(clips)
}

fn read_results(path: &Path) -> Result<Vec<Clip>, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("cannot read results: {error}"))?;
    text.lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .enumerate()
        .map(|(index, line)| Clip::from_csv(line).map_err(|e| format!("row {}: {e}", index + 2)))
        .collect()
}

// ---------------------------------------------------------------------------------------
// Policies and metrics

type Decide = Box<dyn Fn(&Clip) -> Verdict>;

struct Policy {
    name: String,
    /// Shown in the compact per-subset tables as well as the full ones.
    key: bool,
    decide: Decide,
}

fn policy(
    name: impl Into<String>,
    key: bool,
    decide: impl Fn(&Clip) -> Verdict + 'static,
) -> Policy {
    Policy {
        name: name.into(),
        key,
        decide: Box::new(decide),
    }
}

fn smart_turn_says(clip: &Clip, threshold: f64) -> Verdict {
    if clip.smart_turn > threshold {
        Verdict::Complete
    } else {
        Verdict::Incomplete
    }
}

fn policies() -> Vec<Policy> {
    use Verdict::*;
    let mut list = vec![
        policy("Fixed 700 ms, no judge", false, |_| Unsure),
        policy("Text judge (today)", true, |c| c.judge),
    ];
    for t in [0.3, 0.5, 0.7, 0.8, 0.9, 0.95] {
        list.push(policy(
            format!("Smart Turn alone, complete if p > {t}"),
            t == 0.5 || t == 0.9,
            move |c| smart_turn_says(c, t),
        ));
    }
    for (low, high) in [(0.2, 0.8), (0.5, 0.9)] {
        list.push(policy(
            format!("Smart Turn 3-way: complete if p > {high}, incomplete if p < {low}"),
            false,
            move |c| {
                if c.smart_turn > high {
                    Complete
                } else if c.smart_turn < low {
                    Incomplete
                } else {
                    Unsure
                }
            },
        ));
    }
    for t in [0.5, 0.8, 0.9, 0.95] {
        list.push(policy(
            format!("Text; unsure -> complete if p > {t}"),
            t == 0.9,
            move |c| match c.judge {
                Unsure if c.smart_turn > t => Complete,
                other => other,
            },
        ));
    }
    for t in [0.1, 0.2, 0.5] {
        list.push(policy(
            format!("Text; unsure -> incomplete if p < {t}"),
            t == 0.2,
            move |c| match c.judge {
                Unsure if c.smart_turn < t => Incomplete,
                other => other,
            },
        ));
    }
    for t in [0.1, 0.2, 0.5] {
        list.push(policy(
            format!("Text; complete -> unsure if p < {t}"),
            t == 0.2,
            move |c| match c.judge {
                Complete if c.smart_turn < t => Unsure,
                other => other,
            },
        ));
    }
    list.push(policy(
        "Text; complete -> unsure if p < 0.2, unsure -> complete if p > 0.9",
        true,
        |c| match c.judge {
            Complete if c.smart_turn < 0.2 => Unsure,
            Unsure if c.smart_turn > 0.9 => Complete,
            other => other,
        },
    ));
    list.push(policy(
        "Text; complete -> unsure if p < 0.2, unsure -> complete if p > 0.9, unsure -> incomplete if p < 0.2",
        false,
        |c| match c.judge {
            Complete if c.smart_turn < 0.2 => Unsure,
            Unsure if c.smart_turn > 0.9 => Complete,
            Unsure if c.smart_turn < 0.2 => Incomplete,
            other => other,
        },
    ));
    list.push(policy(
        "Both agree: complete if text complete and p > 0.5; incomplete if text incomplete and p < 0.5; else unsure",
        false,
        |c| match (c.judge, c.smart_turn > 0.5) {
            (Complete, true) => Complete,
            (Incomplete, false) => Incomplete,
            _ => Unsure,
        },
    ));
    // Smart Turn first, the text judge as the second opinion.
    list.push(policy(
        "Smart Turn decides when sure (p > 0.8 complete, p < 0.2 incomplete); text judge otherwise",
        false,
        |c| {
            if c.smart_turn > 0.8 {
                Complete
            } else if c.smart_turn < 0.2 {
                Incomplete
            } else {
                c.judge
            }
        },
    ));
    list.push(policy(
        "Smart Turn > 0.5 complete, else incomplete; text incomplete overrides complete to unsure",
        true,
        |c| match (smart_turn_says(c, 0.5), c.judge) {
            (Complete, Incomplete) => Unsure,
            (verdict, _) => verdict,
        },
    ));
    list.push(policy(
        "Smart Turn > 0.5 complete, else incomplete; the text judge moves either one step (text incomplete: complete -> unsure; text complete: incomplete -> unsure)",
        true,
        |c| match (smart_turn_says(c, 0.5), c.judge) {
            (Complete, Incomplete) | (Incomplete, Complete) => Unsure,
            (verdict, _) => verdict,
        },
    ));
    list.push(policy(
        "Smart Turn > 0.5 complete, else unsure (700 ms, never 1,400)",
        false,
        |c| match smart_turn_says(c, 0.5) {
            Complete => Complete,
            _ => Unsure,
        },
    ));
    list
}

#[derive(Debug, Default, Clone, Copy)]
struct Counts {
    /// `[label][verdict]`: label 0 complete, 1 incomplete; verdict 0 complete, 1 unsure,
    /// 2 incomplete.
    table: [[usize; 3]; 2],
}

impl Counts {
    fn add(&mut self, complete: bool, verdict: Verdict) {
        let label = usize::from(!complete);
        let column = match verdict {
            Verdict::Complete => 0,
            Verdict::Unsure => 1,
            Verdict::Incomplete => 2,
        };
        self.table[label][column] += 1;
    }

    fn n_complete(&self) -> usize {
        self.table[0].iter().sum()
    }

    fn n_incomplete(&self) -> usize {
        self.table[1].iter().sum()
    }

    fn mean_wait(&self, label: usize, waits: Waits) -> f64 {
        let n: usize = self.table[label].iter().sum();
        if n == 0 {
            return f64::NAN;
        }
        let verdicts = [Verdict::Complete, Verdict::Unsure, Verdict::Incomplete];
        self.table[label]
            .iter()
            .zip(verdicts)
            .map(|(clips, verdict)| *clips as f64 * waits.of(verdict))
            .sum::<f64>()
            / n as f64
    }
}

fn ratio(a: usize, b: usize) -> f64 {
    if b == 0 {
        f64::NAN
    } else {
        a as f64 / b as f64
    }
}

/// Wilson 95% interval for `hits` out of `n`.
fn wilson(hits: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (f64::NAN, f64::NAN);
    }
    let z = 1.96f64;
    let n = n as f64;
    let p = hits as f64 / n;
    let centre = (p + z * z / (2.0 * n)) / (1.0 + z * z / n);
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / (1.0 + z * z / n);
    ((centre - half).max(0.0), (centre + half).min(1.0))
}

fn pct(value: f64) -> String {
    if value.is_nan() {
        "-".to_string()
    } else {
        format!("{:.1}%", value * 100.0)
    }
}

fn ms(value: f64) -> String {
    if value.is_nan() {
        "-".to_string()
    } else {
        format!("{value:.0}")
    }
}

fn num(value: f64) -> String {
    if value.is_nan() {
        "-".to_string()
    } else {
        format!("{value:.3}")
    }
}

struct Summary {
    cutoff: f64,
    cutoff_ci: (f64, f64),
    delayed: f64,
    longest: f64,
    wait_complete: f64,
    wait_incomplete: f64,
    precision: f64,
    recall: f64,
    f1: f64,
    accuracy: f64,
}

fn summarise(counts: &Counts, waits: Waits) -> Summary {
    let [c, i] = counts.table;
    let (n_c, n_i) = (counts.n_complete(), counts.n_incomplete());
    let recall = ratio(c[0], n_c);
    let precision = ratio(c[0], c[0] + i[0]);
    Summary {
        cutoff: ratio(i[0], n_i),
        cutoff_ci: wilson(i[0], n_i),
        delayed: ratio(c[1] + c[2], n_c),
        longest: ratio(c[2], n_c),
        wait_complete: counts.mean_wait(0, waits),
        wait_incomplete: counts.mean_wait(1, waits),
        precision,
        recall,
        f1: if precision + recall > 0.0 {
            2.0 * precision * recall / (precision + recall)
        } else {
            f64::NAN
        },
        accuracy: ratio(c[0] + i[1] + i[2], n_c + n_i),
    }
}

fn count(clips: &[&Clip], decide: &dyn Fn(&Clip) -> Verdict) -> Counts {
    let mut counts = Counts::default();
    for clip in clips {
        counts.add(clip.complete, decide(clip));
    }
    counts
}

fn full_table(out: &mut String, clips: &[&Clip], policies: &[Policy], waits: Waits) {
    let _ = writeln!(
        out,
        "| Policy | Cut-off (95% CI) | Complete delayed | of which {:.0} ms | Wait, complete | Wait, incomplete | Precision | Recall | F1 | Accuracy |",
        waits.incomplete
    );
    let _ = writeln!(
        out,
        "| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |"
    );
    for policy in policies {
        let s = summarise(&count(clips, &policy.decide), waits);
        let _ = writeln!(
            out,
            "| {} | {} ({}-{}) | {} | {} | {} | {} | {} | {} | {} | {} |",
            policy.name,
            pct(s.cutoff),
            pct(s.cutoff_ci.0),
            pct(s.cutoff_ci.1),
            pct(s.delayed),
            pct(s.longest),
            ms(s.wait_complete),
            ms(s.wait_incomplete),
            num(s.precision),
            num(s.recall),
            num(s.f1),
            num(s.accuracy)
        );
    }
}

fn compact_table(out: &mut String, clips: &[&Clip], policies: &[Policy], waits: Waits) {
    let mut counts = Counts::default();
    for clip in clips {
        counts.add(clip.complete, Verdict::Unsure);
    }
    let _ = writeln!(
        out,
        "{} clips: {} complete, {} incomplete.\n",
        clips.len(),
        counts.n_complete(),
        counts.n_incomplete()
    );
    let _ = writeln!(
        out,
        "| Policy | Cut-off (95% CI) | Complete delayed | Wait, complete | Wait, incomplete | F1 |"
    );
    let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- |");
    for policy in policies.iter().filter(|policy| policy.key) {
        let s = summarise(&count(clips, &policy.decide), waits);
        let _ = writeln!(
            out,
            "| {} | {} ({}-{}) | {} | {} | {} | {} |",
            policy.name,
            pct(s.cutoff),
            pct(s.cutoff_ci.0),
            pct(s.cutoff_ci.1),
            pct(s.delayed),
            ms(s.wait_complete),
            ms(s.wait_incomplete),
            num(s.f1)
        );
    }
}

fn percentile(values: &mut [f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let index = ((values.len() - 1) as f64 * fraction).round() as usize;
    values[index]
}

/// Area under the ROC curve of Smart Turn's probability for "complete" (Mann-Whitney).
fn auc(clips: &[&Clip]) -> f64 {
    let mut scored: Vec<(f64, bool)> = clips.iter().map(|c| (c.smart_turn, c.complete)).collect();
    scored.sort_by(|a, b| a.0.total_cmp(&b.0));
    let positives = scored.iter().filter(|(_, complete)| *complete).count();
    let negatives = scored.len() - positives;
    if positives == 0 || negatives == 0 {
        return f64::NAN;
    }
    // Average ranks over ties.
    let mut rank_sum = 0.0;
    let mut start = 0;
    while start < scored.len() {
        let mut end = start;
        while end + 1 < scored.len() && scored[end + 1].0 == scored[start].0 {
            end += 1;
        }
        let rank = (start + end) as f64 / 2.0 + 1.0;
        rank_sum += rank * scored[start..=end].iter().filter(|(_, c)| *c).count() as f64;
        start = end + 1;
    }
    (rank_sum - (positives * (positives + 1)) as f64 / 2.0) / (positives * negatives) as f64
}

fn report(clips: &[Clip]) -> String {
    let waits = Waits::from_app();
    let policies = policies();
    let mut out = String::new();
    let all: Vec<&Clip> = clips.iter().collect();
    let human: Vec<&Clip> = all.iter().copied().filter(|c| !c.synthetic).collect();
    let synthetic: Vec<&Clip> = all.iter().copied().filter(|c| c.synthetic).collect();

    let _ = writeln!(
        out,
        "Waits: complete {:.0} ms, unsure {:.0} ms, incomplete {:.0} ms (conversation::engine::Limits).\n",
        waits.complete, waits.unsure, waits.incomplete
    );

    // Sample.
    let _ = writeln!(out, "## Sample\n");
    let _ = writeln!(
        out,
        "| Group | Label | Clips | Mean length (s) | Median last-100-ms level (dB) | Empty transcripts |"
    );
    let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- |");
    for (group, subset) in [("Human", &human), ("Synthetic", &synthetic)] {
        for (label, complete) in [("complete", true), ("incomplete", false)] {
            let chosen: Vec<&Clip> = subset
                .iter()
                .copied()
                .filter(|c| c.complete == complete)
                .collect();
            if chosen.is_empty() {
                continue;
            }
            let mut tails: Vec<f64> = chosen.iter().map(|c| c.tail_db).collect();
            let _ = writeln!(
                out,
                "| {group} | {label} | {} | {:.2} | {:.1} | {} |",
                chosen.len(),
                chosen.iter().map(|c| c.seconds).sum::<f64>() / chosen.len() as f64,
                percentile(&mut tails, 0.5),
                chosen.iter().filter(|c| c.end_mark == "empty").count()
            );
        }
    }
    let mut sources: BTreeMap<&str, [usize; 2]> = BTreeMap::new();
    for clip in &human {
        sources.entry(clip.source.as_str()).or_default()[usize::from(!clip.complete)] += 1;
    }
    let _ = writeln!(out, "\nHuman clips by source (complete / incomplete):\n");
    let _ = writeln!(out, "| Source | Complete | Incomplete |");
    let _ = writeln!(out, "| --- | --- | --- |");
    for (source, [c, i]) in &sources {
        let _ = writeln!(out, "| {source} | {c} | {i} |");
    }

    for (title, subset) in [("Human clips", &human), ("Synthetic clips", &synthetic)] {
        if subset.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n## {title}: every policy\n");
        let _ = writeln!(
            out,
            "Smart Turn AUC (complete vs incomplete): {}.\n",
            num(auc(subset))
        );
        full_table(&mut out, subset, &policies, waits);
    }

    // Filler flags exist only on synthetic rows.
    let _ = writeln!(out, "\n## Synthetic clips by filler flag\n");
    let filler_subsets: [(&str, Box<dyn Fn(&Clip) -> bool>); 4] = [
        (
            "Filler word mid-turn (midfiller)",
            Box::new(|c| c.midfiller == Some(true)),
        ),
        (
            "No mid-turn filler (midfiller = false)",
            Box::new(|c| c.midfiller == Some(false)),
        ),
        (
            "Ends on a filler word (endfiller; all incomplete)",
            Box::new(|c| c.endfiller == Some(true)),
        ),
        (
            "Filler flags absent",
            Box::new(|c| c.midfiller.is_none() && c.endfiller.is_none()),
        ),
    ];
    for (title, keep) in &filler_subsets {
        let chosen: Vec<&Clip> = synthetic.iter().copied().filter(|c| keep(c)).collect();
        if chosen.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n### {title}\n");
        compact_table(&mut out, &chosen, &policies, waits);
    }

    let _ = writeln!(out, "\n## Human clips by source\n");
    for source in sources.keys() {
        let chosen: Vec<&Clip> = human
            .iter()
            .copied()
            .filter(|c| c.source == *source)
            .collect();
        let _ = writeln!(out, "\n### {source}\n");
        compact_table(&mut out, &chosen, &policies, waits);
    }

    // Threshold sweep for Smart Turn alone.
    for (title, subset) in [("Human", &human), ("Synthetic", &synthetic)] {
        if subset.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n## Smart Turn alone, threshold sweep ({title})\n");
        let _ = writeln!(
            out,
            "| Threshold | Cut-off | Complete called incomplete | F1 | Accuracy |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
        for t in [
            0.05, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.98, 0.99,
        ] {
            let s = summarise(
                &count(subset, &move |c: &Clip| smart_turn_says(c, t)),
                waits,
            );
            let _ = writeln!(
                out,
                "| {t} | {} | {} | {} | {} |",
                pct(s.cutoff),
                pct(s.delayed),
                num(s.f1),
                num(s.accuracy)
            );
        }
    }

    // How the text judge and Smart Turn agree.
    for (title, subset) in [("Human", &human), ("Synthetic", &synthetic)] {
        if subset.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n## Text judge against Smart Turn at 0.5 ({title})\n");
        let _ = writeln!(
            out,
            "| Label | Text judge | Clips | Smart Turn p > 0.5 | Median p |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
        for (label, complete) in [("complete", true), ("incomplete", false)] {
            for verdict in [Verdict::Complete, Verdict::Unsure, Verdict::Incomplete] {
                let chosen: Vec<&Clip> = subset
                    .iter()
                    .copied()
                    .filter(|c| c.complete == complete && c.judge == verdict)
                    .collect();
                let mut probabilities: Vec<f64> = chosen.iter().map(|c| c.smart_turn).collect();
                let above = chosen.iter().filter(|c| c.smart_turn > 0.5).count();
                let _ = writeln!(
                    out,
                    "| {label} | {} | {} | {} | {} |",
                    verdict.code(),
                    chosen.len(),
                    pct(ratio(above, chosen.len())),
                    num(percentile(&mut probabilities, 0.5))
                );
            }
        }
    }

    // How transcripts end, and what the text judge did with each ending.
    for (title, subset) in [("Human", &human), ("Synthetic", &synthetic)] {
        if subset.is_empty() {
            continue;
        }
        let _ = writeln!(out, "\n## How Parakeet ended the transcript ({title})\n");
        let _ = writeln!(
            out,
            "| Label | Transcript ends with | Clips | Judge: complete | unsure | incomplete |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- |");
        for (label, complete) in [("complete", true), ("incomplete", false)] {
            let mut marks: BTreeMap<&str, [usize; 3]> = BTreeMap::new();
            for clip in subset.iter().filter(|c| c.complete == complete) {
                let column = match clip.judge {
                    Verdict::Complete => 0,
                    Verdict::Unsure => 1,
                    Verdict::Incomplete => 2,
                };
                marks.entry(clip.end_mark.as_str()).or_default()[column] += 1;
            }
            for (mark, [c, u, i]) in marks {
                let _ = writeln!(
                    out,
                    "| {label} | {mark} | {} | {c} | {u} | {i} |",
                    c + u + i
                );
            }
        }
        // Cut-off audio: does the clip stop while sound is still going?
        let _ = writeln!(
            out,
            "\nLevel of the last 100 ms against the loudest 100 ms of the clip. Near 0 dB the clip stops on sound; below -40 dB it ends in near silence. 'Called complete' is the share each judge called complete.\n"
        );
        let _ = writeln!(
            out,
            "| Label | Last 100 ms | Clips | Text judge called complete | Smart Turn p > 0.5 |"
        );
        let _ = writeln!(out, "| --- | --- | --- | --- | --- |");
        for (label, complete) in [("complete", true), ("incomplete", false)] {
            for (band, low, high) in [
                ("above -15 dB", -15.0, f64::INFINITY),
                ("-15 to -30 dB", -30.0, -15.0),
                ("-30 to -45 dB", -45.0, -30.0),
                ("below -45 dB", f64::NEG_INFINITY, -45.0),
            ] {
                let chosen: Vec<&Clip> = subset
                    .iter()
                    .copied()
                    .filter(|c| c.complete == complete && c.tail_db > low && c.tail_db <= high)
                    .collect();
                let text = chosen
                    .iter()
                    .filter(|c| c.judge == Verdict::Complete)
                    .count();
                let audio = chosen.iter().filter(|c| c.smart_turn > 0.5).count();
                let _ = writeln!(
                    out,
                    "| {label} | {band} | {} | {} | {} |",
                    chosen.len(),
                    pct(ratio(text, chosen.len())),
                    pct(ratio(audio, chosen.len()))
                );
            }
        }
    }

    // Timings.
    let _ = writeln!(out, "\n## Time per decision\n");
    let _ = writeln!(
        out,
        "| Stage | Clips | Median ms | 90th percentile | 99th percentile | Max |"
    );
    let _ = writeln!(out, "| --- | --- | --- | --- | --- | --- |");
    for (stage, values) in [
        (
            "Smart Turn (features + model)",
            all.iter().map(|c| c.smart_turn_ms).collect::<Vec<_>>(),
        ),
        (
            "Parakeet transcription of the clip",
            all.iter().map(|c| c.asr_ms).collect(),
        ),
    ] {
        let mut values = values;
        let _ = writeln!(
            out,
            "| {stage} | {} | {} | {} | {} | {} |",
            values.len(),
            ms(percentile(&mut values, 0.5)),
            ms(percentile(&mut values, 0.9)),
            ms(percentile(&mut values, 0.99)),
            ms(percentile(&mut values, 1.0))
        );
    }
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("run") => parse_run(args).and_then(run).map(|clips| report(&clips)),
        Some("report") => {
            let mut files = Vec::new();
            let mut md = None;
            while let Some(arg) = args.next() {
                if arg == "--md" {
                    md = args.next().map(PathBuf::from);
                } else {
                    files.push(PathBuf::from(arg));
                }
            }
            files
                .iter()
                .map(|file| read_results(file))
                .collect::<Result<Vec<_>, _>>()
                .map(|sets| report(&sets.concat()))
                .and_then(|text| match md {
                    Some(path) => fs::write(&path, &text)
                        .map(|_| text)
                        .map_err(|error| format!("cannot write report: {error}")),
                    None => Ok(text),
                })
        }
        _ => Err("usage: turn_judge_bench run <runtime dir> <clip dir> --out <csv> [--pad-ms N] [--limit N] [--asr-model FILE] | report <csv>... [--md FILE]".to_string()),
    };
    match result {
        Ok(text) => println!("{text}"),
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    }
}

fn parse_run(mut args: impl Iterator<Item = String>) -> Result<RunArgs, String> {
    let runtime = args.next().ok_or("missing runtime dir")?.into();
    let clips = args.next().ok_or("missing clip dir")?.into();
    let mut parsed = RunArgs {
        runtime,
        clips,
        out: PathBuf::from("turn_judge_results.csv"),
        pad_ms: 0,
        limit: None,
        asr_model: DEFAULT_ASR_MODEL.to_string(),
    };
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or(format!("{arg} needs a value"));
        match arg.as_str() {
            "--out" => parsed.out = value()?.into(),
            "--pad-ms" => parsed.pad_ms = value()?.parse().map_err(|_| "bad --pad-ms")?,
            "--limit" => parsed.limit = Some(value()?.parse().map_err(|_| "bad --limit")?),
            "--asr-model" => parsed.asr_model = value()?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(parsed)
}
