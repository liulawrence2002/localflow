//! Cuts the language model's streamed reply into pieces the voice can start speaking at once.
//!
//! The delay a listener notices is the wait for the first sound, so the first piece is
//! released early (at a clause mark when no sentence has ended yet) and later pieces are
//! whole sentences, which is where the voice sounds most natural. Pieces are also cleaned
//! of markup a small model tends to emit, because a voice would read `**` and `#` aloud.

/// Abbreviations whose full stop does not end a sentence. Compared in lower case, without
/// the final stop.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "no", "approx",
    "inc", "ltd", "co", "fig", "a.m", "p.m", "u.s", "u.k",
];

/// The first piece may end at a clause mark once it is at least this long.
const FIRST_CLAUSE_MIN_CHARS: usize = 28;
/// A piece that reaches this length without a sentence end is cut at a clause mark or space.
const MAX_PIECE_CHARS: usize = 220;

#[derive(Debug, Default)]
pub struct SentenceChunker {
    pending: String,
    emitted_any: bool,
}

impl SentenceChunker {
    /// Add streamed text and return every piece that is now complete, cleaned for speech.
    /// Pieces with nothing speakable in them are dropped.
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.pending.push_str(delta);
        let mut pieces = Vec::new();

        while let Some(cut) = self.next_cut() {
            let rest = self.pending.split_off(cut);
            let piece = std::mem::replace(&mut self.pending, rest);
            self.pending = self.pending.trim_start().to_string();
            if let Some(clean) = clean_for_speech(&piece) {
                self.emitted_any = true;
                pieces.push(clean);
            }
        }
        pieces
    }

    /// The reply has ended: return whatever is left, if it is speakable.
    pub fn finish(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.pending);
        let clean = clean_for_speech(&rest);
        if clean.is_some() {
            self.emitted_any = true;
        }
        clean
    }

    /// Byte index just after the next piece, if one is complete.
    fn next_cut(&self) -> Option<usize> {
        if let Some(cut) = sentence_end(&self.pending) {
            return Some(cut);
        }

        let chars = self.pending.chars().count();
        if !self.emitted_any && chars >= FIRST_CLAUSE_MIN_CHARS {
            if let Some(cut) = last_clause_mark(&self.pending, FIRST_CLAUSE_MIN_CHARS) {
                return Some(cut);
            }
        }
        if chars >= MAX_PIECE_CHARS {
            // Cut inside the first `MAX_PIECE_CHARS` characters, however much has arrived.
            let limit = self
                .pending
                .char_indices()
                .nth(MAX_PIECE_CHARS)
                .map(|(index, _)| index)
                .unwrap_or(self.pending.len());
            let head = &self.pending[..limit];
            return last_clause_mark(head, MAX_PIECE_CHARS / 2)
                .or_else(|| head.rfind(' ').map(|index| index + 1))
                .or(Some(limit));
        }
        None
    }
}

/// Byte index just after the first sentence in `text`, if a sentence has ended. A sentence
/// ends at `.`, `!` or `?` (plus any closing quotes or brackets) followed by whitespace. The
/// whitespace must already have arrived: a stop at the very end of the text may still be the
/// middle of `3.14` or `e.g.`.
fn sentence_end(text: &str) -> Option<usize> {
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut index = 0;
    while index < bytes.len() {
        let (_, character) = bytes[index];
        if matches!(character, '.' | '!' | '?') {
            let mut end = index + 1;
            while end < bytes.len()
                && matches!(
                    bytes[end].1,
                    '.' | '!' | '?' | '"' | '\'' | ')' | ']' | '”' | '’'
                )
            {
                end += 1;
            }
            let followed_by_space = end < bytes.len() && bytes[end].1.is_whitespace();
            if followed_by_space
                && !(character == '.' && is_non_terminal_stop(text, bytes[index].0))
            {
                return Some(bytes[end].0);
            }
            index = end;
        } else if character == '\n' {
            // A line break ends a piece too: lists and paragraphs from the model.
            if text[..bytes[index].0].chars().any(char::is_alphanumeric) {
                return Some(bytes[index].0 + 1);
            }
            index += 1;
        } else {
            index += 1;
        }
    }
    None
}

