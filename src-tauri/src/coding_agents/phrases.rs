//! Words of a transcript, and a table of known mishearings for the command words.

use std::collections::HashMap;

/// One word of a transcript, lower case, with apostrophes removed ("that's" -> "thats").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    /// Whether a comma, full stop or other punctuation came between this word and the one
    /// before it. "Open Claude Code in localflow, fix the tests" has a break before "fix".
    pub after_break: bool,
    /// Where the word is in the original transcript, in bytes, so a question can be taken
    /// word for word with its own capitals and punctuation.
    pub start: usize,
    pub end: usize,
}

/// Split a transcript into words. Letters and digits make words; apostrophes are dropped;
/// anything else separates words, and punctuation also marks a break.
pub fn words(text: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut start = 0;
    let mut end = 0;
    let mut pending_break = false;
    for (offset, character) in text.char_indices() {
        if character.is_alphanumeric() {
            if current.is_empty() {
                start = offset;
            }
            current.extend(character.to_lowercase());
            end = offset + character.len_utf8();
        } else if matches!(character, '\'' | '\u{2019}') {
            // "that's" -> "thats"
            if !current.is_empty() {
                end = offset + character.len_utf8();
            }
        } else {
            if !current.is_empty() {
                words.push(Word {
                    text: std::mem::take(&mut current),
                    after_break: pending_break,
                    start,
                    end,
                });
                pending_break = false;
            }
            if !character.is_whitespace() && !words.is_empty() {
                pending_break = true;
            }
        }
    }
    if !current.is_empty() {
        words.push(Word {
            text: current,
            after_break: pending_break,
            start,
            end,
        });
    }
    words
}

/// Replaces spoken phrases with other phrases, longest first.
///
/// Ported from the idea of Talon community's `PhraseReplacer`
/// (https://github.com/talonhub/community, MIT licence): phrases are indexed by their first
/// word, and at each position the longest phrase that matches wins. Here it is only ever
/// asked about one position at a time ([`PhraseReplacer::match_at`]), so the grammar can
/// apply it to the command words and nothing else: a task or a question is never rewritten.
pub struct PhraseReplacer {
    /// First word -> (number of further words, further words -> replacement), sorted so the
    /// longest phrases are tried first.
    index: HashMap<String, Vec<(usize, HashMap<Vec<String>, Vec<String>>)>>,
}

impl PhraseReplacer {
    pub fn new(pairs: &[(&str, &str)]) -> Self {
        let mut grouped: HashMap<String, HashMap<usize, HashMap<Vec<String>, Vec<String>>>> =
            HashMap::new();
        for (spoken, written) in pairs {
            let spoken: Vec<String> = spoken.split_whitespace().map(str::to_string).collect();
            let Some((first, rest)) = spoken.split_first() else {
                continue;
            };
            let written: Vec<String> = written.split_whitespace().map(str::to_string).collect();
            grouped
                .entry(first.clone())
                .or_default()
                .entry(rest.len())
                .or_default()
                .insert(rest.to_vec(), written);
        }
        let index = grouped
            .into_iter()
            .map(|(first, by_length)| {
                let mut by_length: Vec<_> = by_length.into_iter().collect();
                by_length.sort_by(|a, b| b.0.cmp(&a.0));
                (first, by_length)
            })
            .collect();
        Self { index }
    }

    /// The replacement for the longest known phrase starting at `words[start]`, and how many
    /// words it covers.
    pub fn match_at(&self, words: &[Word], start: usize) -> Option<(&[String], usize)> {
        let first = words.get(start)?;
        for (further, phrases) in self.index.get(&first.text)? {
            let end = start + 1 + further;
            if end > words.len() {
                continue;
            }
            // A phrase is said in one go; one broken by punctuation is not this phrase.
            if words[start + 1..end].iter().any(|word| word.after_break) {
                continue;
            }
            let continuation: Vec<String> = words[start + 1..end]
                .iter()
                .map(|word| word.text.clone())
                .collect();
            if let Some(replacement) = phrases.get(&continuation) {
                return Some((replacement.as_slice(), 1 + further));
            }
        }
        None
    }

    /// Talon's `replace`: every known phrase in `words` replaced, longest first. Only used on
    /// command words; see the type's documentation.
    pub fn replace(&self, words: &[Word]) -> Vec<String> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < words.len() {
            if let Some((replacement, consumed)) = self.match_at(words, i) {
                out.extend(replacement.iter().cloned());
                i += consumed;
            } else {
                out.push(words[i].text.clone());
                i += 1;
            }
        }
        out
    }
}

/// Join words back into a phrase.
pub fn join(words: &[Word]) -> String {
    words
        .iter()
        .map(|word| word.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(words: &[Word]) -> Vec<&str> {
        words.iter().map(|word| word.text.as_str()).collect()
    }

    #[test]
    fn words_are_lower_case_without_apostrophes_or_punctuation() {
        let heard = words("Open Claude Code in LocalFlow, and fix what's broken!");
        assert_eq!(
            texts(&heard),
            [
                "open",
                "claude",
                "code",
                "in",
                "localflow",
                "and",
                "fix",
                "whats",
                "broken"
            ]
        );
        let breaks: Vec<bool> = heard.iter().map(|word| word.after_break).collect();
        assert_eq!(
            breaks,
            [false, false, false, false, false, true, false, false, false]
        );
        assert!(words("  ... ").is_empty());
    }

    #[test]
    fn words_remember_where_they_were() {
        let text = "Ask Claude: what's new?";
        let heard = words(text);
        assert_eq!(&text[heard[2].start..heard[2].end], "what's");
        assert_eq!(&text[heard[2].start..], "what's new?");
    }

    #[test]
    fn longest_phrase_wins() {
        let replacer = PhraseReplacer::new(&[
            ("cloud", "cloudy"),
            ("cloud code", "claude code"),
            ("cloud code cli", "claude code"),
        ]);
        assert_eq!(
            replacer.replace(&words("open cloud code cli now")),
            ["open", "claude", "code", "now"]
        );
        assert_eq!(
            replacer.replace(&words("open cloud code now")),
            ["open", "claude", "code", "now"]
        );
        assert_eq!(replacer.replace(&words("a cloud")), ["a", "cloudy"]);
    }

    #[test]
    fn a_phrase_broken_by_punctuation_is_not_replaced() {
        let replacer = PhraseReplacer::new(&[("cloud code", "claude code")]);
        assert_eq!(replacer.match_at(&words("cloud. Code"), 0), None);
    }

    #[test]
    fn match_at_reports_how_many_words_it_used() {
        let replacer = PhraseReplacer::new(&[("clawed code", "claude code")]);
        let heard = words("ask clawed code why");
        let (replacement, used) = replacer.match_at(&heard, 1).unwrap();
        assert_eq!(replacement, ["claude", "code"]);
        assert_eq!(used, 2);
        assert_eq!(replacer.match_at(&heard, 0), None);
        assert_eq!(replacer.match_at(&heard, 9), None);
    }
}
