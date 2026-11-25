//! The app's log file.
//!
//! The release build has no console, so without a file its log went nowhere. Lines go to
//! `localflow.log` in the app's log folder; when the next line would take it past
//! [`LOG_MAX_BYTES`] it is renamed to `localflow.log.1` (replacing the previous one) and a new
//! file is started, so at most two files of 1 MB each exist.
//!
//! The log carries what the rest of the app already restricts itself to logging: counts,
//! timings, phases and fixed messages, never transcripts, page text, questions or answers.
//! As a second line of defence every line has user-profile folders replaced by
//! `<user folder>` before it is written.
//!
//! Lines logged before the log folder is known (the first moments of start-up) are kept in
//! memory, up to a limit, and written once it is.

use std::{
    cell::Cell,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing_subscriber::fmt::MakeWriter;

use super::redact::redact_user_paths;

pub const LOG_FILE_NAME: &str = "localflow.log";
/// Size at which the file is rotated.
pub const LOG_MAX_BYTES: u64 = 1024 * 1024;
/// Most bytes kept in memory before the log folder is known.
const EARLY_LIMIT: usize = 64 * 1024;

/// A file that is moved aside to `<name>.1` when it would grow past `max_bytes`.
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    size: u64,
}

impl RotatingFile {
    pub fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        Ok(Self {
            path,
            max_bytes: max_bytes.max(1),
            file: Some(file),
            size,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Where the previous file is kept.
    pub fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_os_string();
        name.push(".1");
        PathBuf::from(name)
    }

    /// Append one record. A record is never split across two files, and one longer than
    /// the cap is cut to it.
    pub fn write_record(&mut self, record: &[u8]) -> io::Result<()> {
        let record = &record[..record.len().min(self.max_bytes as usize)];
        if self.size > 0 && self.size + record.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        if let Some(file) = self.file.as_mut() {
            file.write_all(record)?;
            self.size += record.len() as u64;
        }
        Ok(())
    }

    fn rotate(&mut self) -> io::Result<()> {
        // Closed first: Windows cannot rename a file this process still holds open for
        // writing without sharing.
        self.file = None;
        let rotated = self.rotated_path();
        let _ = fs::remove_file(&rotated);
        let file = match fs::rename(&self.path, &rotated) {
            Ok(()) => OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?,
            // Something else holds the file: start it again instead, which keeps the cap.
            Err(_) => OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.path)?,
        };
        self.file = Some(file);
        self.size = 0;
        Ok(())
    }

    pub fn flush(&mut self) {
        if let Some(file) = self.file.as_mut() {
            let _ = file.flush();
        }
    }
}

enum Sink {
    /// The log folder is not known yet.
    Early(Vec<u8>),
    Open(RotatingFile),
    /// The file could not be opened; lines are dropped.
    Off,
}

static SINK: Mutex<Sink> = Mutex::new(Sink::Early(Vec::new()));

thread_local! {
    /// Set while this thread is inside the sink, so a panic there is not logged into it.
    static WRITING: Cell<bool> = const { Cell::new(false) };
}

/// Whether this thread is part-way through writing a log line. The panic hook checks it so
/// it never waits on the lock its own thread holds.
pub fn writing_on_this_thread() -> bool {
    WRITING.with(Cell::get)
}

fn lock_sink() -> std::sync::MutexGuard<'static, Sink> {
    SINK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Start writing to `dir\localflow.log`, including anything logged before now. Returns the
/// file's path.
pub fn attach(dir: &Path) -> io::Result<PathBuf> {
    let mut sink = lock_sink();
    if let Sink::Open(file) = &*sink {
        return Ok(file.path().to_path_buf());
    }
    let mut file = match RotatingFile::open(dir.join(LOG_FILE_NAME), LOG_MAX_BYTES) {
        Ok(file) => file,
        Err(error) => {
            *sink = Sink::Off;
            return Err(error);
        }
    };
    if let Sink::Early(early) = &*sink {
        if !early.is_empty() {
            let _ = file.write_record(early);
        }
    }
    let path = file.path().to_path_buf();
    *sink = Sink::Open(file);
    Ok(path)
}

