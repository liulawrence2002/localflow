//! Where the local speech binaries and models live: packaged resources first, then the
//! development runtime folder.

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

pub(crate) fn whisper_cli_path(app: &AppHandle) -> Result<PathBuf, String> {
    if let Ok(path) = env::var("LOCALFLOW_WHISPER_CLI") {
        return require_file(PathBuf::from(path), "whisper.cpp executable");
    }

    find_file(
        "whisper.cpp executable",
        runtime_candidates(app).into_iter().map(|runtime| {
            runtime
                .join("whisper")
                .join("Release")
                .join("whisper-cli.exe")
        }),
    )
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

pub(crate) fn runtime_candidates(app: &AppHandle) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(resource_dir) = app.path().resource_dir() {
        candidates.push(resource_dir.join("localflow-runtime"));
    }

    candidates.push(workspace_runtime_dir());
    candidates
}

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
        Err(format!("Missing {label}: {}", path.display()))
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
        checked.push(candidate.display().to_string());
    }

    Err(format!("Missing {label}. Checked: {}", checked.join("; ")))
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
}
