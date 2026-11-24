//! Every sentence LocalFlow speaks about coding-agent commands.
//!
//! Kept in one place so the read-back, the questions back and the results can be reviewed
//! together, and tested. Nothing here is logged.

use super::{
    agent::{AgentErrorKind, AskOutcome, ExecutionOutcome, LaunchFailure},
    grammar::Recognition,
    AgentCommand, AgentKind, CommandDraft, ProjectRef, Unavailability,
};

/// The read-back: what will happen on "yes", word for word where it matters.
pub fn readback(command: &AgentCommand) -> String {
    match command {
        AgentCommand::Open {
            agent: AgentKind::Cursor,
            project,
            task_was_said,
        } => {
            let task = if *task_was_said {
                " Cursor won't be given the task."
            } else {
                ""
            };
            format!("Open {} in Cursor?{task} Shall I?", project.spoken_name)
        }
        AgentCommand::Open {
            agent,
            project,
            task_was_said,
        } => {
            let then = if *task_was_said {
                "I won't type the task for you: dictate it in the new tab once it opens."
            } else {
                "Then dictate your task in the new tab."
            };
            format!(
                "Open {} in {}? {then} Shall I?",
                agent.display_name(),
                project.spoken_name
            )
        }
        AgentCommand::Ask {
            agent,
            project,
            question,
        } => format!(
            "Ask {}, in {}: {} Shall I send it?",
            agent.short_name(),
            project.spoken_name,
            with_final_punctuation(question)
        ),
    }
}

/// What to say for a recognition that is not a complete command. `None` for a complete
/// command (read it back instead) or for something that is not a command at all.
pub fn clarification(recognition: &Recognition, all_projects: &[ProjectRef]) -> Option<String> {
    Some(match recognition {
        Recognition::NotACommand | Recognition::Command(_) => return None,
        Recognition::UnknownProject { heard, .. } => format!(
            "I don't know a project called {heard}. Which project do you mean?{}",
            project_list(all_projects)
        ),
        Recognition::AmbiguousProject {
            heard, candidates, ..
        } => {
            let options: Vec<String> = candidates
                .iter()
                .map(|project| {
                    if project.parent_name.is_empty() {
                        project.spoken_name.clone()
                    } else {
                        format!("{} in {}", project.spoken_name, project.parent_name)
                    }
                })
                .collect();
            format!(
                "{heard} could mean more than one project: {}. Which one?",
                join_list(&options, "or")
            )
        }
        Recognition::MissingProject { draft } => {
            let question = match draft {
                CommandDraft::Open { agent, .. } => {
                    format!("Which project should I open {} in?", agent.display_name())
                }
                CommandDraft::Ask { agent, .. } => {
                    format!("Which project should {} look at?", agent.short_name())
                }
            };
            format!("{question}{}", project_list(all_projects))
        }
        Recognition::NoProjects { .. } => "You haven't added any projects yet. Add a project \
            folder in LocalFlow's settings, under Coding agents."
            .to_string(),
        Recognition::MissingQuestion { agent } => format!(
            "Say the whole question in one go, for example: ask {} what the tests in my \
             project check.",
            agent.short_name()
        ),
        Recognition::AgentNotAvailable { agent, reason } => not_available(*agent, *reason),
    })
}

pub fn not_available(agent: AgentKind, reason: Unavailability) -> String {
    let name = agent.display_name();
    match reason {
        Unavailability::NotSetUpYet => format!("{name} isn't set up for voice commands yet."),
        Unavailability::NotInstalled => format!("I couldn't find {name} on this computer."),
        Unavailability::NoTerminal => {
            format!("Opening {name} needs Windows Terminal, and I couldn't find it.")
        }
        Unavailability::MissingOption => format!(
            "The installed {name} is missing an option LocalFlow needs. Updating it may help."
        ),
    }
}

fn project_list(projects: &[ProjectRef]) -> String {
    const MAX_LISTED: usize = 5;
    if projects.is_empty() || projects.len() > MAX_LISTED {
        return String::new();
    }
    let names: Vec<String> = projects.iter().map(|p| p.spoken_name.clone()).collect();
    format!(" You have {}.", join_list(&names, "and"))
}

/// "a", "a and b", "a, b and c".
pub fn join_list(items: &[String], conjunction: &str) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} {conjunction} {last}", init.join(", ")),
    }
}

/// The user said something other than yes.
pub const DECLINED: &str = "OK, I won't.";
/// Nothing was said for the confirmation time limit.
pub const TIMED_OUT: &str = "I didn't hear a yes, so I've left it.";
/// The answer to "which project?" named none of them.
pub const STILL_UNCLEAR: &str = "I still don't know which project, so I've left it.";
/// "Never mind" while choosing a project.
pub const DROPPED: &str = "OK.";

