//! The user's project folders, their spoken names, and finding a project from what was said.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use super::{
    phrases::{join, Word},
    spoken_forms::{match_key, match_keys, suggest_spoken_names},
    ProjectRef,
};

/// The "Coding agents" part of the persisted settings.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodingAgentSettings {
    pub projects: Vec<CodingProject>,
}

/// One project folder the user can name in a command.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CodingProject {
    pub id: String,
    /// An existing absolute folder, checked by [`validate_folder`] when it was saved.
    pub path: String,
    /// What the user says to mean this project, most preferred first.
    pub spoken_names: Vec<String>,
}

impl CodingProject {
    pub fn folder_name(&self) -> String {
        folder_name(Path::new(&self.path))
    }

    /// The name used when speaking about this project: the first spoken name.
    pub fn spoken_name(&self) -> String {
        self.spoken_names
            .first()
            .cloned()
            .unwrap_or_else(|| self.folder_name())
    }

    pub fn to_ref(&self) -> ProjectRef {
        let path = PathBuf::from(&self.path);
        ProjectRef {
            id: self.id.clone(),
            spoken_name: self.spoken_name(),
            parent_name: path.parent().map(folder_name).unwrap_or_default(),
            path,
        }
    }
}

fn folder_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// Why a folder cannot be used. Each has a sentence for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderProblem {
    Empty,
    TooLong,
    ControlCharacter,
    /// `;`, `%` or `"`. Windows Terminal reads `;` as the start of a new command even inside
    /// quotes, and expands `%NAME%` with no way to escape it.
    TerminalCharacter(char),
    DevicePath,
    NotAbsolute,
    NotFound,
    NotAFolder,
}

impl FolderProblem {
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "Enter the full path of a project folder.".to_string(),
            Self::TooLong => "That path is too long.".to_string(),
            Self::ControlCharacter => "The path contains a line break or another control character.".to_string(),
            Self::TerminalCharacter(';') => "The path contains a semicolon. Windows Terminal treats ; as the start of a new command, even inside quotes, so LocalFlow cannot open this folder safely. Rename the folder or choose another.".to_string(),
            Self::TerminalCharacter('%') => "The path contains a percent sign. Windows Terminal replaces %NAME% with an environment variable and cannot be told not to, so LocalFlow cannot open this folder safely. Rename the folder or choose another.".to_string(),
            Self::TerminalCharacter(other) => format!("The path contains {other}, which cannot be passed to Windows Terminal safely. Remove any quotes around the path."),
            Self::DevicePath => "Use an ordinary folder path such as C:\\Users\\you\\projects\\app.".to_string(),
            Self::NotAbsolute => "Enter the full path, starting with a drive letter such as C:\\.".to_string(),
            Self::NotFound => "No folder exists at that path.".to_string(),
            Self::NotAFolder => "That path is a file, not a folder.".to_string(),
        }
    }
}

/// Characters that may not appear in a folder or program path given to Windows Terminal.
pub const TERMINAL_UNSAFE_CHARACTERS: [char; 3] = [';', '%', '"'];

const MAX_PATH_CHARS: usize = 1024;

/// Checks that never touch the disk: shared by settings and by every launch.
pub fn check_path_text(path: &str) -> Result<(), FolderProblem> {
    if path.is_empty() {
        return Err(FolderProblem::Empty);
    }
    if path.chars().count() > MAX_PATH_CHARS {
        return Err(FolderProblem::TooLong);
    }
    if path.chars().any(char::is_control) {
        return Err(FolderProblem::ControlCharacter);
    }
    if let Some(bad) = path
        .chars()
        .find(|c| TERMINAL_UNSAFE_CHARACTERS.contains(c))
    {
        return Err(FolderProblem::TerminalCharacter(bad));
    }
    if path.starts_with(r"\\?\") || path.starts_with(r"\\.\") {
        return Err(FolderProblem::DevicePath);
    }
    Ok(())
}

