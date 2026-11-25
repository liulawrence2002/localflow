pub mod log_file;
pub mod redact;

use std::{
    fs, io,
    path::{Path, PathBuf},
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

/// Why the database could not be opened. Safe to show: user folders are removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageError(pub String);

impl std::fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for StorageError {}

impl StorageError {
    fn from_error(error: impl std::fmt::Display) -> Self {
        Self(redact_user_paths(&error.to_string()).into_owned())
    }
}

/// Whether opening failed because the file itself is unusable (damaged, not a database, or
/// not readable), in which case it is moved aside and a new one is made. A database that is
/// merely busy is left alone: it may be fine.
fn is_replaceable(error: &rusqlite::Error) -> bool {
    matches!(
        error,
        rusqlite::Error::SqliteFailure(failure, _) if matches!(
            failure.code,
            ErrorCode::DatabaseCorrupt
                | ErrorCode::NotADatabase
                | ErrorCode::CannotOpen
                | ErrorCode::PermissionDenied
                | ErrorCode::ReadOnly
        )
    )
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
    fs::create_dir_all(data_dir).map_err(StorageError::from_error)?;
    let database_path = data_dir.join(DATABASE_FILE_NAME);

    match open_checked(&database_path) {
        Ok(_) => Ok(Initialized {
            path: database_path,
            replaced_damaged: None,
        }),
        Err(error) if database_path.exists() && is_replaceable(&error) => {
            tracing::warn!(error = %error, "the settings database is damaged; replacing it");
            let kept = move_aside(&database_path).map_err(|rename_error| {
                StorageError(format!(
                    "The settings database ({DATABASE_FILE_NAME}) is damaged and could not be \
                     moved aside: {}",
                    redact_user_paths(&rename_error.to_string())
                ))
            })?;
            open_checked(&database_path).map_err(StorageError::from_error)?;
            Ok(Initialized {
                path: database_path,
                replaced_damaged: Some(kept),
            })
        }
        Err(error) => Err(StorageError::from_error(error)),
    }
}

/// Settings read from the database, and how many stored values had to be reset or corrected
/// to read them.
#[derive(Debug, Clone)]
pub struct StoredSettings {
    pub settings: SettingsSnapshot,
    pub repaired: usize,
}

pub fn save_settings(
    app: &AppHandle,
    settings: &SettingsSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    save_settings_to_path(&database_path(app)?, settings)
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

/// The stored settings, `None` when nothing was stored yet. An error means the row is not
/// JSON at all; values that are merely out of range or of an unknown kind are corrected
/// (see [`parse_settings`]).
pub fn load_settings_from_path(
    database_path: &Path,
) -> Result<Option<StoredSettings>, Box<dyn std::error::Error>> {
    let connection = Connection::open(database_path)?;
    let value_json: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()?;

    match value_json {
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
    result.map_err(|error| StorageError::from_error(error).into())
}

/// What start-up found in the settings database.
#[derive(Debug, Clone)]
pub struct StartupSettings {
    /// The database, when it could be opened.
    pub database: Option<PathBuf>,
    /// The settings to run with; `None` means the defaults.
    pub settings: Option<SettingsSnapshot>,
    /// Nothing was stored yet, so the defaults should be saved.
    pub seed_defaults: bool,
    /// A sentence for the user when the defaults had to be used for some or all settings.
    pub notice: Option<String>,
}

/// Open the database under `data_dir` and read the settings, recovering what can be
/// recovered. Never fails: at worst LocalFlow runs on defaults and says why.
pub fn start_at(data_dir: &Path) -> StartupSettings {
    let initialized = match initialize_at(data_dir) {
        Ok(initialized) => initialized,
        Err(error) => {
            tracing::warn!(error = %error, "the settings database could not be opened");
            return StartupSettings {
                database: None,
                settings: None,
                seed_defaults: false,
                notice: Some(format!(
                    "LocalFlow is using default settings and cannot save changes, because \
                     its settings database could not be opened: {error}"
                )),
            };
        }
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

    match load_settings_from_path(&initialized.path) {
        Ok(None) => startup.seed_defaults = true,
        Ok(Some(stored)) if stored.repaired == 0 => startup.settings = Some(stored.settings),
        Ok(Some(stored)) => {
            tracing::warn!(
                values = stored.repaired,
                "some stored settings could not be read and were reset"
            );
            // The next save overwrites the row; keep the original first.
            if let Err(error) = preserve_unreadable_settings_at(&initialized.path) {
                tracing::warn!(error = %error, "could not keep a copy of the stored settings");
            }
            startup.settings = Some(stored.settings);
            startup.notice = Some(
                "Some saved settings could not be read and were reset to their defaults. A copy \
                 of the saved settings was kept."
                    .to_string(),
            );
        }
        Err(error) => {
            tracing::warn!(error = %error, "could not load persisted settings; using defaults");
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

/// [`start_at`] for the app's data folder.
pub fn start(app: &AppHandle) -> StartupSettings {
    match app.path().app_data_dir() {
        Ok(data_dir) => start_at(&data_dir),
        Err(error) => StartupSettings {
            database: None,
            settings: None,
            seed_defaults: false,
            notice: Some(format!(
                "LocalFlow is using default settings and cannot save changes, because its data \
                 folder could not be found: {}",
                redact_user_paths(&error.to_string())
            )),
        },
    }
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
}
