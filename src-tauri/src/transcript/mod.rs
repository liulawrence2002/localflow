//! Deterministic transcript formatting applied before the local LLM cleanup.
//!
//! These transformations are testable and reproducible, so we do them here instead of
//! asking the LLM to do work that can be made exact (spec §3.3 / §5): resolve explicit
//! self-corrections, remove non-lexical fillers, collapse stutters, convert spoken
//! punctuation, and normalize whitespace. The result (`deterministic_text`) is fed to the
//! refinement model as a starting point and is also the safe fallback when the model is
//! unavailable or returns invalid output.
//!
//! This is the authoritative implementation for the native production path; the TypeScript
//! `src/domain/personalization.ts` remains only for the browser dev preview.

use regex::Regex;

/// A user "exact replacement" rule (e.g. a common misrecognition -> correct spelling).
pub struct Replacement {
    pub incorrect: String,
    pub correct: String,
    pub enabled: bool,
}

/// A user snippet: a spoken trigger phrase expanded to canned text.
pub struct Snippet {
    pub trigger: String,
    pub expansion: String,
    pub enabled: bool,
}

/// Apply the full deterministic formatting pipeline to a raw ASR transcript, with no
/// personalization.
pub fn apply_deterministic_formatting(raw: &str) -> String {
    apply_deterministic_formatting_with(raw, &[], &[])
}

/// Apply deterministic formatting plus user snippets and exact replacements. Ordering
/// mirrors the shared TypeScript helper: resolve self-corrections, remove fillers/stutters,
/// expand snippets, apply replacements, then spoken punctuation and normalization.
pub fn apply_deterministic_formatting_with(
    raw: &str,
    replacements: &[Replacement],
    snippets: &[Snippet],
) -> String {
    let corrected = resolve_self_corrections(raw);
    if corrected.is_empty() {
        return String::new();
    }

    let without_fillers = remove_fillers(&corrected);
    let deduped = collapse_repeated_words(&without_fillers);

    let snippet_pairs: Vec<(&str, &str)> = snippets
        .iter()
        .filter(|snippet| snippet.enabled && !snippet.trigger.trim().is_empty())
        .map(|snippet| (snippet.trigger.as_str(), snippet.expansion.as_str()))
        .collect();
    let expanded = apply_token_replacements(&deduped, &snippet_pairs);

    let replacement_pairs: Vec<(&str, &str)> = replacements
        .iter()
        .filter(|rule| rule.enabled && !rule.incorrect.trim().is_empty())
        .map(|rule| (rule.incorrect.as_str(), rule.correct.as_str()))
        .collect();
    let replaced = apply_token_replacements(&expanded, &replacement_pairs);

    let punctuated = apply_spoken_punctuation(&replaced);
    let normalized = normalize_whitespace(&punctuated);
    capitalize_sentences(&normalized)
}

/// Replace whole-word occurrences of each `from` phrase with its `to` value,
/// case-insensitively. The replacement is inserted literally (no `$group` expansion).
fn apply_token_replacements(text: &str, pairs: &[(&str, &str)]) -> String {
    let mut result = text.to_string();

    for (from, to) in pairs {
        if from.trim().is_empty() {
            continue;
        }
        let pattern = format!(r"(?i)\b{}\b", regex::escape(from));
        if let Ok(regex) = Regex::new(&pattern) {
            result = regex
                .replace_all(&result, |_: &regex::Captures| (*to).to_string())
                .into_owned();
        }
    }

    result
}

/// Map a spoken punctuation phrase (already lowercased) to the literal it produces.
fn spoken_punctuation(phrase: &str) -> Option<&'static str> {
    match phrase {
        "comma" => Some(","),
        "period" | "full stop" => Some("."),
        "question mark" => Some("?"),
        "exclamation mark" | "exclamation point" => Some("!"),
        "colon" => Some(":"),
        "semicolon" => Some(";"),
        "new line" => Some("\n"),
        "new paragraph" => Some("\n\n"),
        "open parenthesis" => Some("("),
        "close parenthesis" => Some(")"),
        "quote" => Some("\""),
        "bullet point" => Some("\n-"),
        _ => None,
    }
}

