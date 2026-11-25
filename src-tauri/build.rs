/// Every command the web UI may call. Listing them here makes Tauri require an explicit
/// permission per window (see `capabilities/`), so the overlay window, which only draws the
/// waveform, cannot read transcripts or change settings.
const APP_COMMANDS: &[&str] = &[
    "get_status",
    "save_settings",
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
    "list_coding_projects",
    "save_coding_project",
    "remove_coding_project",
    "suggest_project_spoken_names",
    "detect_coding_agents",
    "readiness_status",
];

/// Commands that exist only in debug builds (`cfg(debug_assertions)` in `lib.rs`): the mock
/// dictation. Their permissions are in `capabilities/debug/`, which only a debug build reads,
/// so a release build neither has the commands nor can grant them.
const DEBUG_ONLY_COMMANDS: &[&str] = &["begin_mock_session", "finish_mock_session"];

fn main() {
    // The GPU build links vulkan-1.lib from the Vulkan SDK; point the linker at it so the
    // build does not depend on LIB having been set by hand.
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");
    if std::env::var_os("CARGO_FEATURE_GPU_VULKAN").is_some() {
        if let Some(sdk) = std::env::var_os("VULKAN_SDK") {
            let lib = std::path::Path::new(&sdk).join("Lib");
            println!("cargo:rustc-link-search=native={}", lib.display());
        }
    }

    // The same switch `lib.rs` uses (`cfg(debug_assertions)`), so the commands the handler
    // registers and the permissions granted always agree, whatever the profile is called.
    let debug_build = std::env::var_os("CARGO_CFG_DEBUG_ASSERTIONS").is_some();
    let (commands, capabilities): (&'static [&'static str], &'static str) = if debug_build {
        let all: Vec<&'static str> = APP_COMMANDS
            .iter()
            .chain(DEBUG_ONLY_COMMANDS)
            .copied()
            .collect();
        (Box::leak(all.into_boxed_slice()), "./capabilities/**/*")
    } else {
        (APP_COMMANDS, "./capabilities/*.json")
    };
    println!("cargo:rerun-if-changed=capabilities");

    tauri_build::try_build(
        tauri_build::Attributes::new()
            .capabilities_path_pattern(capabilities)
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("failed to run the Tauri build script");
}
