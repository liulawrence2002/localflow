//! Tauri commands for the "Coding agents" settings: projects and detection.
//!
//! None of these starts an agent. Launching only ever happens from a confirmed voice command
//! in a conversation, never from the web UI.

use std::{path::Path, sync::Arc};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::{
    agent::{CodingAgents, CodingAgentsStatus},
    projects::{
        clean_spoken_names, validate_folder, CodingAgentSettings, CodingProject, ProjectIndex,
    },
    spoken_forms::suggest_spoken_names,
};
use crate::app_state::LocalFlowRuntime;

/// The agents, shared by the settings screen and the conversation.
pub struct CodingAgentsRuntime {
    agents: Arc<CodingAgents>,
}

impl Default for CodingAgentsRuntime {
    fn default() -> Self {
        Self {
            agents: Arc::new(CodingAgents::system()),
        }
    }
}

impl CodingAgentsRuntime {
    pub fn agents(&self) -> Arc<CodingAgents> {
        self.agents.clone()
    }
}

/// A project as the settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodingProjectView {
    #[serde(flatten)]
    pub project: CodingProject,
    pub folder_name: String,
    /// Why the folder can no longer be used, if it cannot.
    pub folder_problem: Option<String>,
    /// Spoken names another project also has; saying one is asked about.
    pub shared_names: Vec<String>,
}

/// A project to add (no `id`) or change.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodingProjectInput {
    pub id: Option<String>,
    pub path: String,
    pub spoken_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpokenNameSuggestion {
    /// The folder as it would be stored, when it is usable.
    pub folder: Option<String>,
    pub folder_name: String,
    pub suggestions: Vec<String>,
    /// Why the folder cannot be used.
    pub problem: Option<String>,
}

const MAX_PROJECTS: usize = 50;

pub fn project_views(settings: &CodingAgentSettings) -> Vec<CodingProjectView> {
    let index = ProjectIndex::new(&settings.projects);
    settings
        .projects
        .iter()
        .map(|project| CodingProjectView {
            folder_name: project.folder_name(),
            folder_problem: validate_folder(&project.path)
                .err()
                .map(|problem| problem.message()),
            shared_names: index.shared_names(&project.id),
            project: project.clone(),
        })
        .collect()
}

/// Add or change a project after checking its folder and names.
pub fn upsert_project(
    settings: &mut CodingAgentSettings,
    input: &CodingProjectInput,
    new_id: impl FnOnce() -> String,
) -> Result<String, String> {
    let folder = validate_folder(&input.path).map_err(|problem| problem.message())?;
    let names =
        clean_spoken_names(&input.spoken_names, &folder).map_err(|problem| problem.message())?;
    let path = folder.to_string_lossy().into_owned();
    let same_folder = |project: &CodingProject| project.path.eq_ignore_ascii_case(&path);

    match input.id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => {
            if settings
                .projects
                .iter()
                .any(|project| project.id != id && same_folder(project))
            {
                return Err("That folder is already added as another project.".to_string());
            }
            let project = settings
                .projects
                .iter_mut()
                .find(|project| project.id == id)
                .ok_or_else(|| "That project no longer exists.".to_string())?;
            project.path = path;
            project.spoken_names = names;
            Ok(id.to_string())
        }
        None => {
            if settings.projects.iter().any(same_folder) {
                return Err("That folder is already added.".to_string());
            }
            if settings.projects.len() >= MAX_PROJECTS {
                return Err(format!("At most {MAX_PROJECTS} projects can be added."));
            }
            let id = new_id();
            settings.projects.push(CodingProject {
                id: id.clone(),
                path,
                spoken_names: names,
            });
            Ok(id)
        }
    }
}

pub fn suggestion_for(path: &str) -> SpokenNameSuggestion {
    match validate_folder(path) {
        Ok(folder) => {
            let folder_name = folder
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            SpokenNameSuggestion {
                suggestions: suggest_spoken_names(&folder_name),
                folder: Some(folder.to_string_lossy().into_owned()),
                folder_name,
                problem: None,
            }
        }
        Err(problem) => SpokenNameSuggestion {
            folder: None,
            folder_name: Path::new(path.trim())
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            suggestions: Vec::new(),
            problem: Some(problem.message()),
        },
    }
}

fn new_project_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    format!(
        "project-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn persist(
    app: &AppHandle,
) -> impl FnOnce(&crate::app_state::SettingsSnapshot) -> Result<(), String> + '_ {
    move |settings| {
        crate::storage::save_settings(app, settings)
            .map_err(|error| format!("Could not persist settings: {error}"))
    }
}

#[tauri::command]
pub fn list_coding_projects(runtime: State<'_, LocalFlowRuntime>) -> Vec<CodingProjectView> {
    project_views(&runtime.current_settings().coding_agents)
}

// `async`: off the main thread, because saving writes the settings database.
#[tauri::command(async)]
pub fn save_coding_project(
    app: AppHandle,
    runtime: State<'_, LocalFlowRuntime>,
    project: CodingProjectInput,
) -> Result<Vec<CodingProjectView>, String> {
    runtime.update_settings(
        |settings| upsert_project(&mut settings.coding_agents, &project, new_project_id).map(drop),
        persist(&app),
    )?;
    Ok(project_views(&runtime.current_settings().coding_agents))
}