/// Said right after "yes", before the command finishes.
pub fn starting(command: &AgentCommand) -> Option<String> {
    match command {
        AgentCommand::Ask { agent, .. } => Some(format!(
            "Asking {}. Say stop that to cancel.",
            agent.short_name()
        )),
        AgentCommand::Open { .. } => None,
    }
}

/// What to say once a command has been carried out.
pub fn outcome(command: &AgentCommand, outcome: &ExecutionOutcome) -> String {
    let project = &command.project().spoken_name;
    let agent = command.agent();
    match outcome {
        ExecutionOutcome::Opened => match agent {
            AgentKind::Cursor => format!("Opened {project} in Cursor."),
            _ => format!(
                "Opened {} in {project}. Dictate your task in the new tab and press Enter.",
                agent.display_name()
            ),
        },
        ExecutionOutcome::OpenFailed(failure) => {
            let reason = match failure {
                LaunchFailure::Unavailable(reason) => return not_available(agent, *reason),
                LaunchFailure::FolderUnusable => "the project folder can't be used any more",
                LaunchFailure::UnsafePath => "its path can't be passed safely",
                LaunchFailure::Refused => "the launch was refused for safety",
                LaunchFailure::Spawn => "the program didn't start",
            };
            format!("I couldn't open {}: {reason}.", agent.display_name())
        }
        ExecutionOutcome::Answered(answer) => match answer {
            AskOutcome::Answer(text) => claude_says(agent, text),
            AskOutcome::AgentError(kind) => match kind {
                AgentErrorKind::NotSignedIn => format!(
                    "{} needs you to sign in. Run claude in a terminal and log in, then ask \
                     again.",
                    agent.display_name()
                ),
                AgentErrorKind::MaxTurns => {
                    format!("{} stopped before finishing an answer.", agent.short_name())
                }
                AgentErrorKind::DuringExecution => {
                    format!(
                        "{} ran into an error and didn't answer.",
                        agent.short_name()
                    )
                }
                AgentErrorKind::Other => format!("{} couldn't answer that.", agent.short_name()),
            },
            AskOutcome::TimedOut => {
                format!("{} took too long, so I stopped it.", agent.short_name())
            }
            AskOutcome::Cancelled => "Stopped.".to_string(),
            AskOutcome::Unreadable => {
                format!("{}'s answer couldn't be read.", agent.short_name())
            }
            AskOutcome::CouldNotStart(failure) => match failure {
                LaunchFailure::Unavailable(reason) => not_available(agent, *reason),
                _ => format!("I couldn't start {}.", agent.display_name()),
            },
        },
    }
}

/// The answer, labelled as the agent's, made fit to speak: no markdown, no code blocks, and
/// not too long.
pub fn claude_says(agent: AgentKind, answer: &str) -> String {
    format!("{} says: {}", agent.short_name(), speakable(answer))
}

/// About thirty seconds of speech.
const MAX_SPOKEN_CHARS: usize = 600;

pub fn speakable(text: &str) -> String {
    let mut out = String::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            if !in_fence {
                out.push_str(" (code left out) ");
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        let line = line.trim_start_matches(|c: char| c == '#' || c == '>' || c.is_whitespace());
        let line = line
            .strip_prefix("- ")
            .or_else(|| line.strip_prefix("* "))
            .unwrap_or(line);
        out.push_str(&strip_links(line));
        out.push(' ');
    }
    let cleaned: String = out.chars().filter(|c| !matches!(c, '*' | '`')).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= MAX_SPOKEN_CHARS {
        return cleaned;
    }
    let cut: String = cleaned.chars().take(MAX_SPOKEN_CHARS).collect();
    match cut.rfind(['.', '?', '!']) {
        Some(end) if end > MAX_SPOKEN_CHARS / 3 => cut[..=end].to_string(),
        _ => format!("{}...", cut.trim_end()),
    }
}

/// "[text](url)" -> "text".
fn strip_links(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|i| open + i) else {
            break;
        };
        let Some(end) = rest[close..].find(')').map(|i| close + i) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

