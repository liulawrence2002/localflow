use std::{
    fs,
    path::{Path, PathBuf},
};

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use tauri::{AppHandle, Manager};

use crate::app_state::SettingsSnapshot;

pub const SETTINGS_KEY: &str = "settings.v1";

pub fn initialize(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
    initialize_at(&data_dir)
}

pub fn initialize_at(data_dir: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    fs::create_dir_all(&data_dir)?;
    let database_path = data_dir.join("localflow.sqlite3");
    let connection = Connection::open(&database_path)?;

    connection.execute_batch(
        "
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
            ",
    )?;

    Ok(database_path)
}

pub fn load_settings(
    app: &AppHandle,
) -> Result<Option<SettingsSnapshot>, Box<dyn std::error::Error>> {
    load_settings_from_path(&database_path(app)?)
}

pub fn save_settings(
    app: &AppHandle,
    settings: &SettingsSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
    save_settings_to_path(&database_path(app)?, settings)
}

fn database_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    Ok(app.path().app_data_dir()?.join("localflow.sqlite3"))
}

pub fn load_settings_from_path(
    database_path: &Path,
) -> Result<Option<SettingsSnapshot>, Box<dyn std::error::Error>> {
    let connection = Connection::open(database_path)?;
    let value_json: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()?;

    value_json
        .map(|value| serde_json::from_str(&value))
        .transpose()
        .map_err(Into::into)
}

pub fn save_settings_to_path(
    database_path: &Path,
    settings: &SettingsSnapshot,
) -> Result<(), Box<dyn std::error::Error>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_single_json_row() {
        let test_dir = std::env::temp_dir().join(format!(
            "localflow-storage-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));

        let database_path = initialize_at(&test_dir).expect("storage should initialize");
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
}
