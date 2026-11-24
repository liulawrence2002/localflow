//! A stand-in for `claude.exe` in the live Windows Terminal check
//! (`coding_agents::agent::tests::live_windows_terminal_passes_the_folder_and_arguments_through`).
//!
//! It writes the folder it was started in and every argument it received to
//! `wt_probe_received.txt` next to itself, then exits. Windows Terminal starts it in a new tab,
//! whose environment may not be the test's, so the output path does not depend on any
//! environment variable.
//!
//! ```text
//! cargo build --release --example wt_probe
//! ```

use std::io::Write;

fn main() {
    let exe = std::env::current_exe().expect("own path");
    let out = exe.with_file_name("wt_probe_received.txt");
    let mut file = std::fs::File::create(&out).expect("create the output file");
    let cwd = std::env::current_dir().expect("working folder");
    writeln!(file, "cwd={}", cwd.display()).expect("write");
    for (index, arg) in std::env::args().enumerate() {
        writeln!(file, "arg{index}=[{arg}]").expect("write");
    }
}