/// Tidy a typed folder path and check it is an existing absolute folder that Windows
/// Terminal can be given. Forward slashes become backslashes and a trailing separator is
/// dropped (except on a drive root), so the same folder is always stored the same way.
pub fn validate_folder(input: &str) -> Result<PathBuf, FolderProblem> {
    let trimmed = input.trim();
    check_path_text(trimmed)?;
    let mut normalized = trimmed.replace('/', "\\");
    while normalized.len() > 3 && normalized.ends_with('\\') {
        normalized.pop();
    }
    let path = PathBuf::from(&normalized);
    if !path.is_absolute() {
        return Err(FolderProblem::NotAbsolute);
    }
    match std::fs::metadata(&path) {
        Ok(metadata) if metadata.is_dir() => Ok(path),
        Ok(_) => Err(FolderProblem::NotAFolder),
        Err(_) => Err(FolderProblem::NotFound),
    }
}

/// Why a spoken name cannot be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NameProblem {
    NoWords(String),
    TooLong(String),
    /// The name is also how an agent is called ("cursor", "claude code").
    AgentName(String),
}

impl NameProblem {
    pub fn message(&self) -> String {
        match self {
            Self::NoWords(name) => format!("\"{name}\" has no letters or numbers to say."),
            Self::TooLong(name) => format!("\"{name}\" is too long to say as a name."),
            Self::AgentName(name) => format!(
                "\"{name}\" is also the name of a coding agent, so commands using it would be unclear."
            ),
        }
    }
}

const MAX_NAME_CHARS: usize = 60;
const MAX_NAMES: usize = 8;

/// Clean the spoken names the user typed: trimmed, inner spaces collapsed, duplicates (as
/// they would be heard) dropped. With none left, names are suggested from the folder name.
pub fn clean_spoken_names(names: &[String], folder: &Path) -> Result<Vec<String>, NameProblem> {
    let mut cleaned: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for name in names {
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            continue;
        }
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(NameProblem::TooLong(name));
        }
        let key = match_key(&name);
        if key.is_empty() {
            return Err(NameProblem::NoWords(name));
        }
        if super::grammar::is_agent_name(&key) {
            return Err(NameProblem::AgentName(name));
        }
        if !keys.contains(&key) {
            keys.push(key);
            cleaned.push(name);
        }
    }
    cleaned.truncate(MAX_NAMES);
    if cleaned.is_empty() {
        cleaned = suggest_spoken_names(&folder_name(folder))
            .into_iter()
            .filter(|name| !super::grammar::is_agent_name(&match_key(name)))
            .collect();
    }
    if cleaned.is_empty() {
        return Err(NameProblem::NoWords(folder_name(folder)));
    }
    Ok(cleaned)
}

/// What a heard phrase matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectLookup {
    None,
    One(ProjectRef),
    /// The phrase is a spoken name of more than one project.
    Many(Vec<ProjectRef>),
}

/// Words dropped from the ends of a heard project name: "the localflow project".
const LEADING_FILLERS: &[&str] = &["the", "my", "our", "this"];
const TRAILING_FILLERS: &[&str] = &[
    "project",
    "folder",
    "repo",
    "repository",
    "directory",
    "codebase",
    "workspace",
];

/// Every project's spoken names, by the key they are compared with.
#[derive(Debug, Clone, Default)]
pub struct ProjectIndex {
    projects: Vec<CodingProject>,
    by_key: HashMap<String, Vec<usize>>,
    longest_key: usize,
}

impl ProjectIndex {
    pub fn new(projects: &[CodingProject]) -> Self {
        let mut by_key: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, project) in projects.iter().enumerate() {
            for name in &project.spoken_names {
                for key in match_keys(name) {
                    let owners = by_key.entry(key).or_default();
                    if !owners.contains(&index) {
                        owners.push(index);
                    }
                }
            }
        }
        let longest_key = by_key.keys().map(String::len).max().unwrap_or(0);
        Self {
            projects: projects.to_vec(),
            by_key,
            longest_key,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.projects.is_empty()
    }

    pub fn projects(&self) -> &[CodingProject] {
        &self.projects
    }

    pub fn all_refs(&self) -> Vec<ProjectRef> {
        self.projects.iter().map(CodingProject::to_ref).collect()
    }

    /// Look up exactly these words as a project name, ignoring "the" and "project" around
    /// them, and a possessive "s" at the end ("localflow's" is heard as "localflows").
    pub fn lookup(&self, heard: &[Word]) -> ProjectLookup {
        let found = self.lookup_with_fillers(heard);
        if found != ProjectLookup::None {
            return found;
        }
        let Some(last) = heard.last() else {
            return ProjectLookup::None;
        };
        match last.text.strip_suffix('s') {
            Some(stem) if !stem.is_empty() => {
                let mut trimmed = heard.to_vec();
                if let Some(last) = trimmed.last_mut() {
                    last.text = stem.to_string();
                }
                self.lookup_with_fillers(&trimmed)
            }
            _ => ProjectLookup::None,
        }
    }