/// Whether the full stop at byte `stop` belongs to an abbreviation, an initial, or a list
/// number rather than ending a sentence.
fn is_non_terminal_stop(text: &str, stop: usize) -> bool {
    let word_start = text[..stop]
        .rfind(|character: char| character.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    let word = text[word_start..stop].trim_start_matches(['(', '"', '\'', '[']);
    if word.is_empty() {
        return false;
    }
    let lower = word.to_lowercase();
    // "1." at the start of a list item, or a single-letter initial such as "J.".
    let list_number = word.chars().all(|character| character.is_ascii_digit())
        && word_start == line_start(text, stop);
    let initial = word.chars().count() == 1
        && word.chars().all(char::is_alphabetic)
        && word != "I"
        && word != "a";
    list_number || initial || ABBREVIATIONS.contains(&lower.as_str())
}

fn line_start(text: &str, before: usize) -> usize {
    let line = text[..before]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    line + (text[line..before].len() - text[line..before].trim_start().len())
}

/// Byte index just after the last `,`, `;`, `:` or dash that is followed by a space and
/// leaves at least `min_chars` characters before it.
fn last_clause_mark(text: &str, min_chars: usize) -> Option<usize> {
    let mut best = None;
    let mut previous: Option<(usize, char)> = None;
    // `count` is the number of characters before this one.
    for (count, (index, character)) in text.char_indices().enumerate() {
        if let Some((mark_index, mark)) = previous {
            if matches!(mark, ',' | ';' | ':' | '—' | '–')
                && character.is_whitespace()
                && count >= min_chars
            {
                best = Some(mark_index + mark.len_utf8());
            }
        }
        previous = Some((index, character));
    }
    best
}

/// Make one piece of model output fit to be spoken: drop markup, replace links, collapse
/// whitespace. Returns `None` when nothing speakable is left.
///
/// This is for generated replies only. Text the user asked to hear verbatim must not pass
/// through here, because removing characters could change what it says.
pub fn clean_for_speech(piece: &str) -> Option<String> {
    let piece = replace_markdown_links(piece);
    let mut words: Vec<String> = Vec::new();
    for raw in piece.split_whitespace() {
        // List bullets and heading marks at the start of a line.
        if matches!(raw, "-" | "*" | "•" | "#" | "##" | "###" | "####" | ">") {
            continue;
        }
        let lower = raw.to_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("www.")
        {
            words.push("link".to_string());
            continue;
        }
        let cleaned: String = raw
            .chars()
            .filter(|character| !matches!(character, '*' | '_' | '`' | '#' | '~' | '|' | '<' | '>'))
            .filter(|character| !is_emoji(*character))
            .collect();
        if !cleaned.is_empty() {
            words.push(cleaned);
        }
    }

    let text = words.join(" ");
    text.chars().any(char::is_alphanumeric).then_some(text)
}

/// Replace every markdown link `[label](target)` with its label. A label may contain
/// spaces, so this runs on the whole piece before it is split into words.
fn replace_markdown_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(middle) = rest.find("](") {
        let (Some(open), Some(close)) = (
            rest[..middle].rfind('['),
            rest[middle + 2..].find(')').map(|index| middle + 2 + index),
        ) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..middle]);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn is_emoji(character: char) -> bool {
    matches!(character as u32, 0x1F000..=0x1FAFF | 0x2600..=0x27BF | 0xFE0F | 0x200D)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(parts: &[&str]) -> Vec<String> {
        let mut chunker = SentenceChunker::default();
        let mut pieces = Vec::new();
        for part in parts {
            pieces.extend(chunker.push(part));
        }
        pieces.extend(chunker.finish());
        pieces
    }

    #[test]
    fn sentences_are_released_as_soon_as_the_next_one_starts() {
        let mut chunker = SentenceChunker::default();

        assert!(chunker.push("Sure, I can").is_empty());
        assert!(
            chunker.push(" help.").is_empty(),
            "the stop may still be mid-token"
        );
        assert_eq!(chunker.push(" What do"), vec!["Sure, I can help."]);
        assert_eq!(chunker.push(" you need?"), Vec::<String>::new());
        assert_eq!(chunker.finish().as_deref(), Some("What do you need?"));
    }

    #[test]
    fn decimals_abbreviations_and_initials_do_not_end_a_sentence() {
        assert_eq!(
            stream(&["Pi is about 3.14 and Dr. Lee agrees, e.g. in her notes. Done."]),
            vec![
                "Pi is about 3.14 and Dr. Lee agrees, e.g. in her notes.",
                "Done."
            ]
        );
        assert_eq!(
            stream(&["It was written by J. R. Smith in 1990. It is short."]),
            vec!["It was written by J. R. Smith in 1990.", "It is short."]
        );
    }

    #[test]
    fn tokens_split_anywhere_give_the_same_pieces() {
        let text =
            "The meeting moved to 3.30 p.m. on Thursday. You have an extra hour! Use it well.";
        let whole = stream(&[text]);
        let by_char: Vec<String> = text
            .chars()
            .map(|character| character.to_string())
            .collect();
        let by_char: Vec<&str> = by_char.iter().map(String::as_str).collect();

        assert_eq!(stream(&by_char), whole);
        assert_eq!(whole.len(), 3);
    }

    #[test]
    fn the_first_piece_is_released_at_a_clause_when_no_sentence_has_ended() {
        let mut chunker = SentenceChunker::default();

        let pieces =
            chunker.push("Well, that depends on what you are cooking tonight, because some");

        assert_eq!(
            pieces,
            vec!["Well, that depends on what you are cooking tonight,"]
        );
        // Only the first piece is cut early; later ones wait for the sentence to end.
        assert!(chunker.push(" dishes need an hour, and others").is_empty());
        assert_eq!(
            chunker.push(" need minutes. Ok"),
            vec!["because some dishes need an hour, and others need minutes."]
        );
    }

    #[test]
    fn a_runaway_sentence_is_cut_at_a_sensible_place() {
        let long = "word ".repeat(80);

        let pieces = stream(&[&long]);

        assert!(pieces.len() >= 2);
        assert!(pieces
            .iter()
            .all(|piece| piece.chars().count() <= MAX_PIECE_CHARS));
        assert_eq!(pieces.join(" ").split_whitespace().count(), 80);
    }

    #[test]
    fn markup_links_and_emoji_are_not_spoken() {
        assert_eq!(
            clean_for_speech("**Bold** and `code` and _this_ 😀").as_deref(),
            Some("Bold and code and this")
        );
        assert_eq!(
            clean_for_speech("See https://example.com/a?b=1 or [the docs](https://x.y/z).")
                .as_deref(),
            Some("See link or the docs.")
        );
        assert_eq!(
            clean_for_speech("Read [the user guide](https://x.y/a b) and [this](z).").as_deref(),
            Some("Read the user guide and this.")
        );
        assert_eq!(
            clean_for_speech("An array [1](2 is odd").as_deref(),
            Some("An array [1](2 is odd")
        );
        assert_eq!(clean_for_speech("## Heading").as_deref(), Some("Heading"));
        assert_eq!(clean_for_speech(" *** \n --- "), None);
    }

    #[test]
    fn list_lines_become_separate_pieces_without_their_bullets() {
        assert_eq!(
            stream(&["Two ideas:\n- Roast vegetables\n- Fry an egg\n"]),
            vec!["Two ideas:", "Roast vegetables", "Fry an egg"]
        );
        assert_eq!(
            stream(&["1. Boil water.\n2. Add pasta.\n"]),
            vec!["1. Boil water.", "2. Add pasta."]
        );
    }

    #[test]
    fn an_empty_or_unspeakable_reply_yields_nothing() {
        assert!(stream(&["", "  ", "\n"]).is_empty());
        assert!(stream(&["..."]).is_empty());
    }
}
