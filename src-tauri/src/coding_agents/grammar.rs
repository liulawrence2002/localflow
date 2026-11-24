//! Recognising coding-agent commands with fixed rules.
//!
//! In the style of dragonfly: a declarative table of rules, each a sequence of named slots
//! (action word, agent, project, rest), matched against the words of the user's transcript.
//! No language model decides what runs. Matching is forgiving where it is safe (polite words,
//! "local flow" for "localflow", a few known mishearings of the agent's name) and strict
//! where it is not: an agent's name is required, and a project that is unknown, or named by
//! more than one project, is asked about instead of guessed.

use std::sync::OnceLock;

use super::{
    phrases::{words, PhraseReplacer, Word},
    projects::{ProjectIndex, ProjectLookup},
    spoken_forms::match_key,
    AgentAvailability, AgentCommand, AgentKind, CommandDraft, ProjectRef, Unavailability,
};

/// The user's own transcribed speech.
///
/// Only build this from what the microphone heard the user say. Page text, model replies and
/// the assistant's own voice must never be wrapped in it: the matcher accepts nothing else,
/// which is what keeps a web page or a reply from starting a program.
#[derive(Debug, Clone, Copy)]
pub struct UserSpeech<'a>(&'a str);

impl<'a> UserSpeech<'a> {
    pub fn from_user_transcript(transcript: &'a str) -> Self {
        Self(transcript)
    }

    pub fn text(&self) -> &'a str {
        self.0
    }
}

/// What the matcher needs to know about this computer and this user.
#[derive(Debug, Clone)]
pub struct CommandContext {
    pub projects: ProjectIndex,
    pub availability: AgentAvailability,
}

/// What a user turn turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recognition {
    /// Not a coding-agent command: answer it as usual.
    NotACommand,
    /// A complete command, to be read back and confirmed.
    Command(AgentCommand),
    /// The project named matches no project.
    UnknownProject { draft: CommandDraft, heard: String },
    /// The project named matches more than one project.
    AmbiguousProject {
        draft: CommandDraft,
        heard: String,
        candidates: Vec<ProjectRef>,
    },
    /// No project was named.
    MissingProject { draft: CommandDraft },
    /// No project has been added in settings yet.
    NoProjects { agent: AgentKind },
    /// "Ask Claude" with nothing after it.
    MissingQuestion { agent: AgentKind },
    /// The agent cannot do this on this computer.
    AgentNotAvailable {
        agent: AgentKind,
        reason: Unavailability,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Open,
    Ask,
}

/// One element of a rule.
#[derive(Debug, Clone, Copy)]
enum Slot {
    /// An action word of this kind ("open", "launch", "ask").
    Action(Action),
    /// An agent's name, after known mishearings are corrected.
    Agent,
    /// One of these words.
    Joiner(&'static [&'static str]),
    /// One of these words, or nothing.
    OptionalJoiner(&'static [&'static str]),
    /// One of the user's projects, by any of its spoken names; the longest match wins.
    Project,
    /// Anything, or nothing. Only its presence is used ("and fix the tests").
    Tail,
    /// Nothing, or words introduced by "and", "to" or punctuation.
    JoinedTail,
    /// One or more words, kept verbatim.
    Question,
}

struct Rule {
    slots: &'static [Slot],
}

/// Between an agent and the project: "open Claude Code *in* localflow".
const AGENT_TO_PROJECT: &[&str] = &["in", "on", "for", "inside", "at", "within", "into"];
/// Between a project and the agent: "open localflow *in* Cursor".
const PROJECT_TO_AGENT: &[&str] = &["in", "with", "using", "on", "into", "inside"];
/// Words that start a task: "and fix the tests".
const TASK_JOINERS: &[&str] = &["and", "to", "then", "so"];
/// Words that introduce a project inside a question: "what does the overlay do *in* localflow".
const QUESTION_PROJECT_JOINERS: &[&str] = &["in", "about", "for", "on", "of", "inside", "within"];

const RULES: &[Rule] = &[
    // "open Claude Code in localflow [and fix the failing tests]"
    Rule {
        slots: &[
            Slot::Action(Action::Open),
            Slot::Agent,
            Slot::OptionalJoiner(AGENT_TO_PROJECT),
            Slot::Project,
            Slot::Tail,
        ],
    },
    // "open localflow in Cursor"
    Rule {
        slots: &[
            Slot::Action(Action::Open),
            Slot::Project,
            Slot::Joiner(PROJECT_TO_AGENT),
            Slot::Agent,
            Slot::JoinedTail,
        ],
    },
    // "ask Claude what the overlay code in localflow does"
    Rule {
        slots: &[Slot::Action(Action::Ask), Slot::Agent, Slot::Question],
    },
];