    fn lookup_with_fillers(&self, heard: &[Word]) -> ProjectLookup {
        let exact = self.lookup_key(&match_key(&join(heard)));
        if exact != ProjectLookup::None {
            return exact;
        }
        let mut start = 0;
        let mut end = heard.len();
        while start < end && LEADING_FILLERS.contains(&heard[start].text.as_str()) {
            start += 1;
        }
        while end > start && TRAILING_FILLERS.contains(&heard[end - 1].text.as_str()) {
            end -= 1;
        }
        if (start == 0 && end == heard.len()) || start == end {
            return ProjectLookup::None;
        }
        self.lookup_key(&match_key(&join(&heard[start..end])))
    }

    pub fn lookup_text(&self, heard: &str) -> ProjectLookup {
        self.lookup(&super::phrases::words(heard))
    }

    fn lookup_key(&self, key: &str) -> ProjectLookup {
        match self.by_key.get(key).map(Vec::as_slice) {
            None | Some([]) => ProjectLookup::None,
            Some([only]) => ProjectLookup::One(self.projects[*only].to_ref()),
            Some(many) => {
                ProjectLookup::Many(many.iter().map(|&i| self.projects[i].to_ref()).collect())
            }
        }
    }

    /// The longest run of words starting at `start` that names a project, and where it ends.
    /// The run stops at punctuation: a name is said in one go.
    pub fn longest_match_at(&self, words: &[Word], start: usize) -> Option<(ProjectLookup, usize)> {
        let mut best = None;
        let mut key_length = 0;
        for end in start + 1..=words.len() {
            if end > start + 1 && words[end - 1].after_break {
                break;
            }
            key_length += words[end - 1].text.len();
            // Filler words can make a heard name a little longer than any stored key.
            if key_length > self.longest_key + 24 {
                break;
            }
            let found = self.lookup(&words[start..end]);
            if found != ProjectLookup::None {
                best = Some((found, end));
            }
        }
        best
    }

