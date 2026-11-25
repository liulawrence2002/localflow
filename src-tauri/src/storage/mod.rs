pub mod log_file;
pub mod redact;

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use chrono::Utc;
use rusqlite::{params, Connection, ErrorCode, OptionalExtension};
use tauri::{AppHandle, Manager};

use crate::app_state::{parse_settings, SettingsSnapshot};
use redact::{file_name_only, redact_user_paths};

pub const SETTINGS_KEY: &str = "settings.v1";
/// Where a settings row that could not be parsed is copied before defaults replace it.
pub const UNREADABLE_SETTINGS_KEY: &str = "settings.v1.unreadable";
pub const DATABASE_FILE_NAME: &str = "localflow.sqlite3";
/// SQLite's companion files, moved together with a damaged database so a leftover journal
/// is never applied to the new one.
const DATABASE_SIDE_FILES: &[&str] = &["-journal", "-wal", "-shm"];
/// How many times the database is tried when it is in use or not accessible (backup or
/// antivirus software holding it, a permission, a read-only file), and how far apart.
const ACCESS_ATTEMPTS: u32 = 3;
const ACCESS_RETRY_DELAY: Duration = Duration::from_millis(200);

const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS settings (
        key TEXT PRIMARY KEY NOT NULL,
        value_json TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS dictation_history (
        id TEXT PRIMARY KEY NOT NULL,
        created_at TEXT NOT NULL,
        target_application TEXT NOT NULL,
        raw_transcript TEXT,
        final_text TEXT,
        cleanup_level TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS dictionary_entries (
        id TEXT PRIMARY KEY NOT NULL,
        phrase TEXT NOT NULL,
        pronunciation_hint TEXT,
        category TEXT NOT NULL,
        case_sensitive INTEGER NOT NULL DEFAULT 0
    );
";

/// The database once it is usable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Initialized {
    pub path: PathBuf,
    /// The name the damaged database was moved to, when it had to be replaced.
    pub replaced_damaged: Option<String>,
}

/// Why the database could not be used. Safe to show: user folders are removed. In every
/// case the file was left exactly as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// The file is there but could not be opened or read: in use by another program, not
    /// permitted, or read-only. It is probably fine.
    Inaccessible(String),
    /// Anything else: the folder cannot be made, or a damaged file cannot be moved aside.
    Failed(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inaccessible(message) | Self::Failed(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for StorageError {}

fn redacted(error: impl std::fmt::Display) -> String {
    redact_user_paths(&error.to_string()).into_owned()
}

/// Whether the file itself is damaged: not a database, or failing its integrity check. Only
/// then is it moved aside and replaced. Every other failure (in use, not permitted,
/// read-only, busy, an I/O error) is about getting at the file, which may well be intact, so
/// it is left alone.
fn is_damaged(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _)
            if matches!(failure.code, ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase)
    )
}

/// Run `attempt` until it succeeds, fails with damage, or has been tried
/// [`ACCESS_ATTEMPTS`] times: a file held for a moment by backup or antivirus software is
/// usually free again a fraction of a second later.
fn with_access_retries<T>(
    mut attempt: impl FnMut() -> Result<T, rusqlite::Error>,
) -> Result<T, rusqlite::Error> {
    let mut tries = 1;
    loop {
        match attempt() {
            Err(error) if !is_damaged(&error) && tries < ACCESS_ATTEMPTS => {
                tracing::info!(error = %error, tries, "settings database not accessible; retrying");
                tries += 1;
                thread::sleep(ACCESS_RETRY_DELAY);
            }
            result => return result,
        }
    }
}

/// Open the database, check every page, and make sure the tables exist.
fn open_checked(path: &Path) -> Result<Connection, rusqlite::Error> {
    let connection = Connection::open(path)?;
    // Reads every page, so a damaged file fails here rather than on some later query.
    let verdict: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    if verdict != "ok" {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
            Some("integrity check failed".to_string()),
        ));
    }
    connection.execute_batch(SCHEMA)?;
    Ok(connection)
}

