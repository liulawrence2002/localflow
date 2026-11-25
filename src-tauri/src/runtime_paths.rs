//! Where the local speech binaries and models live.
//!
//! Runtime folders are searched in this order:
//!
//! 1. `localflow-runtime` in the app's resource folder: what the installer ships (the
//!    whisper.cpp fallback programs and the tiny.en model). On Windows the resource folder is
//!    the folder of the executable.
//! 2. The per-user folder `%LOCALAPPDATA%\LocalFlow`, where the model scripts put downloads
//!    for an installed copy. It mirrors `.localflow-runtime`: `models\`, `models\phonon\`,
//!    `models\fish\`, `fish\` and so on.
//! 3. Only with the `dev-runtime` feature (on by default): `.localflow-runtime` in the checkout
//!    the app was built from. Release builds turn the feature off, so the executable carries
//!    no build-machine path and never looks there.
//!
//! Executables (the whisper.cpp programs, the Fish engine) are only ever started from these
//! folders. The `LOCALFLOW_WHISPER_CLI` override is honoured only with `dev-runtime`.
//!
//! "Missing" messages name files and the script that installs them, never a local path:
//! they reach the settings window and diagnostics, and paths carry the user's name.

use std::{
    env,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager};

/// Speech models looked for in the runtime `models` folder, in order. The first one that
/// exists is used unless the user configured an explicit model path. Parakeet TDT 0.6B v3
/// kept loaded in-process is both far more accurate than tiny.en and fast enough on CPU (see
/// `docs/benchmarks/ASR_BENCHMARK.md`), so it comes first; tiny.en is the small model the
/// installer bundles and the fallback when Parakeet is not installed.
pub(crate) const PREFERRED_SPEECH_MODELS: &[&str] =
    &["parakeet-tdt-0.6b-v3-Q5_K_M.gguf", "ggml-tiny.en-q5_1.bin"];
/// Whisper models usable by the whisper.cpp sidecars, for the local fallback.
pub(crate) const FALLBACK_WHISPER_MODELS: &[&str] = &["ggml-tiny.en-q5_1.bin"];

/// Folder under the app's resource folder that holds the bundled runtime.
pub(crate) const RESOURCE_RUNTIME_DIR: &str = "localflow-runtime";
/// Folder under `%LOCALAPPDATA%` that holds the user's downloaded models.
pub(crate) const USER_RUNTIME_DIR: &str = "LocalFlow";

/// The whisper.cpp programs, relative to a runtime folder.
pub(crate) const WHISPER_PROGRAM_DIR: &[&str] = &["whisper", "Release"];
pub(crate) const WHISPER_CLI_FILE: &str = "whisper-cli.exe";
pub(crate) const WHISPER_SERVER_FILE: &str = "whisper-server.exe";

pub(crate) fn whisper_cli_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = whisper_cli_override() {
        return require_file(path, "whisper.cpp executable");
    }

    find_file(
        "whisper.cpp executable",
        runtime_candidates(app)
            .iter()
            .map(|runtime| whisper_program_dir(runtime).join(WHISPER_CLI_FILE)),
    )
}

/// `LOCALFLOW_WHISPER_CLI`, honoured only in development builds: a release build never starts
/// a program from outside its own folders.
fn whisper_cli_override() -> Option<PathBuf> {
    let value = env::var_os("LOCALFLOW_WHISPER_CLI")?;
    if cfg!(feature = "dev-runtime") {
        Some(PathBuf::from(value))
    } else {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                "LOCALFLOW_WHISPER_CLI is ignored: this build runs only its own programs"
            );
        });
        None
    }
}

/// Folder holding `whisper-cli.exe`, `whisper-server.exe` and their DLLs under `runtime`.
pub(crate) fn whisper_program_dir(runtime: &Path) -> PathBuf {
    WHISPER_PROGRAM_DIR
        .iter()
        .fold(runtime.to_path_buf(), |dir, part| dir.join(part))
}

