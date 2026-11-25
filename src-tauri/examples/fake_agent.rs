//! A stand-in for `claude.exe -p`, for testing the background question runner.
//!
//! ```text
//! cargo build --release --example fake_agent
//! cargo test --release --lib coding_agents -- --ignored
//! ```
//!
//! It ignores its arguments except to report them, reads the question from standard input,
//! and behaves as the question tells it to:
//!
//! | Question                    | Behaviour                                                       |
//! | --------------------------- | --------------------------------------------------------------- |
//! | `FAKE:answer <text>`        | a successful result whose answer is `<text>`                    |
//! | `FAKE:echo ...`             | a successful result whose answer is JSON: args, cwd and stdin   |
//! | `FAKE:error <subtype>`      | an error result with that subtype                               |
//! | `FAKE:login`                | an error result saying the user is not signed in                |
//! | `FAKE:junk`                 | output that is not JSON                                         |
//! | `FAKE:exit <code>`          | no output, that exit code                                       |
//! | `FAKE:sleep <seconds>`      | waits, then answers                                             |
//! | `FAKE:tree <file>`          | starts a child copy of itself, writes both process ids to the   |
//! |                             | file, then waits for a minute                                   |
//! | `FAKE:reject-max-turns`     | fails like an old version if `--max-turns` was passed, else     |
//! |                             | answers like `FAKE:echo`                                        |
//!
//! It never makes a network connection and never touches any file but the one it is given.

use std::{
    io::Read,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const CHILD_FLAG: &str = "--fake-agent-child";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some(CHILD_FLAG) {
        thread::sleep(Duration::from_secs(60));
        return;
    }

    let mut question = String::new();
    let _ = std::io::stdin().read_to_string(&mut question);
    let directive = question
        .trim()
        .strip_prefix("FAKE:")
        .unwrap_or("answer fake answer");
    let (mode, rest) = directive.split_once(' ').unwrap_or((directive, ""));

    match mode {
        "answer" => print_result("success", false, rest),
        "echo" => print_result("success", false, &echo(&args, &question)),
        "error" => print_result(rest, true, ""),
        "login" => print_result("success", true, "Invalid API key · Please run /login"),
        "junk" => println!("this is not json {{"),
        "exit" => std::process::exit(rest.trim().parse().unwrap_or(3)),
        "sleep" => {
            thread::sleep(Duration::from_secs(rest.trim().parse().unwrap_or(30)));
            print_result("success", false, "slept");
        }
        "tree" => {
            // Never waited on, on purpose: the test checks that killing the job ends this
            // child too.
            #[allow(clippy::zombie_processes)]
            let child = Command::new(std::env::current_exe().expect("own path"))
                .arg(CHILD_FLAG)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("start child");
            std::fs::write(
                rest.trim(),
                format!("{} {}", std::process::id(), child.id()),
            )
            .expect("write process ids");
            thread::sleep(Duration::from_secs(60));
            print_result("success", false, "tree finished");
        }
        "reject-max-turns" => {
            if args.iter().any(|arg| arg == "--max-turns") {
                eprintln!("error: unknown option '--max-turns'");
                std::process::exit(1);
            }
            print_result("success", false, &echo(&args, &question));
        }
        _ => print_result("success", false, "fake answer"),
    }
}

fn echo(args: &[String], stdin: &str) -> String {
    let cwd = std::env::current_dir()
        .map(|dir| dir.to_string_lossy().into_owned())
        .unwrap_or_default();
    let args: Vec<String> = args.iter().map(|arg| quote(arg)).collect();
    format!(
        "{{\"args\":[{}],\"cwd\":{},\"stdin\":{}}}",
        args.join(","),
        quote(&cwd),
        quote(stdin)
    )
}

fn print_result(subtype: &str, is_error: bool, result: &str) {
    println!(
        "{{\"type\":\"result\",\"subtype\":{},\"is_error\":{is_error},\"duration_ms\":12,\"num_turns\":1,\"result\":{},\"session_id\":\"fake\"}}",
        quote(subtype),
        quote(result)
    );
}

/// A JSON string literal.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
