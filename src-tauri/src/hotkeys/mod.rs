use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::{
    conversation,
    desktop_health::{self, HotkeyRegistrationFailure},
    native_dictation, page_reader,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyPayload {
    shortcut: String,
    state: String,
}

/// The bare Escape shortcut used to cancel an active dictation. Registered only while a
/// dictation is in progress so it does not suppress Escape system-wide the rest of the time.
pub fn escape_cancel_shortcut() -> Shortcut {
    Shortcut::new(None, Code::Escape)
}

/// Shortcuts that toggle conversation mode, in order of preference. The second is tried
/// when another program already holds the first.
fn conversation_shortcuts() -> [(Shortcut, &'static str); 2] {
    [
        (
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyT),
            "Ctrl+Alt+T",
        ),
        (
            Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
                Code::KeyT,
            ),
            "Ctrl+Alt+Shift+T",
        ),
    ]
}

/// Shortcuts that read the focused window aloud (and stop reading), in order of preference.
fn reading_shortcuts() -> [(Shortcut, &'static str); 2] {
    [
        (
            Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyR),
            "Ctrl+Alt+R",
        ),
        (
            Shortcut::new(
                Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
                Code::KeyR,
            ),
            "Ctrl+Alt+Shift+R",
        ),
    ]
}

pub fn register_default_hotkey(app: &AppHandle) -> tauri::Result<()> {
    let default_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
    let fallback_shortcut = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
        Code::Space,
    );
    // `Shortcut` is `Copy`: the handler keeps its own copies for matching.
    let handler_default_shortcut = default_shortcut;
    let handler_fallback_shortcut = fallback_shortcut;
    let handler_escape_shortcut = escape_cancel_shortcut();
    let handler_conversation_shortcuts = conversation_shortcuts();
    let handler_reading_shortcuts = reading_shortcuts();

    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                // Escape is registered only while a dictation is active (see
                // native_dictation), so here it always means "cancel the current session".
                if shortcut == &handler_escape_shortcut {
                    if matches!(event.state(), ShortcutState::Pressed) {
                        if let Err(error) = native_dictation::handle_hotkey(
                            app.clone(),
                            native_dictation::HotkeyInput::Cancel,
                        ) {
                            tracing::warn!(error = %error, "escape-to-cancel handling failed");
                        }
                    }
                    return;
                }

                if handler_conversation_shortcuts
                    .iter()
                    .any(|(candidate, _)| candidate == shortcut)
                {
                    // Start, interrupt or end, decided by the conversation's own state.
                    if matches!(event.state(), ShortcutState::Pressed) {
                        conversation::toggle(app);
                    }
                    return;
                }

                if handler_reading_shortcuts
                    .iter()
                    .any(|(candidate, _)| candidate == shortcut)
                {
                    // Acted on at key-down, while the window the user means still has
                    // the focus.
                    if matches!(event.state(), ShortcutState::Pressed) {
                        page_reader::toggle(app);
                    }
                    return;
                }

                let shortcut_label = if shortcut == &handler_default_shortcut {
                    Some("Ctrl+Alt+Space")
                } else if shortcut == &handler_fallback_shortcut {
                    Some("Ctrl+Alt+Shift+Space")
                } else {
                    None
                };

                if let Some(shortcut_label) = shortcut_label {
                    let (state, input) = match event.state() {
                        ShortcutState::Pressed => {
                            ("pressed", native_dictation::HotkeyInput::Pressed)
                        }
                        ShortcutState::Released => {
                            ("released", native_dictation::HotkeyInput::Released)
                        }
                    };
                    desktop_health::record_hotkey_event(app, shortcut_label, state);
                    let _ = app.emit(
                        "localflow://hotkey",
                        HotkeyPayload {
                            shortcut: shortcut_label.to_string(),
                            state: state.to_string(),
                        },
                    );
                    // One microphone, one mode: dictation waits while a conversation is on.
                    if conversation::is_active(app) {
                        tracing::info!("dictation hotkey ignored during a conversation");
                        return;
                    }
                    if let Err(error) = native_dictation::handle_hotkey(app.clone(), input) {
                        tracing::warn!(error = %error, "native dictation hotkey handling failed");
                    }
                }
            })
            .build(),
    )?;

    let mut registered = Vec::new();
    let mut failed = Vec::new();

    match app.global_shortcut().register(default_shortcut) {
        Ok(()) => {
            registered.push("Ctrl+Alt+Space".to_string());
            tracing::info!("registered global hotkey Ctrl+Alt+Space");
        }
        Err(error) => {
            tracing::warn!(error = %error, "Ctrl+Alt+Space global shortcut unavailable");
            failed.push(HotkeyRegistrationFailure {
                shortcut: "Ctrl+Alt+Space".to_string(),
                error: error.to_string(),
            });
        }
    }

    match app.global_shortcut().register(fallback_shortcut) {
        Ok(()) => {
            registered.push("Ctrl+Alt+Shift+Space".to_string());
            tracing::info!("registered global hotkey Ctrl+Alt+Shift+Space");
        }
        Err(error) => {
            tracing::warn!(error = %error, "Ctrl+Alt+Shift+Space global shortcut unavailable");
            failed.push(HotkeyRegistrationFailure {
                shortcut: "Ctrl+Alt+Shift+Space".to_string(),
                error: error.to_string(),
            });
        }
    }

    desktop_health::record_hotkey_registration(app, registered.clone(), failed);

    let mut conversation_hotkey = None;
    for (shortcut, label) in conversation_shortcuts() {
        match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                tracing::info!(hotkey = label, "registered conversation hotkey");
                conversation_hotkey = Some(label.to_string());
                break;
            }
            Err(error) => {
                tracing::warn!(hotkey = label, error = %error, "conversation hotkey unavailable");
            }
        }
    }
    conversation::set_hotkey_label(app, conversation_hotkey);

    let mut reading_hotkey = None;
    for (shortcut, label) in reading_shortcuts() {
        match app.global_shortcut().register(shortcut) {
            Ok(()) => {
                tracing::info!(hotkey = label, "registered reading hotkey");
                reading_hotkey = Some(label.to_string());
                break;
            }
            Err(error) => {
                tracing::warn!(hotkey = label, error = %error, "reading hotkey unavailable");
            }
        }
    }
    page_reader::set_hotkey_label(app, reading_hotkey);

    if registered.is_empty() {
        native_dictation::show_error_pulse(
            app,
            "LocalFlow is running, but no desktop dictation hotkey registered.",
        );
    }

    Ok(())
}
