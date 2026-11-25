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
mod readiness;
mod refinement;
mod runtime_paths;
mod storage;
mod transcript;
pub mod tts;
mod workflow;

use std::{sync::mpsc, thread, time::Duration};

#[cfg(debug_assertions)]
use app_state::{begin_mock_session, finish_mock_session};
use app_state::{cancel_session, get_status, save_settings, LocalFlowRuntime};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

/// The flag the desktop launcher script passes.
const DESKTOP_LAUNCH_FLAG: &str = "--localflow-desktop-launch";
/// How long quitting waits for the engines to stop before leaving the rest to the process
/// exit (every sidecar is in a kill-on-close job, so nothing outlives the app).
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);

/// Log to the size-capped file (see `storage::log_file`). The filter is fixed: an environment
/// variable must not be able to turn on a dependency's verbose logging, which could record
/// request bodies.
fn init_logging() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("localflow=info,tauri=warn")
        .with_ansi(false)
        .with_writer(storage::log_file::LogMakeWriter)
        .try_init();
}

/// Record where a panic happened: in the log and in desktop health. The thread's name and
/// the source location only; the message can quote whatever text was being handled.
fn install_panic_hook() {
    #[cfg(debug_assertions)]
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        let location = info
            .location()
            .map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            })
            .unwrap_or_else(|| "unknown location".to_string());
        // Never wait on the log's lock if this thread panicked while holding it.
        let in_the_log = storage::log_file::writing_on_this_thread();
        if !in_the_log {
            tracing::error!(thread = thread_name, location = %location, "panic");
        }
        desktop_health::record_panic(thread_name, &location);
        if !in_the_log {
            storage::log_file::flush();
        }
        // A debug build also prints the usual report, message included, to its console.
        #[cfg(debug_assertions)]
        previous(info);
    }));
}

/// Whether closing this window should only hide it. The settings window is hidden, so the
/// tray's "Show LocalFlow" can bring it back; a destroyed window cannot be shown again.
fn hides_on_close(label: &str) -> bool {
    label == "main"
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// What a second start of LocalFlow asks of the one already running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecondLaunch {
    /// From the desktop launcher script: say "ready", as on a first launch.
    ReadyPulse,
    /// Anything else (the Start menu, the executable): show the settings window.
    ShowWindow,
}

fn second_launch(argv: &[String]) -> SecondLaunch {
    if argv.iter().any(|arg| arg == DESKTOP_LAUNCH_FLAG) {
        SecondLaunch::ReadyPulse
    } else {
        SecondLaunch::ShowWindow
    }
}

/// Show `message` in a message box on its own thread, so start-up carries on.
fn tell_user_later(title: &'static str, message: String) {
    let _ = thread::Builder::new()
        .name("localflow-notice".to_string())
        .spawn(move || platform::show_message_box(title, &message, false));
}

/// Open the settings database and load the settings, telling the user when defaults had to
/// be used.
fn load_settings_at_startup(app: &AppHandle) {
    let startup = storage::start(app);
    let runtime = app.state::<LocalFlowRuntime>();
    if let Some(database) = &startup.database {
        tracing::info!(
            file = %storage::database_label(database),
            "initialized local settings database"
        );
    }
    if let Some(settings) = startup.settings {
        runtime.replace_settings(settings);
        tracing::info!("loaded persisted settings");
    } else if startup.seed_defaults {
        tracing::info!("no persisted settings found; using defaults");
        if let Err(error) = storage::save_settings(app, &runtime.current_settings()) {
            tracing::warn!(error = %error, "could not seed default persisted settings");
        }
    }
    if let Some(notice) = &startup.notice {
        tell_user_later("LocalFlow settings", notice.clone());
    }
    runtime.set_storage_notice(startup.notice);
}

/// Remove temporary audio files a crashed run left behind, off the start-up path.
fn sweep_temp_files_in_background() {
    let _ = thread::Builder::new()
        .name("localflow-temp-sweep".to_string())
        .spawn(|| {
            let removed = asr::whisper_cli::sweep_stale_temp_files();
            if removed > 0 {
                tracing::info!(
                    files = removed,
                    "removed temporary audio files left by an earlier run"
                );
            }
        });
}

