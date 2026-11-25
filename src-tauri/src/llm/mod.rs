//! Local language model access (Ollama on loopback only).

pub mod chat;
pub mod cleanup;
pub mod ollama;

/// The local Ollama models the user can choose from. Asking Ollama can take seconds, so it
/// runs off the main thread, which also runs the hotkeys and the overlay.
#[tauri::command]
pub async fn list_ollama_models() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(ollama::list_local_models)
        .await
        .map_err(|_| "Listing the local models stopped unexpectedly.".to_string())?
}