/// Resolve the speech model: an explicit environment override, then the path from settings,
/// then the best installed model.
pub(crate) fn speech_model_path(app: &AppHandle, configured: &str) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("LOCALFLOW_WHISPER_MODEL") {
        return require_file(PathBuf::from(path), "speech model");
    }

    let configured = configured.trim();
    if !configured.is_empty() {
        let path = PathBuf::from(configured);
        if path.is_file() {
            return Ok(path);
        }
        // A stale path must not break dictation; fall through to the bundled models.
        tracing::warn!("configured speech model path does not exist; using an installed model");
    }

    let runtimes = runtime_candidates(app);
    find_file(
        "speech model",
        preferred_model_candidates(&runtimes, PREFERRED_SPEECH_MODELS),
    )
}

/// Candidate model paths ordered by model preference first, runtime folder second, so the
/// preferred model is used wherever it is installed.
pub(crate) fn preferred_model_candidates(runtimes: &[PathBuf], preferred: &[&str]) -> Vec<PathBuf> {
    preferred
        .iter()
        .flat_map(|name| {
            runtimes
                .iter()
                .map(move |runtime| runtime.join("models").join(name))
        })
        .collect()
}

/// The runtime folders to search, in order (see the module documentation).
pub(crate) fn runtime_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let paths = app.path();
    ordered_runtime_dirs(
        paths.resource_dir().ok(),
        paths.local_data_dir().ok(),
        dev_runtime_dir(),
    )
}

/// The runtime folders for a resource folder, a local application data folder and an
/// optional development folder, in search order and without duplicates. Pure, for tests.
pub(crate) fn ordered_runtime_dirs(
    resource_dir: Option<PathBuf>,
    local_data_dir: Option<PathBuf>,
    dev_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let ordered = [
        resource_dir.map(|dir| dir.join(RESOURCE_RUNTIME_DIR)),
        local_data_dir.map(|dir| dir.join(USER_RUNTIME_DIR)),
        dev_dir,
    ];
    for candidate in ordered.into_iter().flatten() {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

/// The checkout's `.localflow-runtime`, in development builds only.
fn dev_runtime_dir() -> Option<PathBuf> {
    #[cfg(feature = "dev-runtime")]
    {
        Some(workspace_runtime_dir())
    }
    #[cfg(not(feature = "dev-runtime"))]
    {
        None
    }
}

/// `.localflow-runtime` next to the source tree this build was compiled from. Exists only
/// with the `dev-runtime` feature, so release builds do not embed the build machine's path.
#[cfg(feature = "dev-runtime")]
pub(crate) fn workspace_runtime_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".localflow-runtime")
}

pub(crate) fn require_file(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(missing_message(label, &[path]))
    }
}

pub(crate) fn find_file(
    label: &str,
    candidates: impl IntoIterator<Item = PathBuf>,
) -> Result<PathBuf, String> {
    let mut checked = Vec::new();

    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
        checked.push(candidate);
    }

    Err(missing_message(label, &checked))
}

/// "Missing speech model (a.gguf or b.bin); 6 locations checked. Run scripts\X.ps1." File
/// names only: the folders that were checked are counted, not named.
pub(crate) fn missing_message(label: &str, checked: &[PathBuf]) -> String {
    let mut names: Vec<String> = Vec::new();
    for path in checked {
        if let Some(name) = path.file_name() {
            let name = name.to_string_lossy().into_owned();
            if !names.contains(&name) {
                names.push(name);
            }
        }
    }

    let mut message = format!("Missing {label}");
    if !names.is_empty() {
        message.push_str(&format!(" ({})", names.join(" or ")));
    }
    if checked.len() > 1 {
        message.push_str(&format!("; {} locations checked", checked.len()));
    }
    message.push('.');
    if let Some(hint) = names.iter().find_map(|name| install_hint(name)) {
        message.push(' ');
        message.push_str(hint);
    }
    message
}