/// Push buffered lines to disk. Called on the way out.
pub fn flush() {
    if let Sink::Open(file) = &mut *lock_sink() {
        file.flush();
    }
}

fn write_line(line: &[u8]) {
    let text = String::from_utf8_lossy(line);
    let redacted = redact_user_paths(&text);
    WRITING.with(|writing| writing.set(true));
    {
        let mut sink = lock_sink();
        match &mut *sink {
            Sink::Early(buffer) => {
                if buffer.len() + redacted.len() <= EARLY_LIMIT {
                    buffer.extend_from_slice(redacted.as_bytes());
                }
            }
            Sink::Open(file) => {
                let _ = file.write_record(redacted.as_bytes());
            }
            Sink::Off => {}
        }
    }
    WRITING.with(|writing| writing.set(false));
}

/// The writer `tracing_subscriber` formats each event into. One event becomes one record.
#[derive(Clone, Copy, Default)]
pub struct LogMakeWriter;

/// Collects one formatted event and hands it to the file when dropped.
pub struct EventWriter {
    buffer: Vec<u8>,
}

impl Write for EventWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for EventWriter {
    fn drop(&mut self) {
        if self.buffer.is_empty() {
            return;
        }
        // A debug build also has a console; keep printing there.
        #[cfg(debug_assertions)]
        {
            let _ = io::stdout().write_all(&self.buffer);
        }
        write_line(&self.buffer);
    }
}

impl<'a> MakeWriter<'a> for LogMakeWriter {
    type Writer = EventWriter;

    fn make_writer(&'a self) -> Self::Writer {
        EventWriter {
            buffer: Vec::with_capacity(256),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "localflow-log-test-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_file_rotates_at_the_cap_and_keeps_at_most_two_files() {
        let dir = temp_dir("rotate");
        let mut log = RotatingFile::open(dir.join(LOG_FILE_NAME), 100).unwrap();

        for index in 0..30 {
            log.write_record(format!("line {index:02} xxxxxxxxxx\n").as_bytes())
                .unwrap();
        }
        log.flush();

        let current = fs::read_to_string(log.path()).unwrap();
        let previous = fs::read_to_string(log.rotated_path()).unwrap();
        assert!(current.len() <= 100, "{}", current.len());
        assert!(previous.len() <= 100, "{}", previous.len());
        assert!(current.ends_with("line 29 xxxxxxxxxx\n"));
        // Whole records only: every line in both files is complete.
        for text in [&current, &previous] {
            assert!(text
                .lines()
                .all(|line| line.len() == "line 00 xxxxxxxxxx".len()));
        }
        let files = fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_existing_file_counts_towards_the_cap() {
        let dir = temp_dir("existing");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(LOG_FILE_NAME), vec![b'a'; 90]).unwrap();

        let mut log = RotatingFile::open(dir.join(LOG_FILE_NAME), 100).unwrap();
        log.write_record(b"0123456789abcdef\n").unwrap();
        log.flush();

        assert_eq!(fs::read(log.rotated_path()).unwrap().len(), 90);
        assert_eq!(
            fs::read_to_string(log.path()).unwrap(),
            "0123456789abcdef\n"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_record_longer_than_the_cap_is_cut_to_it() {
        let dir = temp_dir("long");
        let mut log = RotatingFile::open(dir.join(LOG_FILE_NAME), 16).unwrap();
        log.write_record(&[b'z'; 40]).unwrap();
        log.flush();
        assert_eq!(fs::read(log.path()).unwrap().len(), 16);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn formatted_events_are_written_whole_with_user_folders_removed() {
        let mut writer = LogMakeWriter.make_writer();
        writer.write_all(b"part one, ").unwrap();
        writer.write_all(br"model C:\Users\someone\m.gguf").unwrap();
        assert_eq!(
            writer.buffer,
            br"part one, model C:\Users\someone\m.gguf".to_vec()
        );

        let text = String::from_utf8_lossy(&writer.buffer).into_owned();
        assert_eq!(
            redact_user_paths(&text),
            r"part one, model <user folder>\m.gguf"
        );
        // Not dropped into the shared sink here: other tests may be logging.
        std::mem::forget(writer);
    }
}