/// Resolve explicit spoken self-corrections, keeping the corrected intent. Mirrors the
/// tested behavior of the shared TypeScript helper.
fn resolve_self_corrections(text: &str) -> String {
    let normalized = collapse_inline_whitespace(text);
    if normalized.is_empty() {
        return normalized;
    }

    let restart = Regex::new(r"(?i)\blet me restart\b[,.]?\s*(.+)$").unwrap();
    if let Some(captures) = restart.captures(&normalized) {
        return captures.get(1).unwrap().as_str().trim().to_string();
    }

    let actually = Regex::new(r"(?i)^(.+?)\s+actually\s+(.+)$").unwrap();
    if let Some(captures) = actually.captures(&normalized) {
        let before = captures.get(1).unwrap().as_str();
        if word_count(before) >= 3 {
            return captures.get(2).unwrap().as_str().trim().to_string();
        }
    }

    let sorry = Regex::new(r"(?i)^(.+?)\s+sorry\s+(.+)$").unwrap();
    if let Some(captures) = sorry.captures(&normalized) {
        return replace_last_token(
            captures.get(1).unwrap().as_str(),
            captures.get(2).unwrap().as_str(),
        );
    }

    let no = Regex::new(r"(?i)^((?:\S+\s+){2,}\S+)\s+no\s+(\S+(?:\s+\S+){0,2})$").unwrap();
    if let Some(captures) = no.captures(&normalized) {
        return replace_last_token(
            captures.get(1).unwrap().as_str(),
            captures.get(2).unwrap().as_str(),
        );
    }

    normalized
}

/// Drop the final whitespace-delimited token of `before` (only if there is one) and append
/// `after`, matching the shared helper's `/\s+\S+$/` semantics.
fn replace_last_token(before: &str, after: &str) -> String {
    let before = before.trim();
    let prefix = match before.rfind(char::is_whitespace) {
        Some(index) => before[..index].trim().to_string(),
        None => before.to_string(),
    };
    format!("{} {}", prefix, after.trim()).trim().to_string()
}

/// Remove non-lexical fillers (um, uh, erm, hmm and their elongations) as whole words.
fn remove_fillers(text: &str) -> String {
    let fillers = Regex::new(r"(?i)\b(?:u+m+|u+h+m?|erm+|h+m+)\b").unwrap();
    collapse_inline_whitespace(&fillers.replace_all(text, ""))
}

/// Collapse immediately repeated words (stutters) such as "the the" -> "the".
fn collapse_repeated_words(text: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();

    for token in text.split_whitespace() {
        if let Some(previous) = kept.last() {
            if is_wordy(token) && previous.eq_ignore_ascii_case(token) {
                continue;
            }
        }
        kept.push(token);
    }

    kept.join(" ")
}

fn is_wordy(token: &str) -> bool {
    !token.is_empty() && token.chars().all(char::is_alphanumeric)
}

/// Replace standalone spoken-punctuation phrases (one or two words) with their literals.
/// Uses token windows rather than regex lookahead so adjacent commands both convert.
fn apply_spoken_punctuation(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut output: Vec<String> = Vec::new();
    let mut index = 0;

    while index < words.len() {
        if index + 1 < words.len() {
            let pair = format!("{} {}", words[index], words[index + 1]).to_lowercase();
            if let Some(punctuation) = spoken_punctuation(&pair) {
                output.push(punctuation.to_string());
                index += 2;
                continue;
            }
        }

        if let Some(punctuation) = spoken_punctuation(&words[index].to_lowercase()) {
            output.push(punctuation.to_string());
            index += 1;
            continue;
        }

        output.push(words[index].to_string());
        index += 1;
    }

    output.join(" ")
}

/// Normalize spacing around punctuation and newlines. Deliberately does NOT force a space
/// *after* punctuation, which would corrupt URLs, emails, and decimals — spoken-punctuation
/// tokens are already space-separated, so a trailing space is present when it should be.
fn normalize_whitespace(text: &str) -> String {
    let steps: [(Regex, &str); 7] = [
        (Regex::new(r"\n{3,}-").unwrap(), "\n\n-"),
        (Regex::new(r"\s+([,.;:?!])").unwrap(), "$1"),
        (Regex::new(r"\(\s+").unwrap(), "("),
        (Regex::new(r"\s+\)").unwrap(), ")"),
        (Regex::new(r"[ \t]+\n").unwrap(), "\n"),
        (Regex::new(r"\n[ \t]+").unwrap(), "\n"),
        (Regex::new(r"[ \t]{2,}").unwrap(), " "),
    ];

    let mut result = text.to_string();
    for (pattern, replacement) in steps.iter() {
        result = pattern.replace_all(&result, *replacement).into_owned();
    }
    result.trim().to_string()
}