/// Move a damaged database and its companion files aside. Returns the new name.
fn move_aside(path: &Path) -> io::Result<String> {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| DATABASE_FILE_NAME.to_string());
    let mut kept_name = format!("{name}.corrupt-{stamp}");
    let mut kept = path.with_file_name(&kept_name);
    let mut attempt = 1;
    while kept.exists() {
        attempt += 1;
        kept_name = format!("{name}.corrupt-{stamp}-{attempt}");
        kept = path.with_file_name(&kept_name);
    }
    fs::rename(path, &kept)?;
    for suffix in DATABASE_SIDE_FILES {
        let side = path.with_file_name(format!("{name}{suffix}"));
        if side.exists() {
            let _ = fs::rename(&side, path.with_file_name(format!("{kept_name}{suffix}")));
        }
    }
    Ok(kept_name)
}

pub fn initialize_at(data_dir: &Path) -> Result<Initialized, StorageError> {
    fs::create_dir_all(data_dir).map_err(|error| StorageError::Failed(redacted(error)))?;
    let database_path = data_dir.join(DATABASE_FILE_NAME);

    match with_access_retries(|| open_checked(&database_path)) {
        Ok(_) => Ok(Initialized {
            path: database_path,
            replaced_damaged: None,
        }),
        Err(error) if database_path.exists() && is_damaged(&error) => {
            tracing::warn!(error = %error, "the settings database is damaged; replacing it");
            let kept = move_aside(&database_path).map_err(|rename_error| {
                StorageError::Failed(format!(
                    "the settings database ({DATABASE_FILE_NAME}) is damaged and could not be \
                     moved aside: {}",
                    redacted(rename_error)
                ))
            })?;
            open_checked(&database_path).map_err(|error| StorageError::Failed(redacted(error)))?;
            Ok(Initialized {
                path: database_path,
                replaced_damaged: Some(kept),
            })
        }
        Err(error) if database_path.exists() => Err(StorageError::Inaccessible(redacted(error))),
        Err(error) => Err(StorageError::Failed(redacted(error))),
    }
}

/// Settings read from the database, and how many stored values had to be reset or corrected
/// to read them. Start-up reads the row itself ([`start_at`]); this is for the tests.
#[cfg(test)]
#[derive(Debug, Clone)]
pub struct StoredSettings {
    pub settings: SettingsSnapshot,
    pub repaired: usize,
}

/// Set at start-up when the database could not be used. For the rest of the session nothing
/// is written to it: the file may hold the user's settings, and a save would replace them
/// with the defaults LocalFlow is running on.
static SAVES_REFUSED: AtomicBool = AtomicBool::new(false);

const SAVES_REFUSED_MESSAGE: &str = "settings are not being saved this session, because the \
     settings database could not be opened when LocalFlow started and saving now could replace \
     your saved settings. Restart LocalFlow to save again.";

pub fn save_settings(
    app: &AppHandle,
    settings: &SettingsSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    save_unless_refused(&SAVES_REFUSED, &database_path(app)?, settings)
}

fn save_unless_refused(
    refused: &AtomicBool,
    database_path: &Path,
    settings: &SettingsSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    if refused.load(Ordering::SeqCst) {
        return Err(StorageError::Failed(SAVES_REFUSED_MESSAGE.to_string()).into());
    }
    save_settings_to_path(database_path, settings)
}

pub fn preserve_unreadable_settings_at(
    database_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::open(database_path)?;
    connection.execute(
        "
            INSERT INTO settings (key, value_json, updated_at)
            SELECT ?2, value_json, ?3 FROM settings WHERE key = ?1
            ON CONFLICT(key) DO UPDATE SET
                value_json = excluded.value_json,
                updated_at = excluded.updated_at
            ",
        params![
            SETTINGS_KEY,
            UNREADABLE_SETTINGS_KEY,
            Utc::now().to_rfc3339()
        ],
    )?;

    Ok(())
}

fn database_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(app.path().app_data_dir()?.join(DATABASE_FILE_NAME))
}

/// The stored settings row, as text.
fn read_settings_row(database_path: &Path) -> Result<Option<String>, rusqlite::Error> {
    Connection::open(database_path)?
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()
}

/// The stored settings, `None` when nothing was stored yet. An error means the database
/// could not be read or the row is not JSON at all; values that are merely out of range or
/// of an unknown kind are corrected (see [`parse_settings`]).
#[cfg(test)]
pub fn load_settings_from_path(
    database_path: &Path,
) -> Result<Option<StoredSettings>, Box<dyn std::error::Error>> {
    match read_settings_row(database_path)? {
        None => Ok(None),
        Some(json) => {
            let (settings, repaired) = parse_settings(&json)?;
            Ok(Some(StoredSettings { settings, repaired }))
        }
    }
}