/// Action words, as phrases.
const ACTION_WORDS: &[(&str, Action)] = &[
    ("open", Action::Open),
    ("open up", Action::Open),
    ("opened", Action::Open),
    ("start", Action::Open),
    ("start up", Action::Open),
    ("launch", Action::Open),
    ("fire up", Action::Open),
    ("run", Action::Open),
    ("ask", Action::Ask),
];

/// Agent names, as phrases, in their canonical spelling.
const AGENT_NAMES: &[(&str, AgentKind)] = &[
    ("claude code", AgentKind::ClaudeCode),
    ("claude", AgentKind::ClaudeCode),
    ("cursor", AgentKind::Cursor),
    ("cursor editor", AgentKind::Cursor),
    ("cursor agent", AgentKind::CursorAgent),
    ("cursor cli", AgentKind::CursorAgent),
    ("codex", AgentKind::Codex),
    ("codex cli", AgentKind::Codex),
    ("openai codex", AgentKind::Codex),
    ("open ai codex", AgentKind::Codex),
];

/// How speech recognition tends to mishear agent names. Applied only at the agent slot, so
/// a task or a question is never rewritten. "cursor" deliberately maps to itself.
const MISHEARINGS: &[(&str, &str)] = &[
    ("cloud code", "claude code"),
    ("clawed code", "claude code"),
    ("clod code", "claude code"),
    ("claud code", "claude code"),
    ("clause code", "claude code"),
    ("claude coat", "claude code"),
    ("cloud coat", "claude code"),
    ("clawd code", "claude code"),
    ("clawed", "claude"),
    ("clod", "claude"),
    ("claud", "claude"),
    ("clawd", "claude"),
    ("cursor", "cursor"),
    ("curser", "cursor"),
    ("curser agent", "cursor agent"),
    ("code x", "codex"),
    ("codecs", "codex"),
    ("code ex", "codex"),
];

/// Said at the start of a command without changing it.
const LEADING_POLITE: &[&str] = &[
    "please",
    "hey",
    "hi",
    "ok",
    "okay",
    "so",
    "um",
    "uh",
    "er",
    "now",
    "alright",
    "all right",
    "can you",
    "could you",
    "would you",
    "will you",
    "can you please",
    "could you please",
    "would you please",
    "go ahead and",
    "lets",
    "let us",
    "i want you to",
    "i want to",
    "id like you to",
    "id like to",
    "i would like you to",
    "i need you to",
];
const TRAILING_POLITE: &[&str] = &["please", "thanks", "now", "ok", "okay"];

struct Tables {
    actions: PhraseReplacer,
    agents: PhraseReplacer,
    polite: PhraseReplacer,
}

fn tables() -> &'static Tables {
    static TABLES: OnceLock<Tables> = OnceLock::new();
    TABLES.get_or_init(|| {
        let action_pairs: Vec<(&str, &str)> = ACTION_WORDS
            .iter()
            .map(|(phrase, action)| {
                (
                    *phrase,
                    match action {
                        Action::Open => "open",
                        Action::Ask => "ask",
                    },
                )
            })
            .collect();
        let mut agent_pairs: Vec<(&str, &str)> =
            AGENT_NAMES.iter().map(|(name, _)| (*name, *name)).collect();
        agent_pairs.extend_from_slice(MISHEARINGS);
        let polite_pairs: Vec<(&str, &str)> =
            LEADING_POLITE.iter().map(|phrase| (*phrase, "")).collect();
        Tables {
            actions: PhraseReplacer::new(&action_pairs),
            agents: PhraseReplacer::new(&agent_pairs),
            polite: PhraseReplacer::new(&polite_pairs),
        }
    })
}

fn action_at(words: &[Word], at: usize) -> Option<(Action, usize)> {
    let (canonical, used) = tables().actions.match_at(words, at)?;
    let action = match canonical.first()?.as_str() {
        "open" => Action::Open,
        _ => Action::Ask,
    };
    Some((action, used))
}