#[tauri::command(async)]
pub fn remove_coding_project(
    app: AppHandle,
    runtime: State<'_, LocalFlowRuntime>,
    id: String,
) -> Result<Vec<CodingProjectView>, String> {
    runtime.update_settings(
        |settings| {
            settings
                .coding_agents
                .projects
                .retain(|project| project.id != id);
            Ok(())
        },
        persist(&app),
    )?;
    Ok(project_views(&runtime.current_settings().coding_agents))
}

#[tauri::command]
pub fn suggest_project_spoken_names(path: String) -> SpokenNameSuggestion {
    suggestion_for(&path)
}

/// Look for the agents again. Runs `claude --version` and `--help` only.
#[tauri::command]
pub async fn detect_coding_agents(
    runtime: State<'_, CodingAgentsRuntime>,
) -> Result<CodingAgentsStatus, String> {
    let agents = runtime.agents();
    tauri::async_runtime::spawn_blocking(move || agents.detect_all())
        .await
        .map_err(|_| "Detection stopped unexpectedly.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn here() -> String {
        env!("CARGO_MANIFEST_DIR").to_string()
    }

    fn input(path: &str, names: &[&str]) -> CodingProjectInput {
        CodingProjectInput {
            id: None,
            path: path.to_string(),
            spoken_names: names.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn a_project_is_added_with_suggested_names_when_none_are_given() {
        let mut settings = CodingAgentSettings::default();
        let id = upsert_project(&mut settings, &input(&here(), &[]), || "p1".into()).unwrap();
        assert_eq!(id, "p1");
        assert_eq!(settings.projects[0].path, here());
        assert_eq!(settings.projects[0].spoken_names, ["src tauri"]);
    }

    #[test]
    fn the_same_folder_cannot_be_added_twice() {
        let mut settings = CodingAgentSettings::default();
        upsert_project(&mut settings, &input(&here(), &["app"]), || "p1".into()).unwrap();
        let again = upsert_project(
            &mut settings,
            &input(&format!("{}\\", here().to_uppercase()), &["other"]),
            || "p2".into(),
        );
        assert_eq!(again, Err("That folder is already added.".to_string()));
    }

    #[test]
    fn editing_changes_names_and_keeps_the_id() {
        let mut settings = CodingAgentSettings::default();
        upsert_project(&mut settings, &input(&here(), &["app"]), || "p1".into()).unwrap();
        let edit = CodingProjectInput {
            id: Some("p1".into()),
            ..input(&here(), &["tauri side", "backend"])
        };
        upsert_project(&mut settings, &edit, || unreachable!()).unwrap();
        assert_eq!(settings.projects.len(), 1);
        assert_eq!(settings.projects[0].spoken_names, ["tauri side", "backend"]);

        let missing = CodingProjectInput {
            id: Some("nope".into()),
            ..input(&here(), &[])
        };
        assert!(upsert_project(&mut settings, &missing, || unreachable!()).is_err());
    }

    #[test]
    fn bad_folders_and_names_explain_themselves() {
        let mut settings = CodingAgentSettings::default();
        let error =
            upsert_project(&mut settings, &input(r"C:\a;b", &[]), || "x".into()).unwrap_err();
        assert!(error.contains("semicolon"), "{error}");
        let error =
            upsert_project(&mut settings, &input(&here(), &["cursor"]), || "x".into()).unwrap_err();
        assert!(error.contains("coding agent"), "{error}");
        assert!(settings.projects.is_empty());
    }

    #[test]
    fn views_flag_shared_names_and_missing_folders() {
        let settings = CodingAgentSettings {
            projects: vec![
                CodingProject {
                    id: "a".into(),
                    path: here(),
                    spoken_names: vec!["app".into()],
                },
                CodingProject {
                    id: "b".into(),
                    path: r"C:\definitely\gone".into(),
                    spoken_names: vec!["app".into(), "old app".into()],
                },
            ],
        };
        let views = project_views(&settings);
        assert_eq!(views[0].shared_names, ["app"]);
        assert_eq!(views[0].folder_problem, None);
        assert_eq!(views[1].folder_name, "gone");
        assert!(views[1].folder_problem.is_some());
        let json = serde_json::to_value(&views[0]).unwrap();
        assert_eq!(json["spokenNames"][0], "app");
        assert_eq!(json["folderName"], "src-tauri");
    }

    #[test]
    fn suggestions_come_with_the_folder_problem_if_any() {
        let suggestion = suggestion_for(&here());
        assert_eq!(suggestion.suggestions, ["src tauri"]);
        assert_eq!(suggestion.folder.as_deref(), Some(here().as_str()));
        let bad = suggestion_for(r"C:\work\100%");
        assert!(bad.problem.unwrap().contains("percent"));
        assert!(bad.suggestions.is_empty());
    }
}
