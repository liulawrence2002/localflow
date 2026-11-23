//! Measures the conversation voice on this machine: model load, time to first audio, and
//! speed relative to real time, for sentences of different lengths.
//!
//! ```text
//! cargo run --release --example tts_bench -- <model dir> [--out <wav dir>] [--passes N]
//! ```
//!
//! Only timings and character counts are printed. `--out` additionally writes each sentence
//! as a WAV file for listening.

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use localflow_lib::tts::{
    phonon::{PhononConfig, PhononProvider},
    stream::{CancelFlag, StreamingTextToSpeech},
};

const SENTENCES: &[&str] = &[
    "Sure, I can help with that.",
    "The meeting moved to three thirty on Thursday, so you have an extra hour.",
    "A mutex is a lock that lets only one thread touch a piece of data at a time, which keeps two threads from changing it at once and leaving it in a broken state.",
    "Here is a quick idea for dinner tonight: roast some vegetables with olive oil and garlic, cook a pot of rice, and fry an egg to put on top.",
];

fn main() {
    let mut args = std::env::args().skip(1);
    let model_dir = PathBuf::from(args.next().expect("usage: tts_bench <model dir>"));
    let mut out_dir: Option<PathBuf> = None;
    let mut passes = 3usize;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--out" => out_dir = args.next().map(PathBuf::from),
            "--passes" => passes = args.next().and_then(|v| v.parse().ok()).unwrap_or(passes),
            other => panic!("unknown argument {other}"),
        }
    }

    let load_started = Instant::now();
    let provider = PhononProvider::new(PhononConfig { model_dir });
    provider.prepare().expect("model should load");
    println!("load_ms={}", load_started.elapsed().as_millis());
    println!("sample_rate={}", provider.sample_rate());

    let cancel = CancelFlag::default();
    println!("pass,sentence,chars,first_audio_ms,total_ms,audio_ms,speed_x_realtime,chunks");
    for pass in 0..passes {
        for (index, sentence) in SENTENCES.iter().enumerate() {
            let started = Instant::now();
            let mut first_audio: Option<Duration> = None;
            let mut samples: Vec<f32> = Vec::new();
            let mut chunks = 0usize;
            provider
                .synthesize_stream(sentence, &cancel, &mut |chunk| {
                    first_audio.get_or_insert_with(|| started.elapsed());
                    chunks += 1;
                    samples.extend_from_slice(chunk);
                })
                .expect("synthesis should succeed");
            let total = started.elapsed();
            let audio_secs = samples.len() as f64 / provider.sample_rate() as f64;
            println!(
                "{pass},{index},{},{},{},{},{:.2},{chunks}",
                sentence.chars().count(),
                first_audio.map(|d| d.as_millis()).unwrap_or(0),
                total.as_millis(),
                (audio_secs * 1000.0).round() as u64,
                audio_secs / total.as_secs_f64(),
            );

            if let (Some(dir), 0) = (out_dir.as_ref(), pass) {
                std::fs::create_dir_all(dir).expect("output dir");
                let path = dir.join(format!("sentence-{index}.wav"));
                let spec = hound::WavSpec {
                    channels: 1,
                    sample_rate: provider.sample_rate(),
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                };
                let mut writer = hound::WavWriter::create(&path, spec).expect("wav file");
                for sample in &samples {
                    writer
                        .write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                        .expect("wav sample");
                }
                writer.finalize().expect("wav finalize");
            }
        }
    }

    // Cancellation: how long after the flag is set does synthesis return?
    let long = SENTENCES[2];
    let started = Instant::now();
    let mut cancelled_at: Option<Instant> = None;
    let cancel = CancelFlag::default();
    let _ = provider.synthesize_stream(long, &cancel, &mut |_| {
        if cancelled_at.is_none() {
            cancel.cancel();
            cancelled_at = Some(Instant::now());
        }
    });
    if let Some(at) = cancelled_at {
        println!(
            "cancel_return_ms={} (first audio at {} ms)",
            at.elapsed().as_millis(),
            at.duration_since(started).as_millis()
        );
    }
}