fn agent_at(words: &[Word], at: usize) -> Option<(AgentKind, usize)> {
    let (canonical, used) = tables().agents.match_at(words, at)?;
    let canonical = canonical.join(" ");
    AGENT_NAMES
        .iter()
        .find(|(name, _)| *name == canonical)
        .map(|(_, agent)| (*agent, used))
}

/// Whether a match key is also an agent's name, or a known mishearing of one. A project may
/// not be called that.
pub fn is_agent_name(key: &str) -> bool {
    AGENT_NAMES
        .iter()
        .map(|(name, _)| *name)
        .chain(MISHEARINGS.iter().map(|(heard, _)| *heard))
        .any(|name| match_key(name) == key)
}

/// What the slots captured.
#[derive(Debug, Clone, Default)]
struct Captures {
    action: Option<Action>,
    agent: Option<AgentKind>,
    /// The project's words, and what they matched (`None` when matching leniently).
    project: Option<(usize, usize, Option<ProjectLookup>)>,
    /// Words after the command, if any.
    tail: Option<(usize, usize)>,
    question: Option<(usize, usize)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// The project slot only accepts a known project.
    Strict,
    /// The project slot accepts any words, to say which name was not understood.
    Lenient,
}

fn match_slots(
    slots: &[Slot],
    words: &[Word],
    at: usize,
    captures: Captures,
    index: &ProjectIndex,
    mode: Mode,
) -> Option<Captures> {
    let Some((slot, rest)) = slots.split_first() else {
        return (at == words.len()).then_some(captures);
    };
    match *slot {
        Slot::Action(kind) => {
            let (action, used) = action_at(words, at)?;
            (action == kind).then_some(())?;
            let captures = Captures {
                action: Some(action),
                ..captures
            };
            match_slots(rest, words, at + used, captures, index, mode)
        }
        Slot::Agent => {
            let (agent, used) = agent_at(words, at)?;
            let captures = Captures {
                agent: Some(agent),
                ..captures
            };
            match_slots(rest, words, at + used, captures, index, mode)
        }
        Slot::Joiner(joiners) => {
            let word = words.get(at)?;
            joiners.contains(&word.text.as_str()).then_some(())?;
            match_slots(rest, words, at + 1, captures, index, mode)
        }
        Slot::OptionalJoiner(joiners) => {
            if words
                .get(at)
                .is_some_and(|word| joiners.contains(&word.text.as_str()))
            {
                if let Some(found) = match_slots(rest, words, at + 1, captures.clone(), index, mode)
                {
                    return Some(found);
                }
            }
            match_slots(rest, words, at, captures, index, mode)
        }
        Slot::Project => {
            if at >= words.len() {
                return None;
            }
            let mut ends: Vec<usize> = (at + 1..=words.len()).rev().collect();
            if mode == Mode::Lenient {
                // Try the words up to the first task word or punctuation first, so "open
                // Claude Code in foo and fix it" reports "foo" as the name not understood.
                let natural = (at + 1..words.len())
                    .find(|&i| {
                        words[i].after_break || TASK_JOINERS.contains(&words[i].text.as_str())
                    })
                    .unwrap_or(words.len());
                ends.retain(|&end| end != natural);
                ends.insert(0, natural);
            }
            for end in ends {
                let lookup = match mode {
                    Mode::Strict => {
                        let found = index.lookup(&words[at..end]);
                        if found == ProjectLookup::None {
                            continue;
                        }
                        Some(found)
                    }
                    Mode::Lenient => None,
                };
                let captures = Captures {
                    project: Some((at, end, lookup)),
                    ..captures.clone()
                };
                if let Some(found) = match_slots(rest, words, end, captures, index, mode) {
                    return Some(found);
                }
            }
            None
        }
        Slot::Tail => {
            let tail = (at < words.len()).then_some((at, words.len()));
            Some(Captures { tail, ..captures })
        }
        Slot::JoinedTail => {
            if at == words.len() {
                return Some(captures);
            }
            let word = &words[at];
            (word.after_break || TASK_JOINERS.contains(&word.text.as_str())).then_some(())?;
            Some(Captures {
                tail: Some((at, words.len())),
                ..captures
            })
        }
        Slot::Question => (at < words.len()).then_some(Captures {
            question: Some((at, words.len())),
            ..captures
        }),
    }
}