pub fn save_settings_to_path(
    database_path: &Path,
    settings: &SettingsSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let connection = Connection::open(database_path)?;
        let value_json = serde_json::to_string(settings)?;
        connection.execute(
            "
                INSERT INTO settings (key, value_json, updated_at)
                VALUES (?1, ?2, ?3)
                ON CONFLICT(key) DO UPDATE SET
                    value_json = excluded.value_json,
                    updated_at = excluded.updated_at
                ",
            params![SETTINGS_KEY, value_json, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    })();
    // The message is shown in the settings window; keep the user's folder out of it.
    result.map_err(|error| StorageError::Failed(redacted(error)).into())
}

/// What start-up found in the settings database.
#[derive(Debug, Clone)]
pub struct StartupSettings {
    /// The database, when it could be used. `None` means nothing may be saved this session.
    pub database: Option<PathBuf>,
    /// The settings to run with; `None` means the defaults.
    pub settings: Option<SettingsSnapshot>,
    /// Nothing was stored yet, so the defaults should be saved.
    pub seed_defaults: bool,
    /// A sentence for the user when the defaults had to be used for some or all settings.
    pub notice: Option<String>,
}

/// Running on defaults because the database could not be used; it is not touched.
fn without_database(error: &StorageError) -> StartupSettings {
    tracing::warn!(error = %error, "the settings database could not be used; not saving");
    let notice = match error {
        StorageError::Inaccessible(reason) => format!(
            "LocalFlow started with default settings and cannot save changes until it is \
             restarted, because its settings database is in use by another program or cannot \
             be accessed ({reason}). Nothing in it was changed."
        ),
        StorageError::Failed(reason) => format!(
            "LocalFlow started with default settings and cannot save changes until it is \
             restarted, because its settings database could not be opened: {reason}. Nothing \
             in it was changed."
        ),
    };
    StartupSettings {
        database: None,
        settings: None,
        seed_defaults: false,
        notice: Some(notice),
    }
}

/// Open the database under `data_dir` and read the settings, recovering what can be
/// recovered. Never fails: at worst LocalFlow runs on defaults and says why. Only a damaged
/// file is replaced; one that is merely in use or not accessible is left as it is.
pub fn start_at(data_dir: &Path) -> StartupSettings {
    let initialized = match initialize_at(data_dir) {
        Ok(initialized) => initialized,
        Err(error) => return without_database(&error),
    };
    let row = match with_access_retries(|| read_settings_row(&initialized.path)) {
        Ok(row) => row,
        // Opened a moment ago but not readable now: the same as not opening at all.
        Err(error) => return without_database(&StorageError::Inaccessible(redacted(error))),
    };
    let mut startup = StartupSettings {
        database: Some(initialized.path.clone()),
        settings: None,
        seed_defaults: false,
        notice: initialized.replaced_damaged.as_ref().map(|kept| {
            format!(
                "The settings database was damaged, so LocalFlow started with a new one and \
                 default settings. The damaged file was kept as {kept}."
            )
        }),
    };

    match row.map(|json| parse_settings(&json)) {
        None => startup.seed_defaults = true,
        Some(Ok((settings, 0))) => startup.settings = Some(settings),
        Some(Ok((settings, repaired))) => {
            tracing::warn!(
                values = repaired,
                "some stored settings could not be read and were reset"
            );
            // The next save overwrites the row; keep the original first.
            if let Err(error) = preserve_unreadable_settings_at(&initialized.path) {
                tracing::warn!(error = %error, "could not keep a copy of the stored settings");
            }
            startup.settings = Some(settings);
            startup.notice = Some(
                "Some saved settings could not be read and were reset to their defaults. A copy \
                 of the saved settings was kept."
                    .to_string(),
            );
        }
        Some(Err(error)) => {
            tracing::warn!(error = %error, "the stored settings are not JSON; using defaults");
            if let Err(error) = preserve_unreadable_settings_at(&initialized.path) {
                tracing::warn!(error = %error, "could not keep a copy of the unreadable settings");
            }
            startup.notice = Some(
                "The saved settings could not be read, so LocalFlow started with default \
                 settings. A copy of them was kept."
                    .to_string(),
            );
        }
    }
    startup
}

/// [`start_at`] for the app's data folder. When the database cannot be used, saving is
/// refused for the rest of the session (see [`save_settings`]).
pub fn start(app: &AppHandle) -> StartupSettings {
    let startup = match app.path().app_data_dir() {
        Ok(data_dir) => start_at(&data_dir),
        Err(error) => without_database(&StorageError::Failed(format!(
            "its data folder could not be found ({})",
            redacted(error)
        ))),
    };
    SAVES_REFUSED.store(startup.database.is_none(), Ordering::SeqCst);
    startup
}

