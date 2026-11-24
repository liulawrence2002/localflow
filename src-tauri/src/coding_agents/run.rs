//! Running a program in the background with a time limit, a cancel flag and captured output.
//!
//! The program and everything it starts run in one kill-on-close job
//! (`platform::process::ContainedChild`), so cancelling, timing out or LocalFlow exiting
//! ends the whole tree. No shell is involved.

use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{cancel::CancelFlag, platform::process::ContainedChild};

/// How often the run checks for exit, cancellation and the time limit.
const POLL: Duration = Duration::from_millis(25);

/// Environment variables removed for agent processes. `CLAUDECODE` marks a process started
/// from inside a Claude Code session, and would make a nested run behave differently when
/// LocalFlow itself was started from one.
pub const REMOVED_ENV: &[&str] = &["CLAUDECODE"];

#[derive(Debug, Clone, Copy)]
pub struct RunLimits {
    pub time_limit: Duration,
    pub max_stdout: usize,
    pub max_stderr: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Exited(Option<i32>),
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct RunResult {
    pub end: RunEnd,
    pub stdout: String,
    pub stderr: String,
    pub elapsed: Duration,
}

/// Run `program` with `args` in `cwd`, writing `stdin` (as UTF-8) to it, until it exits, the
/// time limit passes or `cancel` is set. On a time-out or cancel the whole process tree is
/// killed. Anything the program left running is also killed once it exits.
pub fn run_captured(
    program: &Path,
    args: &[OsString],
    cwd: Option<&Path>,
    stdin: Option<&str>,
    limits: RunLimits,
    cancel: &CancelFlag,
) -> io::Result<RunResult> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    for name in REMOVED_ENV {
        command.env_remove(name);
    }

    let started = Instant::now();
    let mut child = ContainedChild::spawn_hidden(&mut command)?;

    let writer = match (stdin, child.child_mut().stdin.take()) {
        (Some(text), Some(mut pipe)) => {
            let bytes = text.as_bytes().to_vec();
            // On its own thread so a program that does not read cannot block the run; the
            // pipe closes when the thread ends, which is how the program sees the end.
            Some(thread::spawn(move || {
                let _ = pipe.write_all(&bytes);
            }))
        }
        _ => None,
    };
    let stdout = spawn_reader(child.child_mut().stdout.take(), limits.max_stdout);
    let stderr = spawn_reader(child.child_mut().stderr.take(), limits.max_stderr);

    let end = loop {
        match child.try_wait() {
            Ok(Some(status)) => break RunEnd::Exited(status.code()),
            Ok(None) => {}
            Err(error) => {
                child.kill_tree();
                return Err(error);
            }
        }
        if cancel.is_cancelled() {
            break RunEnd::Cancelled;
        }
        if started.elapsed() >= limits.time_limit {
            break RunEnd::TimedOut;
        }
        thread::sleep(POLL);
    };
    // End the rest of the tree, so pipes held open by a leftover process close and the
    // readers below finish.
    child.kill_tree();

    let stdout = join_reader(stdout);
    let stderr = join_reader(stderr);
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Ok(RunResult {
        end,
        stdout,
        stderr,
        elapsed: started.elapsed(),
    })
}

fn spawn_reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    limit: usize,
) -> Option<JoinHandle<Vec<u8>>> {
    let mut pipe = pipe?;
    Some(thread::spawn(move || {
        let mut kept = Vec::new();
        let mut buffer = [0u8; 8192];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    // Past the limit, keep draining so the program is never blocked on a
                    // full pipe, but stop keeping.
                    let room = limit.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..read.min(room)]);
                }
            }
        }
        kept
    }))
}

fn join_reader(reader: Option<JoinHandle<Vec<u8>>>) -> String {
    reader
        .and_then(|reader| reader.join().ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}
