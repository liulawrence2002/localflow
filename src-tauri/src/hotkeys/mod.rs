use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{
    Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyPayload {
    shortcut: String,
    state: String,
}

pub fn register_default_hotkey(app: &AppHandle) -> tauri::Result<()> {
    let default_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
    let handler_shortcut = default_shortcut.clone();

    app.plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, shortcut, event| {
                if shortcut == &handler_shortcut {
                    let state = match event.state() {
                        ShortcutState::Pressed => "pressed",
                        ShortcutState::Released => "released",
                    };
                    let _ = app.emit(
                        "localflow://hotkey",
                        HotkeyPayload {
                            shortcut: "Ctrl+Alt+Space".to_string(),
                            state: state.to_string(),
                        },
                    );
                }
            })
            .build(),
    )?;

    app.global_shortcut().register(default_shortcut)?;
    Ok(())
}