/// The database's file name, for logs.
pub fn database_label(path: &Path) -> String {
    file_name_only(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "localflow-storage-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ))
    }

    fn store_raw(database_path: &Path, json: &str) {
        Connection::open(database_path)
            .expect("database should open")
            .execute(
                "INSERT INTO settings (key, value_json, updated_at) VALUES (?1, ?2, ?3)",
                params![SETTINGS_KEY, json, Utc::now().to_rfc3339()],
            )
            .expect("row should insert");
    }

    fn kept_copy(database_path: &Path) -> String {
        Connection::open(database_path)
            .expect("database should open")
            .query_row(
                "SELECT value_json FROM settings WHERE key = ?1",
                params![UNREADABLE_SETTINGS_KEY],
                |row| row.get(0),
            )
            .expect("the copy should exist")
    }

    #[test]
    fn settings_round_trip_through_single_json_row() {
        let test_dir = test_dir("round-trip");

        let database_path = initialize_at(&test_dir)
            .expect("storage should initialize")
            .path;
        let mut settings = crate::app_state::default_settings();
        settings.models.ollama_model = "test-model:1b".to_string();
        settings.models.low_resource_mode = true;
        settings.dictionary.push(crate::app_state::DictionaryEntry {
            id: "dict-test".to_string(),
            phrase: "LocalFlow".to_string(),
            pronunciation_hint: None,
            category: "product".to_string(),
            case_sensitive: true,
        });

        save_settings_to_path(&database_path, &settings).expect("settings should save");

        let loaded = load_settings_from_path(&database_path)
            .expect("settings should load")
            .expect("settings row should exist");
        assert_eq!(loaded.repaired, 0);
        let loaded = loaded.settings;
        assert_eq!(loaded.models.ollama_model, "test-model:1b");
        assert!(loaded.models.low_resource_mode);
        assert_eq!(
            loaded.dictionary.last().map(|entry| entry.phrase.as_str()),
            Some("LocalFlow")
        );

        let row_count: i64 = Connection::open(&database_path)
            .expect("database should open")
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key = ?1",
                params![SETTINGS_KEY],
                |row| row.get(0),
            )
            .expect("settings row should be countable");
        assert_eq!(row_count, 1);

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn an_unreadable_settings_row_is_kept_before_defaults_replace_it() {
        let test_dir = test_dir("unreadable");
        let database_path = initialize_at(&test_dir)
            .expect("storage should initialize")
            .path;
        let broken = r#"{"dictionary": [ not json at all"#;
        store_raw(&database_path, broken);

        assert!(load_settings_from_path(&database_path).is_err());
        let startup = start_at(&test_dir);
        assert!(startup.settings.is_none());
        assert!(startup
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("could not be read")));
        // The app now saves defaults over the main row, as it does after a failed load.
        save_settings_to_path(&database_path, &crate::app_state::default_settings())
            .expect("defaults should save");

        assert_eq!(kept_copy(&database_path), broken);
        assert!(load_settings_from_path(&database_path)
            .expect("defaults should load")
            .is_some());

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn a_row_with_bad_values_loads_the_good_ones_and_keeps_a_copy() {
        let test_dir = test_dir("repaired");
        let database_path = initialize_at(&test_dir)
            .expect("storage should initialize")
            .path;
        let stored = r#"{"models": {"ollamaModel": "mine:7b", "asrThreads": 900},
                         "hotkeys": {"activationMode": "from_a_newer_build"},
                         "dictionary": "this is not a list"}"#;
        store_raw(&database_path, stored);

        let startup = start_at(&test_dir);

        let settings = startup.settings.expect("the good values are kept");
        assert_eq!(settings.models.ollama_model, "mine:7b");
        assert_eq!(settings.models.asr_threads, u8::MAX);
        assert!(startup
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("reset to their defaults")));
        assert_eq!(kept_copy(&database_path), stored);
        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn a_damaged_database_is_moved_aside_and_replaced() {
        let test_dir = test_dir("damaged");
        fs::create_dir_all(&test_dir).unwrap();
        let database_path = test_dir.join(DATABASE_FILE_NAME);
        fs::write(
            &database_path,
            b"this is certainly not an SQLite database file....",
        )
        .unwrap();
        fs::write(
            test_dir.join(format!("{DATABASE_FILE_NAME}-journal")),
            b"old",
        )
        .unwrap();

        let startup = start_at(&test_dir);

        assert_eq!(startup.database.as_deref(), Some(database_path.as_path()));
        assert!(startup.seed_defaults);
        let notice = startup.notice.expect("the user is told");
        assert!(notice.contains("damaged"));
        assert!(notice.contains("localflow.sqlite3.corrupt-"));
        assert!(!notice.contains(&test_dir.to_string_lossy().to_string()));
        // The new database works, and the old file and its journal were kept.
        save_settings_to_path(&database_path, &crate::app_state::default_settings()).unwrap();
        let names: Vec<String> = fs::read_dir(&test_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names
            .iter()
            .any(|name| name.starts_with("localflow.sqlite3.corrupt-")
                && !name.ends_with("-journal")));
        // No journal of the old file is left to be applied to the new one (SQLite itself may
        // already have discarded one that was not valid).
        assert!(!names.iter().any(|name| name == "localflow.sqlite3-journal"));
        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn a_healthy_database_is_not_replaced() {
        let test_dir = test_dir("healthy");
        initialize_at(&test_dir).unwrap();
        let again = initialize_at(&test_dir).unwrap();
        assert_eq!(again.replaced_damaged, None);
        let startup = start_at(&test_dir);
        assert!(startup.notice.is_none());
        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn a_data_folder_that_cannot_be_made_runs_on_defaults_and_says_why() {
        let test_dir = test_dir("blocked");
        fs::create_dir_all(test_dir.parent().unwrap()).unwrap();
        // A file where the folder should be.
        fs::write(&test_dir, b"x").unwrap();

        let startup = start_at(&test_dir);

        assert!(startup.database.is_none());
        assert!(startup.settings.is_none());
        assert!(startup
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("cannot save changes")));
        let _ = fs::remove_file(&test_dir);
    }

    fn sqlite_error(code: std::ffi::c_int) -> rusqlite::Error {
        rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(code), None)
    }

    /// A healthy database holding the user's settings, and its bytes.
    fn healthy_database(name: &str) -> (PathBuf, PathBuf, Vec<u8>) {
        let test_dir = test_dir(name);
        let database_path = initialize_at(&test_dir).unwrap().path;
        let mut settings = crate::app_state::default_settings();
        settings.models.ollama_model = "mine:7b".to_string();
        save_settings_to_path(&database_path, &settings).unwrap();
        let bytes = fs::read(&database_path).unwrap();
        (test_dir, database_path, bytes)
    }

    fn moved_aside(test_dir: &Path) -> bool {
        fs::read_dir(test_dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".corrupt-")
        })
    }

    #[test]
    fn only_damage_counts_as_damage() {
        use rusqlite::ffi;
        for damaged in [ffi::SQLITE_CORRUPT, ffi::SQLITE_NOTADB] {
            assert!(is_damaged(&sqlite_error(damaged)), "{damaged}");
        }
        // Getting at the file failed; the file itself may be fine.
        for access in [
            ffi::SQLITE_CANTOPEN,
            ffi::SQLITE_PERM,
            ffi::SQLITE_READONLY,
            ffi::SQLITE_BUSY,
            ffi::SQLITE_LOCKED,
            ffi::SQLITE_IOERR,
            ffi::SQLITE_AUTH,
            ffi::SQLITE_FULL,
        ] {
            assert!(!is_damaged(&sqlite_error(access)), "{access}");
        }
        assert!(!is_damaged(&rusqlite::Error::QueryReturnedNoRows));
    }

    #[test]
    fn access_errors_are_tried_again_and_damage_is_not() {
        let mut calls = 0;
        let started = std::time::Instant::now();
        let result: Result<(), _> = with_access_retries(|| {
            calls += 1;
            Err(sqlite_error(rusqlite::ffi::SQLITE_CANTOPEN))
        });
        assert!(result.is_err());
        assert_eq!(calls, ACCESS_ATTEMPTS);
        assert!(started.elapsed() >= ACCESS_RETRY_DELAY * (ACCESS_ATTEMPTS - 1));

        // Free again on the last try.
        let mut calls = 0;
        let result = with_access_retries(|| {
            calls += 1;
            if calls < ACCESS_ATTEMPTS {
                Err(sqlite_error(rusqlite::ffi::SQLITE_BUSY))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(result.unwrap(), ACCESS_ATTEMPTS);

        // Damage is final at once.
        let mut calls = 0;
        let result: Result<(), _> = with_access_retries(|| {
            calls += 1;
            Err(sqlite_error(rusqlite::ffi::SQLITE_CORRUPT))
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_page_that_fails_the_integrity_check_counts_as_damage() {
        let (test_dir, database_path, mut bytes) = healthy_database("bad-page");
        // Keep the header and the schema page; ruin the next page (a table's root).
        let page = u16::from_be_bytes([bytes[16], bytes[17]]) as usize;
        let page = if page == 1 { 65_536 } else { page };
        assert!(bytes.len() >= 2 * page, "the database has a second page");
        bytes[page..2 * page].fill(0xA5);
        fs::write(&database_path, &bytes).unwrap();

        let startup = start_at(&test_dir);

        assert!(startup
            .notice
            .as_deref()
            .is_some_and(|notice| notice.contains("damaged")));
        assert!(moved_aside(&test_dir));
        let _ = fs::remove_dir_all(test_dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_database_held_by_another_program_is_left_alone_and_never_saved_over() {
        use std::os::windows::fs::OpenOptionsExt;
        let (test_dir, database_path, original) = healthy_database("held");
        // As backup software can: open with no sharing at all.
        let holder = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&database_path)
            .unwrap();

        let startup = start_at(&test_dir);

        assert!(
            startup.database.is_none(),
            "nothing may be saved this session"
        );
        assert!(startup.settings.is_none());
        let notice = startup.notice.expect("the user is told");
        assert!(notice.contains("in use by another program"), "{notice}");
        assert!(notice.contains("Nothing in it was changed"), "{notice}");
        assert!(!notice.contains(&*test_dir.to_string_lossy()), "{notice}");
        drop(holder);
        assert!(!moved_aside(&test_dir));
        assert_eq!(fs::read(&database_path).unwrap(), original);

        // Later in the session the file is free again, but a save is still refused.
        let refused = AtomicBool::new(true);
        let error = save_unless_refused(
            &refused,
            &database_path,
            &crate::app_state::default_settings(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not being saved this session"));
        assert_eq!(fs::read(&database_path).unwrap(), original);
        let kept = load_settings_from_path(&database_path).unwrap().unwrap();
        assert_eq!(kept.settings.models.ollama_model, "mine:7b");
        let _ = fs::remove_dir_all(test_dir);
    }

    #[cfg(windows)]
    #[test]
    fn a_database_held_only_briefly_is_opened_on_a_later_try() {
        use std::os::windows::fs::OpenOptionsExt;
        let (test_dir, database_path, _) = healthy_database("briefly-held");
        let holder = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&database_path)
            .unwrap();
        let release = thread::spawn(move || {
            thread::sleep(Duration::from_millis(250));
            drop(holder);
        });

        let startup = start_at(&test_dir);
        release.join().unwrap();

        assert!(startup.database.is_some());
        assert_eq!(
            startup
                .settings
                .expect("the saved settings")
                .models
                .ollama_model,
            "mine:7b"
        );
        assert!(startup.notice.is_none());
        assert!(!moved_aside(&test_dir));
        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn a_read_only_database_is_not_replaced() {
        let (test_dir, database_path, original) = healthy_database("read-only");
        let mut permissions = fs::metadata(&database_path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&database_path, permissions.clone()).unwrap();

        let startup = start_at(&test_dir);

        assert!(!moved_aside(&test_dir));
        assert_eq!(fs::read(&database_path).unwrap(), original);
        // Either it was read (SQLite opens such a file read-only) or it was left alone and
        // saving is off; it is never replaced with defaults.
        match startup.settings {
            Some(settings) => assert_eq!(settings.models.ollama_model, "mine:7b"),
            None => assert!(startup.database.is_none()),
        }
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        let _ = fs::set_permissions(&database_path, permissions);
        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn saves_go_through_unless_refused() {
        let (test_dir, database_path, _) = healthy_database("gate");
        let mut changed = crate::app_state::default_settings();
        changed.models.ollama_model = "other:1b".to_string();

        save_unless_refused(&AtomicBool::new(false), &database_path, &changed).unwrap();

        let stored = load_settings_from_path(&database_path).unwrap().unwrap();
        assert_eq!(stored.settings.models.ollama_model, "other:1b");
        let _ = fs::remove_dir_all(test_dir);
    }
}