/// Recognise a coding-agent command in what the user said.
pub fn recognize(speech: UserSpeech<'_>, context: &CommandContext) -> Recognition {
    let transcript = speech.text();
    let all_words = words(transcript);
    let (start, end) = strip_polite(&all_words);
    if start >= end {
        return Recognition::NotACommand;
    }
    // Rules see the words without politeness; a question is cut from the full transcript.
    let core = &all_words[start..end];

    for mode in [Mode::Strict, Mode::Lenient] {
        for rule in RULES {
            if let Some(captures) = match_slots(
                rule.slots,
                core,
                0,
                Captures::default(),
                &context.projects,
                mode,
            ) {
                return build(captures, core, &all_words[start..], transcript, context);
            }
        }
    }

    // An action word and an agent's name, but no rule fits the rest: a command without a
    // project ("open Claude Code", "open Claude Code and fix the tests").
    if let Some((action, used)) = action_at(core, 0) {
        if let Some((agent, agent_used)) = agent_at(core, used) {
            let after = used + agent_used;
            let captures = Captures {
                action: Some(action),
                agent: Some(agent),
                tail: (after < core.len()).then_some((after, core.len())),
                ..Captures::default()
            };
            if action == Action::Ask && after >= core.len() {
                return match availability(action, agent, context) {
                    Ok(agent) => Recognition::MissingQuestion { agent },
                    Err(reason) => Recognition::AgentNotAvailable {
                        agent: unavailable_agent(action, agent),
                        reason,
                    },
                };
            }
            return build(captures, core, &all_words[start..], transcript, context);
        }
    }

    Recognition::NotACommand
}

/// Which agent actually does the work, and whether it can on this computer.
fn availability(
    action: Action,
    agent: AgentKind,
    context: &CommandContext,
) -> Result<AgentKind, Unavailability> {
    let available = &context.availability;
    match (action, agent) {
        (Action::Open, AgentKind::ClaudeCode) => available.open_claude.map(|()| agent),
        (Action::Open, AgentKind::Cursor) => available.open_cursor.map(|()| agent),
        (Action::Ask, AgentKind::ClaudeCode) => available.ask_claude.map(|()| agent),
        // The Cursor editor cannot answer questions; that would be the Cursor Agent.
        (_, AgentKind::Codex | AgentKind::CursorAgent) | (Action::Ask, AgentKind::Cursor) => {
            Err(Unavailability::NotSetUpYet)
        }
    }
}

fn unavailable_agent(action: Action, agent: AgentKind) -> AgentKind {
    match (action, agent) {
        (Action::Ask, AgentKind::Cursor) => AgentKind::CursorAgent,
        _ => agent,
    }
}

fn build(
    captures: Captures,
    core: &[Word],
    from_start: &[Word],
    transcript: &str,
    context: &CommandContext,
) -> Recognition {
    let (Some(action), Some(agent)) = (captures.action, captures.agent) else {
        return Recognition::NotACommand;
    };
    let agent = match availability(action, agent, context) {
        Ok(agent) => agent,
        Err(reason) => {
            return Recognition::AgentNotAvailable {
                agent: unavailable_agent(action, agent),
                reason,
            }
        }
    };
    if context.projects.is_empty() {
        return Recognition::NoProjects { agent };
    }

    match action {
        Action::Open => {
            let draft = CommandDraft::Open {
                agent,
                task_was_said: captures.tail.is_some(),
            };
            resolve_project(draft, captures.project, core, transcript, &context.projects)
        }
        Action::Ask => {
            let Some((question_start, _)) = captures.question else {
                return Recognition::MissingQuestion { agent };
            };
            let mut question_start = question_start;
            // "ask Claude to explain ..." asks "explain ...".
            if core[question_start].text == "to" && question_start + 1 < core.len() {
                question_start += 1;
            }
            // The question runs to the end of what was said, politeness and punctuation
            // included, exactly as the user said it.
            let question = transcript[core[question_start].start..].trim().to_string();
            let draft = CommandDraft::Ask { agent, question };
            let question_words = &from_start[question_start..];
            project_in_question(draft, question_words, &context.projects)
        }
    }
}

