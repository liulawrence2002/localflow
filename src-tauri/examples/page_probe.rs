//! Reads one window the way "read this page" does and reports what would be spoken.
//!
//! ```text
//! cargo run --release --example page_probe -- "<part of the window title>" [--show-text] [--show-tree]
//! ```
//!
//! Without `--show-text` only counts, kinds, lengths and timings are printed, so it is safe to
//! point at any window. `--show-text` prints the text of each block and is meant for test
//! pages you wrote yourself: do not use it on windows holding anything private.

use std::time::Instant;

use localflow_lib::{
    context::page::{
        cursor::{ReadRequest, ReadingPositions},
        extract::extract,
        policy::{evaluate_capture, CapturePolicy},
        speech::speech_chunks,
        types::{BlockKind, ExtractLimits},
    },
    platform::uia::{window_info, SnapshotOptions, UiaReader},
};
use windows::{
    core::BOOL,
    Win32::{
        Foundation::{HWND, LPARAM},
        UI::WindowsAndMessaging::{
            EnumWindows, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
        },
    },
};

struct Search {
    needle: String,
    found: Option<isize>,
}

unsafe extern "system" fn visit(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam.0 as *mut Search);
    if IsWindowVisible(hwnd).as_bool() {
        let length = GetWindowTextLengthW(hwnd);
        if length > 0 {
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = GetWindowTextW(hwnd, &mut buffer);
            let title = String::from_utf16_lossy(&buffer[..copied.max(0) as usize]).to_lowercase();
            if title.contains(&search.needle) {
                search.found = Some(hwnd.0 as isize);
                return BOOL(0);
            }
        }
    }
    BOOL(1)
}

fn find_window(title_part: &str) -> Option<isize> {
    let mut search = Search {
        needle: title_part.to_lowercase(),
        found: None,
    };
    unsafe {
        let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
    }
    search.found
}

/// One line per node: kind, rectangle, flags, and the name (or only its length).
fn print_node(node: &localflow_lib::context::page::raw::RawNode, depth: usize, show_text: bool) {
    let rect = node
        .rect
        .map(|r| format!("{}x{}@{},{}", r.width(), r.height(), r.left, r.top))
        .unwrap_or_else(|| "no-rect".to_string());
    let name = if show_text {
        node.name.chars().take(48).collect::<String>()
    } else {
        format!("{} chars", node.name.chars().count())
    };
    println!(
        "{}{:?} [{rect}]{}{} type={:?} role={:?} | {name}",
        "  ".repeat(depth),
        node.control,
        if node.is_offscreen { " offscreen" } else { "" },
        if node.is_password { " password" } else { "" },
        node.localized_type,
        node.aria_role,
    );
    for child in &node.children {
        print_node(child, depth + 1, show_text);
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let title_part = args
        .next()
        .expect("usage: page_probe <part of the window title> [--show-text]");
    let flags: Vec<String> = args.collect();
    let show_text = flags.iter().any(|flag| flag == "--show-text");
    let show_tree = flags.iter().any(|flag| flag == "--show-tree");

    let Some(hwnd) = find_window(&title_part) else {
        println!("no visible window has that in its title");
        std::process::exit(2);
    };
    let policy = CapturePolicy::default();
    let info = window_info(hwnd).expect("window facts");
    println!("program={} class={}", info.process_name, info.class_name);
    if let Err(reason) = evaluate_capture(&info, false, &policy) {
        println!("refused before reading: {reason:?}");
        return;
    }

    let reader = UiaReader::new();
    for attempt in 1..=2 {
        let started = Instant::now();
        let tree = match reader.snapshot(hwnd, &SnapshotOptions::default()) {
            Ok(tree) => tree,
            Err(error) => {
                println!("attempt {attempt}: read failed: {error}");
                continue;
            }
        };
        println!(
            "attempt {attempt}: nodes={} truncated={} focused_is_password={} url_known={} read_ms={}",
            tree.node_count,
            tree.truncated,
            tree.focused_is_password,
            !tree.window.url.is_empty(),
            started.elapsed().as_millis()
        );
        if let Err(reason) = evaluate_capture(&tree.window, tree.focused_is_password, &policy) {
            println!("refused after reading: {reason:?}");
            return;
        }

        if attempt == 2 && show_tree {
            if let Some(root) = tree.root.as_ref() {
                print_node(root, 0, show_text);
            }
        }

        let extract_started = Instant::now();
        let snapshot = match extract(&tree, ExtractLimits::default()) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                println!("nothing to read: {error}");
                return;
            }
        };
        println!(
            "blocks={} quality={:?} truncated={} first_visible={} extract_us={}",
            snapshot.blocks.len(),
            snapshot.quality,
            snapshot.truncated,
            snapshot.first_visible(),
            extract_started.elapsed().as_micros()
        );
        if attempt == 1 {
            continue;
        }

        let mut pieces_total = 0;
        for (index, block) in snapshot.blocks.iter().enumerate() {
            let kind = match block.kind {
                BlockKind::Heading(level) => format!("h{level}"),
                other => format!("{other:?}").to_lowercase(),
            };
            let pieces = speech_chunks(block);
            pieces_total += pieces.len();
            if show_text {
                println!(
                    "{index:>3} {kind:<10} visible={:<5} | {}",
                    block.in_viewport,
                    pieces.join(" ¦ ")
                );
            } else {
                println!(
                    "{index:>3} {kind:<10} visible={:<5} chars={} pieces={}",
                    block.in_viewport,
                    block.text.char_count(),
                    pieces.len()
                );
            }
        }
        println!("pieces_total={pieces_total}");

        let mut positions = ReadingPositions::new();
        let plan = positions.plan(&snapshot, ReadRequest::FromHere);
        println!(
            "from-here plan: start_index={} blocks={} reached_end={}",
            plan.start_index,
            plan.blocks.len(),
            plan.reached_end
        );
    }
}
