/// Every command the web UI may call. Listing them here makes Tauri require an explicit
/// permission per window (see `capabilities/`), so the overlay window, which only draws the
/// waveform, cannot read transcripts or change settings.
const APP_COMMANDS: &[&str] = &[
    "get_status",
    "save_settings",
    "begin_mock_session",
    "finish_mock_session",
    "cancel_session",
    "get_last_transcript",
    "copy_last_transcript",
    "tts_status",
    "speak_text",
    "speak_last_transcript",
    "stop_speaking",
    "conversation_status",
    "toggle_conversation",
    "list_ollama_models",
    "screen_reading_status",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(APP_COMMANDS)),
    )
    .expect("failed to run the Tauri build script");
}
