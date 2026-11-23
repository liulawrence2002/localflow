//! Local language model access (Ollama on loopback only).

pub mod chat;
pub mod cleanup;
pub mod ollama;

/// The local Ollama models the user can choose from.
#[tauri::command]
pub fn list_ollama_models() -> Result<Vec<String>, String> {
    ollama::list_local_models()
}
