use std::{fs, path::PathBuf};

use rusqlite::Connection;
use tauri::{AppHandle, Manager};

pub fn initialize(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let data_dir = app.path().app_data_dir()?;
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
