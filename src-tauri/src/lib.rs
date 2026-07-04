mod app_state;
mod asr;
mod audio;
mod context;
mod hotkeys;
mod insertion;
mod native_dictation;
mod platform;
mod privacy;
mod refinement;
mod storage;
mod transcript;
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
        .plugin(tauri_plugin_opener::init())
        .manage(LocalFlowRuntime::default())
        .manage(native_dictation::NativeDictationRuntime::default())
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
                }
            }

            build_tray(app)?;

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
            native_dictation::copy_last_transcript
        ])
        .run(tauri::generate_context!())
        .expect("error while running LocalFlow");
}

fn build_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show LocalFlow", true, None::<&str>)?;
    let copy_last =
        MenuItem::with_id(app, "copy_last", "Copy last transcript", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &copy_last, &quit])?;

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
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}