fn with_final_punctuation(text: &str) -> String {
    let text = text.trim();
    if text.ends_with(['.', '?', '!']) {
        text.to_string()
    } else {
        format!("{text}?")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn localflow() -> ProjectRef {
        ProjectRef {
            id: "lf".into(),
            path: PathBuf::from(r"C:\work\localflow"),
            spoken_name: "localflow".into(),
            parent_name: "work".into(),
        }
    }

    #[test]
    fn opening_claude_code_says_where_and_what_to_do_next() {
        let text = readback(&AgentCommand::Open {
            agent: AgentKind::ClaudeCode,
            project: localflow(),
            task_was_said: false,
        });
        assert_eq!(
            text,
            "Open Claude Code in localflow? Then dictate your task in the new tab. Shall I?"
        );
    }

    /// The read-back is spoken while the microphone is open. No stretch of it, heard back on
    /// its own, may count as the user's "yes": the confirmation must come from the user.
    #[test]
    fn no_part_of_a_read_back_is_itself_a_yes() {
        use super::super::{
            dialog::{classify_reply, Reply},
            grammar::UserSpeech,
        };
        let mut readbacks = Vec::new();
        for agent in [AgentKind::ClaudeCode, AgentKind::Cursor] {
            for task_was_said in [false, true] {
                readbacks.push(readback(&AgentCommand::Open {
                    agent,
                    project: localflow(),
                    task_was_said,
                }));
            }
        }
        readbacks.push(readback(&AgentCommand::Ask {
            agent: AgentKind::ClaudeCode,
            project: localflow(),
            question: "what does the overlay code do".into(),
        }));
        for text in &readbacks {
            let words: Vec<&str> = text.split_whitespace().collect();
            for start in 0..words.len() {
                for end in start + 1..=words.len() {
                    let stretch = words[start..end].join(" ");
                    assert_ne!(
                        classify_reply(UserSpeech::from_user_transcript(&stretch)),
                        Reply::Yes,
                        "{stretch:?} from {text:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_spoken_task_is_said_to_be_left_for_dictation() {
        let text = readback(&AgentCommand::Open {
            agent: AgentKind::ClaudeCode,
            project: localflow(),
            task_was_said: true,
        });
        assert!(text.contains("I won't type the task"), "{text}");
        assert!(text.contains("dictate it in the new tab"), "{text}");
        let cursor = readback(&AgentCommand::Open {
            agent: AgentKind::Cursor,
            project: localflow(),
            task_was_said: true,
        });
        assert_eq!(
            cursor,
            "Open localflow in Cursor? Cursor won't be given the task. Shall I?"
        );
    }

    #[test]
    fn a_question_is_read_back_word_for_word() {
        let text = readback(&AgentCommand::Ask {
            agent: AgentKind::ClaudeCode,
            project: localflow(),
            question: "what does the overlay code do".into(),
        });
        assert_eq!(
            text,
            "Ask Claude, in localflow: what does the overlay code do? Shall I send it?"
        );
    }

    #[test]
    fn questions_back_list_the_projects() {
        let projects = vec![localflow(), {
            let mut other = localflow();
            other.id = "pdf".into();
            other.spoken_name = "pdf viewer".into();
            other
        }];
        let text = clarification(
            &Recognition::UnknownProject {
                draft: CommandDraft::Open {
                    agent: AgentKind::ClaudeCode,
                    task_was_said: false,
                },
                heard: "foobar".into(),
            },
            &projects,
        )
        .unwrap();
        assert_eq!(
            text,
            "I don't know a project called foobar. Which project do you mean? You have localflow and pdf viewer."
        );
        let ambiguous = clarification(
            &Recognition::AmbiguousProject {
                draft: CommandDraft::Open {
                    agent: AgentKind::ClaudeCode,
                    task_was_said: false,
                },
                heard: "my app".into(),
                candidates: vec![localflow(), {
                    let mut other = localflow();
                    other.parent_name = "personal".into();
                    other
                }],
            },
            &projects,
        )
        .unwrap();
        assert_eq!(
            ambiguous,
            "my app could mean more than one project: localflow in work or localflow in personal. Which one?"
        );
        assert_eq!(clarification(&Recognition::NotACommand, &projects), None);
    }

    #[test]
    fn agents_not_set_up_use_the_agreed_sentence() {
        assert_eq!(
            not_available(AgentKind::Codex, Unavailability::NotSetUpYet),
            "Codex isn't set up for voice commands yet."
        );
        assert_eq!(
            not_available(AgentKind::CursorAgent, Unavailability::NotSetUpYet),
            "Cursor Agent isn't set up for voice commands yet."
        );
    }

    #[test]
    fn answers_start_with_claude_says_and_lose_their_markdown() {
        let spoken = claude_says(
            AgentKind::ClaudeCode,
            "## Overlay\nIt draws the **waveform**. See [the docs](https://x.y).\n```rust\nfn x() {}\n```\n- Uses `tauri`.",
        );
        assert_eq!(
            spoken,
            "Claude says: Overlay It draws the waveform. See the docs. (code left out) Uses tauri."
        );
    }

    #[test]
    fn long_answers_are_cut_at_a_sentence() {
        let long = "This is one sentence. ".repeat(100);
        let spoken = speakable(&long);
        assert!(spoken.len() <= MAX_SPOKEN_CHARS);
        assert!(spoken.ends_with('.'));
    }

    #[test]
    fn lists_read_naturally() {
        assert_eq!(join_list(&[], "and"), "");
        assert_eq!(join_list(&["a".into()], "and"), "a");
        assert_eq!(join_list(&["a".into(), "b".into()], "or"), "a or b");
        assert_eq!(
            join_list(&["a".into(), "b".into(), "c".into()], "and"),
            "a, b and c"
        );
    }
}
