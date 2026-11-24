mod app_state;
pub mod asr;
pub mod audio;
pub mod cancel;
pub mod coding_agents;
pub mod context;
pub mod conversation;
mod desktop_health;
mod hotkeys;
mod insertion;
pub mod llm;
mod native_dictation;
mod overlay;
mod page_reader;
pub mod platform;
mod privacy;
mod refinement;
mod runtime_paths;
mod storage;
mod transcript;
pub mod tts;
mod workflow;

use app_state::{
    begin_mock_session, cancel_session, finish_mock_session, get_status, save_settings,
    LocalFlowRuntime,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter("localflow=info,tauri=warn")
        .without_time()
        .init();

    tauri::Builder::default()
        .manage(LocalFlowRuntime::default())
        .manage(native_dictation::NativeDictationRuntime::default())
        .manage(asr::engine_select::AsrEngines::default())
        .manage(overlay::OverlayRuntime::default())
        .manage(tts::TtsRuntime::default())
        .manage(coding_agents::CodingAgentsRuntime::default())
        .setup(|app| {
            let database_path = storage::initialize(app.handle())?;
            tracing::info!(path = %database_path.display(), "initialized local settings database");
            match storage::load_settings(app.handle()) {
                Ok(Some(settings)) => {
                    app.state::<LocalFlowRuntime>().replace_settings(settings);
                    tracing::info!("loaded persisted settings");
                }
                Ok(None) => {
                    tracing::info!("no persisted settings found; using defaults");
                    let defaults = app.state::<LocalFlowRuntime>().current_settings();
                    if let Err(error) = storage::save_settings(app.handle(), &defaults) {
                        tracing::warn!(error = %error, "could not seed default persisted settings");
                    }
                }
                Err(error) => {
                    tracing::warn!(error = %error, "could not load persisted settings; using defaults");
                    // The next save overwrites the stored row, so keep a copy of it first.
                    if let Err(error) = storage::preserve_unreadable_settings(app.handle()) {
                        tracing::warn!(error = %error, "could not keep a copy of the unreadable settings");
                    }
                }
            }

            app.manage(page_reader::ScreenReader::default());
            app.manage(conversation::ConversationRuntime::new(app.handle().clone()));

            build_tray(app)?;
            desktop_health::record_app_start(app.handle());
            desktop_health::start_launch_signal_watcher(app.handle().clone());

            if std::env::args().any(|arg| arg == "--localflow-desktop-launch") {
                desktop_health::record_shortcut_launch(app.handle());
                native_dictation::show_ready_pulse(app.handle(), "LocalFlow ready");
            }

            #[cfg(desktop)]
            hotkeys::register_default_hotkey(app.handle())?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            save_settings,
            begin_mock_session,
            finish_mock_session,
            cancel_session,
            native_dictation::get_last_transcript,
            native_dictation::copy_last_transcript,
            tts::tts_status,
            tts::speak_text,
            tts::speak_last_transcript,
            tts::stop_speaking,
            conversation::conversation_status,
            conversation::toggle_conversation,
            llm::list_ollama_models,
            page_reader::screen_reading_status,
            coding_agents::commands::list_coding_projects,
            coding_agents::commands::save_coding_project,
            coding_agents::commands::remove_coding_project,
            coding_agents::commands::suggest_project_spoken_names,
            coding_agents::commands::detect_coding_agents
        ])
        .build(tauri::generate_context!())
        .expect("error while building LocalFlow")
        .run(|app, event| {
            // Stop the ASR sidecar on the way out so no whisper process is left running.
            if let tauri::RunEvent::Exit = event {
                conversation::end(app);
                page_reader::stop(app);
                asr::engine_select::shutdown(app);
                tts::shutdown(app);
            }
        });
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show LocalFlow", true, None::<&str>)?;
    let copy_last =
        MenuItem::with_id(app, "copy_last", "Copy last transcript", true, None::<&str>)?;
    let read_last = MenuItem::with_id(
        app,
        "read_last",
        "Read last transcript aloud",
        true,
        None::<&str>,
    )?;
    let stop_reading = MenuItem::with_id(app, "stop_reading", "Stop reading", true, None::<&str>)?;
    let conversation = MenuItem::with_id(
        app,
        "conversation",
        "Start or stop conversation",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &conversation,
            &copy_last,
            &read_last,
            &stop_reading,
            &quit,
        ],
    )?;

    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("LocalFlow")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "copy_last" => {
                if let Err(error) = native_dictation::copy_last_transcript_to_clipboard(app) {
                    tracing::info!(error = %error, "copy last transcript from tray failed");
                }
            }
            "read_last" => {
                if let Err(error) = tts::speak_last_transcript(app.clone()) {
                    tracing::info!(error = %error, "read last transcript from tray failed");
                }
            }
            "stop_reading" => {
                tts::stop(app);
                page_reader::stop(app);
            }
            "conversation" => conversation::toggle(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