/// Stop the conversation, the readers and the engines on a worker, waiting at most
/// [`SHUTDOWN_WAIT`]: a model lock held by a transcription in progress must not keep the app
/// from closing.
fn shut_down(app: &AppHandle) {
    let (done_tx, done_rx) = mpsc::channel();
    let worker_app = app.clone();
    let started = thread::Builder::new()
        .name("localflow-shutdown".to_string())
        .spawn(move || {
            conversation::end(&worker_app);
            page_reader::stop(&worker_app);
            asr::engine_select::shutdown(&worker_app);
            tts::shutdown(&worker_app);
            let _ = done_tx.send(());
        });
    if started.is_ok() && done_rx.recv_timeout(SHUTDOWN_WAIT).is_err() {
        tracing::warn!("shutdown did not finish in time; the process exit ends the rest");
    }
    desktop_health::flush();
    storage::log_file::flush();
}

/// Everything start-up does once the event loop is running.
fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    match app.path().app_log_dir() {
        Ok(dir) => match storage::log_file::attach(&dir) {
            Ok(path) => tracing::info!(
                file = %storage::redact::file_name_only(&path),
                "logging to the app's log folder"
            ),
            Err(error) => tracing::warn!(error = %error, "could not open the log file"),
        },
        Err(error) => tracing::warn!(error = %error, "no log folder"),
    }
    load_settings_at_startup(app.handle());
    sweep_temp_files_in_background();

    app.manage(page_reader::ScreenReader::default());
    app.manage(conversation::ConversationRuntime::new(app.handle().clone()));

    build_tray(app)?;
    desktop_health::record_app_start(app.handle());
    desktop_health::start_launch_signal_watcher(app.handle().clone());

    if std::env::args().any(|arg| arg == DESKTOP_LAUNCH_FLAG) {
        desktop_health::record_shortcut_launch(app.handle());
        native_dictation::show_ready_pulse(app.handle(), "LocalFlow ready");
    }

    #[cfg(desktop)]
    hotkeys::register_default_hotkey(app.handle())?;

    Ok(())
}

/// Say that LocalFlow cannot start, in the log and in a message box (the release build has
/// no console, so otherwise nothing would appear at all).
fn report_start_failure(reason: &str) {
    let reason = storage::redact::redact_user_paths(reason).into_owned();
    tracing::error!(error = %reason, "LocalFlow could not start");
    storage::log_file::flush();
    platform::show_message_box(
        "LocalFlow could not start",
        &format!("LocalFlow could not start: {reason}"),
        true,
    );
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    install_panic_hook();

    let built = tauri::Builder::default()
        // First, so a second start hands over before it sets anything else up.
        .plugin(tauri_plugin_single_instance::init(
            |app, argv, _cwd| match second_launch(&argv) {
                SecondLaunch::ReadyPulse => {
                    desktop_health::record_shortcut_launch(app);
                    native_dictation::show_ready_pulse(app, "LocalFlow ready");
                }
                SecondLaunch::ShowWindow => show_main_window(app),
            },
        ))
        .manage(LocalFlowRuntime::default())
        .manage(native_dictation::NativeDictationRuntime::default())
        .manage(asr::engine_select::AsrEngines::default())
        .manage(overlay::OverlayRuntime::default())
        .manage(tts::TtsRuntime::default())
        .manage(coding_agents::CodingAgentsRuntime::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if hides_on_close(window.label()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        // Tauri panics on a setup error; say why first, or the app just never appears.
        .setup(|app| setup_app(app).inspect_err(|error| report_start_failure(&error.to_string())))
        .invoke_handler(tauri::generate_handler![
            get_status,
            save_settings,
            #[cfg(debug_assertions)]
            begin_mock_session,
            #[cfg(debug_assertions)]
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
            coding_agents::commands::detect_coding_agents,
            readiness::readiness_status
        ])
        .build(tauri::generate_context!());

    let app = match built {
        Ok(app) => app,
        Err(error) => {
            report_start_failure(&error.to_string());
            std::process::exit(1);
        }
    };
    app.run(|app, event| {
        // Stop the engines and sidecars on the way out so no process is left running.
        if let tauri::RunEvent::Exit = event {
            shut_down(app);
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
            "show" => show_main_window(app),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_settings_window_hides_it_and_other_windows_close() {
        assert!(hides_on_close("main"));
        assert!(!hides_on_close("overlay"));
    }

    #[test]
    fn a_second_start_shows_the_window_unless_the_launcher_sent_it() {
        let args = |list: &[&str]| list.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        assert_eq!(
            second_launch(&args(&["localflow.exe"])),
            SecondLaunch::ShowWindow
        );
        assert_eq!(
            second_launch(&args(&["localflow.exe", DESKTOP_LAUNCH_FLAG])),
            SecondLaunch::ReadyPulse
        );
    }
}