fn resolve_project(
    draft: CommandDraft,
    project: Option<(usize, usize, Option<ProjectLookup>)>,
    core: &[Word],
    transcript: &str,
    index: &ProjectIndex,
) -> Recognition {
    let Some((start, end, lookup)) = project else {
        return Recognition::MissingProject { draft };
    };
    let heard = transcript[core[start].start..core[end - 1].end].to_string();
    // A lenient match is a name that was not understood; a heard name that starts with a
    // task word means none was given ("open Claude Code and fix it").
    let lookup = match lookup {
        Some(found) => found,
        None if TASK_JOINERS.contains(&core[start].text.as_str()) => {
            let draft = match draft {
                CommandDraft::Open { agent, .. } => CommandDraft::Open {
                    agent,
                    task_was_said: true,
                },
                other => other,
            };
            return Recognition::MissingProject { draft };
        }
        None => index.lookup(&core[start..end]),
    };
    match lookup {
        ProjectLookup::One(project) => Recognition::Command(draft.complete(project)),
        ProjectLookup::Many(candidates) => Recognition::AmbiguousProject {
            draft,
            heard,
            candidates,
        },
        ProjectLookup::None => Recognition::UnknownProject { draft, heard },
    }
}

/// Find which project a question is about. A name introduced by "in" or "about" is
/// preferred over one that merely appears; two different projects are asked about.
fn project_in_question(
    draft: CommandDraft,
    question: &[Word],
    index: &ProjectIndex,
) -> Recognition {
    let mut introduced: Vec<ProjectRef> = Vec::new();
    let mut bare: Vec<ProjectRef> = Vec::new();
    let mut heard_names: Vec<String> = Vec::new();
    let mut at = 0;
    while at < question.len() {
        let Some((found, end)) = index.longest_match_at(question, at) else {
            at += 1;
            continue;
        };
        let is_introduced =
            at > 0 && QUESTION_PROJECT_JOINERS.contains(&question[at - 1].text.as_str());
        let target = if is_introduced {
            &mut introduced
        } else {
            &mut bare
        };
        let found = match found {
            ProjectLookup::One(project) => vec![project],
            ProjectLookup::Many(projects) => projects,
            ProjectLookup::None => Vec::new(),
        };
        for project in found {
            if !target.iter().any(|known| known.id == project.id) {
                target.push(project);
            }
        }
        heard_names.push(super::phrases::join(&question[at..end]));
        at = end;
    }
    let candidates = if introduced.is_empty() {
        bare
    } else {
        introduced
    };
    match candidates.len() {
        0 => Recognition::MissingProject { draft },
        1 => Recognition::Command(draft.complete(candidates.into_iter().next().unwrap())),
        _ => Recognition::AmbiguousProject {
            draft,
            heard: heard_names.join(", "),
            candidates,
        },
    }
}

/// The range of words left once polite words are dropped from both ends.
fn strip_polite(words: &[Word]) -> (usize, usize) {
    let mut start = 0;
    while let Some((_, used)) = tables().polite.match_at(words, start) {
        start += used;
    }
    let mut end = words.len();
    loop {
        if end > start && TRAILING_POLITE.contains(&words[end - 1].text.as_str()) {
            end -= 1;
        } else if end >= start + 2
            && ((words[end - 2].text == "thank" && words[end - 1].text == "you")
                || (words[end - 2].text == "for" && words[end - 1].text == "me"))
        {
            end -= 2;
        } else {
            break;
        }
    }
    (start, end)
}