/// What installs a runtime file, for the "missing" messages.
pub(crate) fn install_hint(file_name: &str) -> Option<&'static str> {
    let name = file_name.to_ascii_lowercase();
    if name.starts_with("parakeet-") && name.ends_with(".gguf") {
        Some("Run scripts\\Get-AsrModel.ps1.")
    } else if name.starts_with("ggml-tiny.en") {
        Some("Run scripts\\Get-AsrModel.ps1 -Model tiny.en.")
    } else if name == WHISPER_CLI_FILE || name == WHISPER_SERVER_FILE {
        Some("Reinstall LocalFlow, or run scripts\\Get-WhisperRuntime.ps1.")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_models_are_ordered_by_model_before_runtime_folder() {
        let runtimes = vec![PathBuf::from("bundled"), PathBuf::from("dev")];

        let candidates = preferred_model_candidates(&runtimes, &["best.bin", "small.bin"]);

        assert_eq!(
            candidates,
            vec![
                PathBuf::from("bundled").join("models").join("best.bin"),
                PathBuf::from("dev").join("models").join("best.bin"),
                PathBuf::from("bundled").join("models").join("small.bin"),
                PathBuf::from("dev").join("models").join("small.bin"),
            ]
        );
    }

    #[test]
    fn runtime_folders_are_bundled_then_per_user_then_development() {
        let dirs = ordered_runtime_dirs(
            Some(PathBuf::from("app")),
            Some(PathBuf::from("local")),
            Some(PathBuf::from("checkout").join(".localflow-runtime")),
        );

        assert_eq!(
            dirs,
            vec![
                PathBuf::from("app").join("localflow-runtime"),
                PathBuf::from("local").join("LocalFlow"),
                PathBuf::from("checkout").join(".localflow-runtime"),
            ]
        );
    }

    #[test]
    fn missing_folders_are_skipped_and_duplicates_dropped() {
        assert_eq!(
            ordered_runtime_dirs(None, Some(PathBuf::from("local")), None),
            vec![PathBuf::from("local").join("LocalFlow")]
        );
        let same = PathBuf::from("local").join("LocalFlow");
        assert_eq!(
            ordered_runtime_dirs(None, Some(PathBuf::from("local")), Some(same.clone())),
            vec![same]
        );
        assert!(ordered_runtime_dirs(None, None, None).is_empty());
    }

    #[test]
    fn the_checkout_is_searched_only_in_development_builds() {
        assert_eq!(dev_runtime_dir().is_some(), cfg!(feature = "dev-runtime"));
    }

    #[test]
    fn whisper_programs_live_in_whisper_release() {
        assert_eq!(
            whisper_program_dir(Path::new("rt")).join(WHISPER_CLI_FILE),
            PathBuf::from("rt")
                .join("whisper")
                .join("Release")
                .join("whisper-cli.exe")
        );
    }

    #[test]
    fn missing_messages_name_files_and_scripts_but_no_folder() {
        let secret = PathBuf::from("C:\\Users\\someone-private\\AppData\\Local\\LocalFlow");
        let checked = preferred_model_candidates(
            &[secret.clone(), PathBuf::from("D:\\elsewhere")],
            PREFERRED_SPEECH_MODELS,
        );

        let message = find_file("speech model", checked).unwrap_err();

        assert_eq!(
            message,
            "Missing speech model (parakeet-tdt-0.6b-v3-Q5_K_M.gguf or ggml-tiny.en-q5_1.bin); \
             4 locations checked. Run scripts\\Get-AsrModel.ps1."
        );
        assert!(!message.contains("someone-private"));
        assert!(!message.contains("elsewhere"));
    }

    #[test]
    fn a_missing_single_file_names_only_the_file() {
        let message = require_file(
            PathBuf::from("C:\\Users\\someone-private\\tools\\whisper-cli.exe"),
            "whisper.cpp executable",
        )
        .unwrap_err();

        assert_eq!(
            message,
            "Missing whisper.cpp executable (whisper-cli.exe). Reinstall LocalFlow, or run \
             scripts\\Get-WhisperRuntime.ps1."
        );
    }

    #[test]
    fn unknown_files_get_no_install_hint() {
        assert_eq!(
            missing_message("speech model", &[PathBuf::from("x").join("custom.bin")]),
            "Missing speech model (custom.bin)."
        );
        assert_eq!(missing_message("thing", &[]), "Missing thing.");
        assert_eq!(
            install_hint("ggml-tiny.en-q5_1.bin"),
            Some("Run scripts\\Get-AsrModel.ps1 -Model tiny.en.")
        );
    }
}