/// Capitalize the first letter of the text and of each sentence. A sentence boundary is a
/// terminator (`.`/`!`/`?`) or newline followed by whitespace, so URL/email/decimal dots
/// (`b.com`, `3.14`) are left untouched.
fn capitalize_sentences(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut at_sentence_start = true;
    let mut previous_was_terminator = false;

    for ch in text.chars() {
        if ch.is_whitespace() {
            result.push(ch);
            if previous_was_terminator || ch == '\n' {
                at_sentence_start = true;
            }
            continue;
        }

        if at_sentence_start && ch.is_alphabetic() {
            for upper in ch.to_uppercase() {
                result.push(upper);
            }
            at_sentence_start = false;
        } else if at_sentence_start && matches!(ch, '-' | '*' | '•' | '(' | '"' | '\'') {
            // Leading list markers / opening brackets do not end the sentence start, so the
            // item's first letter still capitalizes ("- first" -> "- First").
            result.push(ch);
        } else {
            result.push(ch);
            at_sentence_start = false;
        }
        previous_was_terminator = matches!(ch, '.' | '!' | '?');
    }

    result
}

fn collapse_inline_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| !token.is_empty())
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_spoken_punctuation_and_capitalizes() {
        let result = apply_deterministic_formatting("hello comma world period new line next");
        assert_eq!(result, "Hello, world.\nNext");
    }

    #[test]
    fn adjacent_spoken_commands_both_convert() {
        let result = apply_deterministic_formatting("done period new paragraph then");
        assert_eq!(result, "Done.\n\nThen");
    }

    #[test]
    fn resolves_actually_self_correction() {
        // Needs at least three words before "actually".
        let result = apply_deterministic_formatting("meet me tuesday actually wednesday");
        assert_eq!(result, "Wednesday");
    }

    #[test]
    fn resolves_no_backtrack_correction() {
        let result = apply_deterministic_formatting("the meeting is tuesday no wednesday");
        assert_eq!(result, "The meeting is wednesday");
    }

    #[test]
    fn resolves_let_me_restart() {
        let result =
            apply_deterministic_formatting("some rambling let me restart the plan is ready");
        assert_eq!(result, "The plan is ready");
    }

    #[test]
    fn removes_fillers_and_stutters() {
        let result = apply_deterministic_formatting("um the the plan uh is ready");
        assert_eq!(result, "The plan is ready");
    }

    #[test]
    fn preserves_urls_emails_and_decimals() {
        // No space forced after in-token dots; no capitalization of the domain.
        let result = apply_deterministic_formatting("email me at test@example.com about 3.14");
        assert_eq!(result, "Email me at test@example.com about 3.14");
    }

    #[test]
    fn does_not_remove_words_containing_filler_substrings() {
        let result = apply_deterministic_formatting("summer umbrella huh");
        assert_eq!(result, "Summer umbrella huh");
    }

    #[test]
    fn empty_input_returns_empty() {
        assert_eq!(apply_deterministic_formatting("   "), "");
    }

    #[test]
    fn bullet_point_starts_a_list_line() {
        let result =
            apply_deterministic_formatting("tasks colon bullet point first bullet point second");
        assert_eq!(result, "Tasks:\n- First\n- Second");
    }

    #[test]
    fn applies_replacements_case_insensitively() {
        let replacements = vec![Replacement {
            incorrect: "pie torch".to_string(),
            correct: "PyTorch".to_string(),
            enabled: true,
        }];
        let result = apply_deterministic_formatting_with("i love Pie Torch", &replacements, &[]);
        assert_eq!(result, "I love PyTorch");
    }

    #[test]
    fn expands_enabled_snippets() {
        let snippets = vec![Snippet {
            trigger: "my sig".to_string(),
            expansion: "Best, Ada".to_string(),
            enabled: true,
        }];
        let result = apply_deterministic_formatting_with("add my sig here", &[], &snippets);
        assert_eq!(result, "Add Best, Ada here");
    }

    #[test]
    fn disabled_rules_are_ignored() {
        let replacements = vec![Replacement {
            incorrect: "pie torch".to_string(),
            correct: "PyTorch".to_string(),
            enabled: false,
        }];
        let result = apply_deterministic_formatting_with("pie torch", &replacements, &[]);
        assert_eq!(result, "Pie torch");
    }
}
