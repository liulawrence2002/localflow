use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::native_dictation;

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

pub fn register_default_hotkey(app: &AppHandle) -> tauri::Result<()> {
    let default_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
    let fallback_shortcut = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT),
        Code::Space,
    );
    let handler_default_shortcut = default_shortcut.clone();
    let handler_fallback_shortcut = fallback_shortcut.clone();
    let handler_escape_shortcut = escape_cancel_shortcut();

    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                // Escape is registered only while a dictation is active (see
                // native_dictation), so here it always means "cancel the current session".
                if shortcut == &handler_escape_shortcut {
                    if matches!(event.state(), ShortcutState::Pressed) {
                        if let Err(error) = native_dictation::handle_hotkey(app.clone(), "cancel") {
                            tracing::warn!(error = %error, "escape-to-cancel handling failed");
                        }
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
                    let state = match event.state() {
                        ShortcutState::Pressed => "pressed",
                        ShortcutState::Released => "released",
                    };
                    let _ = app.emit(
                        "localflow://hotkey",
                        HotkeyPayload {
                            shortcut: shortcut_label.to_string(),
                            state: state.to_string(),
                        },
                    );
                    if let Err(error) = native_dictation::handle_hotkey(app.clone(), state) {
                        tracing::warn!(error = %error, "native dictation hotkey handling failed");
                    }
                }
            })
            .build(),
    )?;

    if let Err(error) = app.global_shortcut().register(default_shortcut) {
        tracing::warn!(
            error = %error,
            "Ctrl+Alt+Space global shortcut unavailable; trying fallback"
        );

        if let Err(fallback_error) = app.global_shortcut().register(fallback_shortcut) {
            tracing::warn!(
                error = %fallback_error,
                "Ctrl+Alt+Shift+Space fallback global shortcut unavailable; continuing without a global hotkey"
            );
        } else {
            tracing::info!("registered fallback global hotkey Ctrl+Alt+Shift+Space");
        }
    } else {
        tracing::info!("registered global hotkey Ctrl+Alt+Space");
    }

    Ok(())
}