/// Resolve the user's answer to "which project?" against the projects offered (or all of
/// them when `candidates` is empty). Accepts a spoken name, the parent folder's name for
/// same-named projects ("the one in Documents"), or "the first one".
pub fn resolve_project_reply(
    reply: UserSpeech<'_>,
    candidates: &[ProjectRef],
    index: &ProjectIndex,
) -> Option<ProjectRef> {
    let heard = words(reply.text());
    let (start, end) = strip_polite(&heard);
    let heard = &heard[start..end];
    if heard.is_empty() {
        return None;
    }
    let offered = |project: &ProjectRef| {
        candidates.is_empty() || candidates.iter().any(|c| c.id == project.id)
    };

    // A spoken name, possibly inside a longer reply ("I meant localflow").
    let mut named: Vec<ProjectRef> = Vec::new();
    for at in 0..heard.len() {
        if let Some((found, _)) = index.longest_match_at(heard, at) {
            let found = match found {
                ProjectLookup::One(project) => vec![project],
                ProjectLookup::Many(projects) => projects,
                ProjectLookup::None => Vec::new(),
            };
            for project in found.into_iter().filter(|p| offered(p)) {
                if !named.iter().any(|known| known.id == project.id) {
                    named.push(project);
                }
            }
        }
    }
    if named.len() == 1 {
        return named.pop();
    }

    // Among same-named candidates, the folder they are in.
    if !candidates.is_empty() {
        let reply_key = match_key(&super::phrases::join(heard));
        let by_parent: Vec<&ProjectRef> = candidates
            .iter()
            .filter(|c| {
                let parent = match_key(&c.parent_name);
                !parent.is_empty() && reply_key.contains(&parent)
            })
            .collect();
        if by_parent.len() == 1 {
            return Some(by_parent[0].clone());
        }

        const ORDINALS: &[(&str, usize)] = &[
            ("first", 0),
            ("second", 1),
            ("third", 2),
            ("fourth", 3),
            ("last", usize::MAX),
        ];
        let ordinal = heard.iter().find_map(|word| {
            ORDINALS
                .iter()
                .find(|(name, _)| *name == word.text)
                .map(|(_, position)| *position)
        });
        if let Some(position) = ordinal {
            let position = if position == usize::MAX {
                candidates.len() - 1
            } else {
                position
            };
            return candidates.get(position).cloned();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coding_agents::projects::CodingProject;

    fn project(id: &str, path: &str, names: &[&str]) -> CodingProject {
        CodingProject {
            id: id.to_string(),
            path: path.to_string(),
            spoken_names: names.iter().map(|name| name.to_string()).collect(),
        }
    }

    fn context() -> CommandContext {
        CommandContext {
            projects: ProjectIndex::new(&[
                project("lf", r"C:\Users\me\Documents\localflow", &["localflow"]),
                project(
                    "pdf",
                    r"C:\Users\me\Documents\clarovo-pdf-viewer",
                    &["pdf viewer", "clarovo"],
                ),
                project("app1", r"C:\work\app", &["my app"]),
                project("app2", r"C:\personal\app", &["my app", "side app"]),
            ]),
            availability: AgentAvailability::all(),
        }
    }

    fn hear(text: &str) -> Recognition {
        recognize(UserSpeech::from_user_transcript(text), &context())
    }

    fn open(agent: AgentKind, project: &str, task: bool) -> (AgentKind, String, bool) {
        (agent, project.to_string(), task)
    }

    fn as_open(recognition: &Recognition) -> Option<(AgentKind, String, bool)> {
        match recognition {
            Recognition::Command(AgentCommand::Open {
                agent,
                project,
                task_was_said,
            }) => Some((*agent, project.id.clone(), *task_was_said)),
            _ => None,
        }
    }

    fn as_ask(recognition: &Recognition) -> Option<(AgentKind, String, String)> {
        match recognition {
            Recognition::Command(AgentCommand::Ask {
                agent,
                project,
                question,
            }) => Some((*agent, project.id.clone(), question.clone())),
            _ => None,
        }
    }

    #[test]
    fn open_claude_code_in_a_project() {
        for phrase in [
            "Open Claude Code in localflow",
            "open claude code in local flow.",
            "Open Claude Code in LocalFlow, please.",
            "Please open Claude Code in the localflow project",
            "Can you open Claude Code in localflow?",
            "Could you please launch Claude Code in localflow",
            "Start Claude Code in localflow",
            "Fire up Claude Code for localflow",
            "Open Claude in localflow",
            "Open Claude Code localflow",
            "Hey, open up Claude Code in localflow",
        ] {
            assert_eq!(
                as_open(&hear(phrase)),
                Some(open(AgentKind::ClaudeCode, "lf", false)),
                "{phrase}"
            );
        }
    }

    #[test]
    fn a_task_after_the_project_is_noticed_but_not_kept() {
        for phrase in [
            "Open Claude Code in localflow and fix the failing tests",
            "Open Claude Code in localflow to fix the failing tests",
            "Open Claude Code in localflow, fix the failing tests.",
            "open claude code in local flow then run the linter",
            "Launch Claude Code in localflow and; rm -rf / & echo \"hi\"",
        ] {
            assert_eq!(
                as_open(&hear(phrase)),
                Some(open(AgentKind::ClaudeCode, "lf", true)),
                "{phrase}"
            );
        }
    }

    #[test]
    fn misheard_agent_names_are_corrected() {
        for phrase in [
            "open cloud code in localflow",
            "open clawed code in localflow",
            "open clod code in localflow",
            "Open Claude coat in localflow",
        ] {
            assert_eq!(
                as_open(&hear(phrase)),
                Some(open(AgentKind::ClaudeCode, "lf", false)),
                "{phrase}"
            );
        }
        assert_eq!(
            as_open(&hear("open localflow in curser")),
            Some(open(AgentKind::Cursor, "lf", false))
        );
    }

    #[test]
    fn mishearings_are_never_applied_to_the_question() {
        let (_, _, question) = as_ask(&hear(
            "Ask clawed why the cloud code module in localflow is slow",
        ))
        .unwrap();
        assert_eq!(question, "why the cloud code module in localflow is slow");
    }

    #[test]
    fn open_a_project_in_cursor() {
        for phrase in [
            "Open localflow in Cursor",
            "open local flow in cursor.",
            "Open the PDF viewer in Cursor",
            "open clarovo with cursor please",
            "Open Cursor in localflow",
        ] {
            let found = as_open(&hear(phrase));
            assert!(
                matches!(found, Some((AgentKind::Cursor, _, false))),
                "{phrase}: {found:?}"
            );
        }
        assert_eq!(
            as_open(&hear("Open localflow in Claude Code")),
            Some(open(AgentKind::ClaudeCode, "lf", false))
        );
    }

    #[test]
    fn ask_claude_about_a_project() {
        assert_eq!(
            as_ask(&hear("Ask Claude what the overlay code in localflow does")),
            Some((
                AgentKind::ClaudeCode,
                "lf".to_string(),
                "what the overlay code in localflow does".to_string()
            ))
        );
        assert_eq!(
            as_ask(&hear(
                "Ask Claude Code to explain the settings screen in localflow."
            )),
            Some((
                AgentKind::ClaudeCode,
                "lf".to_string(),
                "explain the settings screen in localflow.".to_string()
            ))
        );
        assert_eq!(
            as_ask(&hear("ask claude about the PDF viewer")).map(|(_, id, _)| id),
            Some("pdf".to_string())
        );
        // A bare mention counts when nothing introduces a project, possessive or not.
        assert_eq!(
            as_ask(&hear("Ask Claude what localflow's tray menu does")).map(|(_, id, _)| id),
            Some("lf".to_string())
        );
        assert_eq!(
            as_ask(&hear("Ask Claude how localflow starts")).map(|(_, id, _)| id),
            Some("lf".to_string())
        );
    }

    #[test]
    fn the_question_keeps_the_users_exact_words() {
        let (_, _, question) = as_ask(&hear(
            "Ask Claude: what's the \"overlay\" code in LocalFlow for?",
        ))
        .unwrap();
        assert_eq!(question, "what's the \"overlay\" code in LocalFlow for?");
    }

    #[test]
    fn ordinary_questions_and_near_misses_are_not_commands() {
        for phrase in [
            "What does open source mean?",
            "Can you open the window?",
            "Open the pod bay doors",
            "How do I open Claude Code?",
            "Ask me a question",
            "Ask my boss about localflow",
            "Tell Claude Code to open localflow",
            "I opened the file in localflow yesterday",
            "Open the file in cursor position three",
            "What is the cursor doing in localflow?",
            "Is Claude Code better than Cursor?",
            "Cloud code is a kind of infrastructure",
            "open",
            "please",
            "",
            "Run the tests",
            "Start the timer for five minutes",
            "Launch the rocket",
        ] {
            assert_eq!(hear(phrase), Recognition::NotACommand, "{phrase}");
        }
    }

    #[test]
    fn an_unknown_project_is_asked_about() {
        match hear("Open Claude Code in foobar and fix it") {
            Recognition::UnknownProject { draft, heard } => {
                assert_eq!(heard, "foobar");
                assert_eq!(
                    draft,
                    CommandDraft::Open {
                        agent: AgentKind::ClaudeCode,
                        task_was_said: true
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            hear("Open the kitchen sink in Cursor"),
            Recognition::UnknownProject { heard, .. } if heard == "the kitchen sink"
        ));
    }

    #[test]
    fn a_name_shared_by_two_projects_is_asked_about() {
        match hear("Open Claude Code in my app") {
            Recognition::AmbiguousProject {
                heard, candidates, ..
            } => {
                assert_eq!(heard, "my app");
                let ids: Vec<&str> = candidates.iter().map(|p| p.id.as_str()).collect();
                assert_eq!(ids, ["app1", "app2"]);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            hear("Ask Claude to compare localflow with the pdf viewer"),
            Recognition::AmbiguousProject { .. }
        ));
    }

    #[test]
    fn a_missing_project_is_asked_about() {
        for phrase in [
            "Open Claude Code",
            "Open Claude Code please",
            "Open Claude Code and fix the tests",
            "Ask Claude why the build is slow",
        ] {
            assert!(
                matches!(hear(phrase), Recognition::MissingProject { .. }),
                "{phrase}: {:?}",
                hear(phrase)
            );
        }
        assert_eq!(
            hear("Ask Claude"),
            Recognition::MissingQuestion {
                agent: AgentKind::ClaudeCode
            }
        );
    }

    #[test]
    fn agents_that_are_not_set_up_say_so() {
        for (phrase, agent) in [
            ("Open Codex in localflow", AgentKind::Codex),
            (
                "Ask Codex what the overlay does in localflow",
                AgentKind::Codex,
            ),
            ("open code x in localflow", AgentKind::Codex),
            ("Open Cursor Agent in localflow", AgentKind::CursorAgent),
            (
                "Ask Cursor why the tests fail in localflow",
                AgentKind::CursorAgent,
            ),
        ] {
            assert_eq!(
                hear(phrase),
                Recognition::AgentNotAvailable {
                    agent,
                    reason: Unavailability::NotSetUpYet
                },
                "{phrase}"
            );
        }
    }

    #[test]
    fn agents_missing_from_this_computer_say_so() {
        let context = CommandContext {
            availability: AgentAvailability {
                open_claude: Err(Unavailability::NoTerminal),
                ask_claude: Err(Unavailability::NotInstalled),
                open_cursor: Err(Unavailability::NotInstalled),
            },
            ..context()
        };
        let hear = |text| recognize(UserSpeech::from_user_transcript(text), &context);
        assert_eq!(
            hear("open claude code in localflow"),
            Recognition::AgentNotAvailable {
                agent: AgentKind::ClaudeCode,
                reason: Unavailability::NoTerminal
            }
        );
        assert_eq!(
            hear("ask claude what this does in localflow"),
            Recognition::AgentNotAvailable {
                agent: AgentKind::ClaudeCode,
                reason: Unavailability::NotInstalled
            }
        );
        assert_eq!(
            hear("open localflow in cursor"),
            Recognition::AgentNotAvailable {
                agent: AgentKind::Cursor,
                reason: Unavailability::NotInstalled
            }
        );
    }

    #[test]
    fn with_no_projects_the_user_is_told_to_add_one() {
        let context = CommandContext {
            projects: ProjectIndex::new(&[]),
            availability: AgentAvailability::all(),
        };
        assert_eq!(
            recognize(
                UserSpeech::from_user_transcript("open claude code in localflow"),
                &context
            ),
            Recognition::NoProjects {
                agent: AgentKind::ClaudeCode
            }
        );
    }

    #[test]
    fn replies_to_which_project_are_resolved() {
        let context = context();
        let reply = |text| UserSpeech::from_user_transcript(text);
        let all = context.projects.all_refs();
        let resolved = |text, candidates: &[ProjectRef]| {
            resolve_project_reply(reply(text), candidates, &context.projects).map(|p| p.id)
        };
        assert_eq!(resolved("localflow", &[]), Some("lf".to_string()));
        assert_eq!(
            resolved("I meant the PDF viewer, please", &[]),
            Some("pdf".to_string())
        );
        assert_eq!(resolved("my app", &[]), None);
        let same_named: Vec<ProjectRef> = all
            .iter()
            .filter(|p| p.id.starts_with("app"))
            .cloned()
            .collect();
        assert_eq!(
            resolved("the one in personal", &same_named),
            Some("app2".to_string())
        );
        assert_eq!(
            resolved("the first one", &same_named),
            Some("app1".to_string())
        );
        assert_eq!(resolved("side app", &same_named), Some("app2".to_string()));
        assert_eq!(resolved("localflow", &same_named), None);
        assert_eq!(resolved("never mind", &[]), None);
    }

    #[test]
    fn agent_names_are_recognised_as_such() {
        assert!(is_agent_name("cursor"));
        assert!(is_agent_name("claudecode"));
        assert!(is_agent_name("cloudcode"));
        assert!(!is_agent_name("localflow"));
    }
}
