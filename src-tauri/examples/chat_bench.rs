//! Measures the streaming chat client against local Ollama on its own: time to the first
//! piece of each reply across a multi-turn conversation, and whether cancelling stops the
//! server generating.
//!
//! ```text
//! cargo run --release --example chat_bench -- [model]
//! ```
//!
//! Only timings and counts are printed.

use std::time::{Duration, Instant};

use localflow_lib::{
    cancel::CancelFlag,
    conversation::{chunker::SentenceChunker, memory::ConversationMemory},
    llm::chat::{ChatProvider, OllamaChat},
};

const QUESTIONS: &[&str] = &[
    "What is the capital of France?",
    "Can you explain what a mutex is in simple terms?",
    "Give me a quick idea for dinner tonight.",
    "I have been trying to stay focused while working from home, but I keep getting distracted by my phone. What would you suggest?",
    "What is the capital of Spain?",
    "And of Italy?",
];

fn main() {
    let model = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "llama3.2:3b".to_string());
    // `spoken`: remember each reply the way the voice pipeline cuts and cleans it, instead of
    // exactly as the model wrote it. Shows what that costs in the server's prompt cache.
    let spoken = std::env::args().nth(2).as_deref() == Some("spoken");
    let chat = OllamaChat::new().expect("chat client");
    let load_started = Instant::now();
    chat.prepare(&model)
        .expect("Ollama should be running with the model installed");
    println!(
        "model={model} prepare_ms={}",
        load_started.elapsed().as_millis()
    );

    let mut memory = ConversationMemory::default();
    println!("turn,first_text_ms,total_ms,prompt_tokens,output_tokens,reply_chars");
    for (turn, question) in QUESTIONS.iter().enumerate() {
        memory.push_user(question);
        let started = Instant::now();
        let mut first_text: Option<Duration> = None;
        let mut reply = String::new();
        let stats = chat
            .stream_chat(
                &model,
                &memory.messages(),
                &CancelFlag::default(),
                &mut |piece| {
                    first_text.get_or_insert_with(|| started.elapsed());
                    reply.push_str(piece);
                },
            )
            .expect("reply");
        println!(
            "{turn},{},{},{},{},{}",
            first_text.map(|d| d.as_millis()).unwrap_or(0),
            started.elapsed().as_millis(),
            stats.prompt_tokens,
            stats.output_tokens,
            reply.chars().count()
        );
        if spoken {
            let mut chunker = SentenceChunker::default();
            let mut pieces = chunker.push(&reply);
            pieces.extend(chunker.finish());
            memory.push_assistant(&pieces.join(" "));
        } else {
            memory.push_assistant(&reply);
        }
    }

    // Cancellation: ask for a long reply, cancel after the first piece, then time a short
    // request. If the server kept generating the cancelled reply, the short request would
    // have to wait behind it (Ollama serves one request at a time by default).
    let cancel = CancelFlag::default();
    let mut long = ConversationMemory::default();
    long.push_user("Count slowly from one to two hundred, writing every number as a word.");
    let started = Instant::now();
    chat.stream_chat(&model, &long.messages(), &cancel, &mut |_| cancel.cancel())
        .expect("cancelled reply");
    let cancelled_after = started.elapsed();

    let mut short = ConversationMemory::default();
    short.push_user("Say the single word: ready.");
    let started = Instant::now();
    let mut first_text: Option<Duration> = None;
    chat.stream_chat(
        &model,
        &short.messages(),
        &CancelFlag::default(),
        &mut |_| {
            first_text.get_or_insert_with(|| started.elapsed());
        },
    )
    .expect("short reply");
    println!(
        "cancel_returned_ms={} next_request_first_text_ms={}",
        cancelled_after.as_millis(),
        first_text.map(|d| d.as_millis()).unwrap_or(0)
    );
}