    /// For each project, the spoken names it shares with another project. Such a name is
    /// asked about rather than guessed, so settings warns about it.
    pub fn shared_names(&self, project_id: &str) -> Vec<String> {
        let Some(index) = self.projects.iter().position(|p| p.id == project_id) else {
            return Vec::new();
        };
        self.projects[index]
            .spoken_names
            .iter()
            .filter(|name| {
                self.by_key
                    .get(&match_key(name))
                    .is_some_and(|owners| owners.iter().any(|&owner| owner != index))
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::phrases::words;
    use super::*;

    pub(crate) fn project(id: &str, path: &str, names: &[&str]) -> CodingProject {
        CodingProject {
            id: id.to_string(),
            path: path.to_string(),
            spoken_names: names.iter().map(|name| name.to_string()).collect(),
        }
    }

    #[test]
    fn folders_with_terminal_separator_characters_are_rejected_with_a_reason() {
        for (path, bad) in [
            (r"C:\work\a;b", ';'),
            (r"C:\work\100%", '%'),
            (r#""C:\work\app""#, '"'),
        ] {
            let problem = validate_folder(path).unwrap_err();
            assert_eq!(problem, FolderProblem::TerminalCharacter(bad), "{path}");
            assert!(!problem.message().is_empty());
        }
        assert!(FolderProblem::TerminalCharacter(';')
            .message()
            .contains("new command"));
        assert!(FolderProblem::TerminalCharacter('%')
            .message()
            .contains("environment variable"));
    }

    #[test]
    fn folders_must_be_absolute_existing_directories() {
        assert_eq!(validate_folder("   "), Err(FolderProblem::Empty));
        assert_eq!(
            validate_folder(r"work\app"),
            Err(FolderProblem::NotAbsolute)
        );
        assert_eq!(
            validate_folder(r"\work\app"),
            Err(FolderProblem::NotAbsolute)
        );
        assert_eq!(
            validate_folder(r"C:\definitely\not\a\real\folder\here"),
            Err(FolderProblem::NotFound)
        );
        assert_eq!(
            validate_folder("C:\\a\nb"),
            Err(FolderProblem::ControlCharacter)
        );
        assert_eq!(
            validate_folder(r"\\?\C:\Windows"),
            Err(FolderProblem::DevicePath)
        );

        let manifest = env!("CARGO_MANIFEST_DIR");
        assert_eq!(
            validate_folder(&format!(r"{manifest}\Cargo.toml")),
            Err(FolderProblem::NotAFolder)
        );
        let folder = validate_folder(&format!("  {}/src/  ", manifest.replace('\\', "/"))).unwrap();
        assert_eq!(folder, PathBuf::from(format!(r"{manifest}\src")));
    }

    #[test]
    fn a_drive_root_keeps_its_backslash() {
        assert_eq!(validate_folder(r"C:\").unwrap(), PathBuf::from(r"C:\"));
    }

    #[test]
    fn spoken_names_are_cleaned_and_deduplicated_as_heard() {
        let folder = Path::new(r"C:\work\LocalFlow");
        let names = clean_spoken_names(
            &[
                "  local   flow ".to_string(),
                "LocalFlow".to_string(),
                String::new(),
                "the app".to_string(),
            ],
            folder,
        )
        .unwrap();
        assert_eq!(names, ["local flow", "the app"]);
    }

    #[test]
    fn with_no_names_the_folder_name_is_suggested() {
        let names = clean_spoken_names(&[], Path::new(r"C:\work\my-API_v2")).unwrap();
        assert_eq!(names, ["my api v two"]);
    }

    #[test]
    fn agent_names_and_unsayable_names_are_refused() {
        let folder = Path::new(r"C:\work\app");
        assert!(matches!(
            clean_spoken_names(&["Cursor".to_string()], folder),
            Err(NameProblem::AgentName(_))
        ));
        assert!(matches!(
            clean_spoken_names(&["claude code".to_string()], folder),
            Err(NameProblem::AgentName(_))
        ));
        assert!(matches!(
            clean_spoken_names(&["!!!".to_string()], folder),
            Err(NameProblem::NoWords(_))
        ));
    }

    #[test]
    fn heard_names_match_with_or_without_spaces() {
        let index = ProjectIndex::new(&[project("p1", r"C:\work\localflow", &["localflow"])]);
        for heard in [
            "localflow",
            "local flow",
            "Local-Flow",
            "the localflow project",
        ] {
            assert!(
                matches!(index.lookup_text(heard), ProjectLookup::One(ref p) if p.id == "p1"),
                "{heard}"
            );
        }
        assert_eq!(index.lookup_text("flow"), ProjectLookup::None);
        assert_eq!(index.lookup_text("the"), ProjectLookup::None);
    }

    #[test]
    fn a_name_shared_by_two_projects_is_ambiguous_not_the_shortest() {
        let index = ProjectIndex::new(&[
            project("a", r"C:\work\app", &["app"]),
            project("b", r"C:\personal\app-v2", &["app", "app two"]),
        ]);
        match index.lookup_text("app") {
            ProjectLookup::Many(found) => {
                let ids: Vec<&str> = found.iter().map(|p| p.id.as_str()).collect();
                assert_eq!(ids, ["a", "b"]);
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
        assert!(matches!(index.lookup_text("app 2"), ProjectLookup::One(ref p) if p.id == "b"));
        assert_eq!(index.shared_names("a"), ["app"]);
        assert_eq!(index.shared_names("b"), ["app"]);
    }

    #[test]
    fn the_longest_name_at_a_position_wins() {
        let index = ProjectIndex::new(&[
            project("short", r"C:\work\local", &["local"]),
            project("long", r"C:\work\localflow", &["local flow"]),
        ]);
        let heard = words("local flow and fix the tests");
        let (found, end) = index.longest_match_at(&heard, 0).unwrap();
        assert!(matches!(found, ProjectLookup::One(ref p) if p.id == "long"));
        assert_eq!(end, 2);
        assert!(index.longest_match_at(&heard, 3).is_none());
    }

    #[test]
    fn project_refs_carry_the_spoken_and_parent_names() {
        let reference = project("p", r"C:\Users\me\Documents\localflow", &["local flow"]).to_ref();
        assert_eq!(reference.spoken_name, "local flow");
        assert_eq!(reference.parent_name, "Documents");
    }
}
