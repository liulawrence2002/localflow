//! Prepares a block's text for a streaming voice without changing what it says.
//!
//! The voice starts sooner and sounds better when it is given a sentence at a time, so a
//! block is cut into sentence-sized chunks. Cutting is the only thing done to the words:
//! this is text the listener asked to hear exactly as written. The one kind of character
//! removed is the kind a reader never sees and a voice must not act on (control, zero-width,
//! direction and tag characters), which is also how text can be hidden inside a page.

use super::types::{Block, BlockKind};

/// No chunk is longer than this, counted in characters.
pub const MAX_SPEECH_CHUNK_CHARS: usize = 240;

/// Spoken before an image's description so the listener knows it is not part of the prose.
const IMAGE_PREFIX: &str = "Image: ";

/// Abbreviations whose full stop does not end a sentence. Compared in lower case, without
/// the final stop.
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "e.g", "i.e", "no", "approx",
    "inc", "ltd", "co", "fig", "a.m", "p.m", "u.s", "u.k", "vol", "pp", "cf", "al", "gen", "col",
    "lt", "sgt", "rev", "jan", "feb", "aug", "sep", "sept", "oct", "nov", "dec",
];

/// Cut `block`'s text into chunks of at most [`MAX_SPEECH_CHUNK_CHARS`] characters.
///
/// Chunks end at sentence ends where possible; a sentence longer than the limit is cut at a
/// clause mark, failing that at a space, and only a single "word" longer than the limit is
/// cut inside. Apart from the removed invisible characters and the spaces at the cuts, the
/// chunks joined in order are the block's text. An image's first chunk starts with
/// "Image: ".
pub fn speech_chunks(block: &Block) -> Vec<String> {
    let cleaned: String = block
        .text
        .as_str()
        .chars()
        .filter(|character| !is_unspeakable(*character))
        .collect();
    let prefix = if block.kind == BlockKind::Image {
        IMAGE_PREFIX
    } else {
        ""
    };
    // The prefix counts towards the first chunk's length.
    let room = |index: usize| {
        if index == 0 {
            MAX_SPEECH_CHUNK_CHARS - prefix.chars().count()
        } else {
            MAX_SPEECH_CHUNK_CHARS
        }
    };

    let mut chunks: Vec<String> = Vec::new();
    let mut rest = cleaned.trim();
    while !rest.is_empty() {
        let cut = next_cut(rest, room(chunks.len()));
        let (piece, tail) = rest.split_at(cut);
        let piece = piece.trim_end();
        rest = tail.trim_start();

        // A piece with nothing to pronounce ("..." or a stray bracket) rides along with the
        // sentence before it rather than being sent to the voice alone.
        let last = chunks.len().saturating_sub(1);
        match chunks.last_mut() {
            Some(previous)
                if !piece.chars().any(char::is_alphanumeric)
                    && previous.chars().count() + 1 + piece.chars().count() <= room(last) =>
            {
                previous.push(' ');
                previous.push_str(piece);
            }
            _ => chunks.push(piece.to_string()),
        }
    }

    if let Some(first) = chunks.first_mut() {
        first.insert_str(0, prefix);
    }
    chunks
}

/// Characters that are never shown to a reader and must not reach a voice: control
/// characters, zero-width and direction marks, invisible operators, and Unicode tags.
fn is_unspeakable(character: char) -> bool {
    character.is_control()
        || matches!(
            character as u32,
            0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0xFEFF | 0xE0000..=0xE007F
        )
}

/// Byte index at which to cut the next chunk off `text`, which is at most `room` characters
/// long. `text` is not empty and does not start with whitespace.
fn next_cut(text: &str, room: usize) -> usize {
    let sentence = sentence_end(text).unwrap_or(text.len());
    if text[..sentence].chars().count() <= room {
        return sentence;
    }

    let limit = text
        .char_indices()
        .nth(room)
        .map(|(index, _)| index)
        .unwrap_or(text.len());
    let head = &text[..limit];
    if let Some(mark) = last_clause_mark(head, room / 2) {
        return mark;
    }
    // The limit happens to fall between two words.
    if text[limit..].starts_with(char::is_whitespace) {
        return limit;
    }
    head.rfind(char::is_whitespace).unwrap_or(limit)
}

/// Byte index just after the first sentence in `text`, when more text follows it. A sentence
/// ends at `.`, `!` or `?` (plus any closing quotes or brackets) followed by whitespace, so
/// the stop inside `3.14` or `example.com` never ends one.
fn sentence_end(text: &str) -> Option<usize> {
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    let mut index = 0;
    while index < characters.len() {
        let (position, character) = characters[index];
        if !matches!(character, '.' | '!' | '?') {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        while end < characters.len()
            && matches!(
                characters[end].1,
                '.' | '!' | '?' | '"' | '\'' | ')' | ']' | '”' | '’'
            )
        {
            end += 1;
        }
        let followed_by_space = end < characters.len() && characters[end].1.is_whitespace();
        if followed_by_space && !(character == '.' && is_non_terminal_stop(text, position)) {
            return Some(characters[end].0);
        }
        index = end;
    }
    None
}

/// Whether the full stop at byte `stop` belongs to an abbreviation, an initial, or a list
/// number rather than ending a sentence.
fn is_non_terminal_stop(text: &str, stop: usize) -> bool {
    let word_start = text[..stop]
        .rfind(char::is_whitespace)
        .map(|index| index + text[index..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(0);
    let word = text[word_start..stop].trim_start_matches(['(', '"', '\'', '[', '“', '‘']);
    if word.is_empty() {
        return false;
    }
    // "1." opening a list item: a number alone at the very start.
    let list_number = word_start == 0 && word.chars().all(|character| character.is_ascii_digit());
    // A single-letter initial such as "J.", but not the words "I" and "a".
    let initial = word.chars().count() == 1
        && word.chars().all(char::is_alphabetic)
        && word != "I"
        && word != "a";
    list_number || initial || ABBREVIATIONS.contains(&word.to_lowercase().as_str())
}

/// Byte index just after the last `,`, `;`, `:` or dash that is followed by whitespace and
/// has at least `min_chars` characters before it.
fn last_clause_mark(text: &str, min_chars: usize) -> Option<usize> {
    let mut best = None;
    let mut previous: Option<(usize, char)> = None;
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

#[cfg(test)]
mod tests {
    use super::super::types::{BlockId, VerbatimText};
    use super::*;

    fn block(kind: BlockKind, text: &str) -> Block {
        Block {
            id: BlockId(1),
            kind,
            text: VerbatimText::from_page(text),
            in_viewport: true,
        }
    }

    fn chunks(text: &str) -> Vec<String> {
        speech_chunks(&block(BlockKind::Paragraph, text))
    }

    fn tokens(text: &str) -> Vec<String> {
        text.split_whitespace().map(str::to_string).collect()
    }

    fn cleaned(text: &str) -> String {
        VerbatimText::from_page(text)
            .as_str()
            .chars()
            .filter(|character| !is_unspeakable(*character))
            .collect()
    }

    /// A small deterministic generator (xorshift64*), so the property test needs no crate
    /// and fails the same way every time.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound as u64) as usize
        }

        fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
            items[self.below(items.len())]
        }
    }

    const WORDS: &[&str] = &[
        "the",
        "harbour",
        "bridge",
        "Council",
        "I",
        "a",
        "3.14",
        "Dr.",
        "e.g.",
        "J.",
        "etc.",
        "U.S.",
        "1.",
        "42",
        "naïve",
        "Zürich",
        "日本語",
        "über-long-hyphenated-compound",
        "$5.00",
        "example.com",
        "(see",
        "note)",
        "\"quoted\"",
        "“curly”",
        "don't",
        "—",
        "...",
        "?!",
        "wait...",
        "end.)",
        "No.",
        "p.m.",
        "x",
        "antidisestablishmentarianism",
        "😀",
        "a\u{200B}b",
        "\u{202E}reversed",
        "\u{FEFF}",
        "tag\u{E0041}\u{E0042}",
        "bell\u{7}",
        "soft\u{AD}hyphen",
        "\u{2060}",
        "\u{200F}",
    ];
    const ENDINGS: &[&str] = &[
        "", "", "", "", ",", ".", ".", "!", "?", ";", ":", ".\"", "...", " —",
    ];

    fn random_text(random: &mut Random) -> String {
        let count = random.below(120);
        let mut text = String::new();
        for _ in 0..count {
            match random.below(40) {
                // A long unbroken run, still under the limit.
                0 => text.push_str(&"m".repeat(1 + random.below(200))),
                1 => text.push_str("  \t\n "),
                _ => text.push_str(random.pick(WORDS)),
            }
            text.push_str(random.pick(ENDINGS));
            text.push(' ');
        }
        text
    }

    #[test]
    fn chunks_hold_exactly_the_words_of_the_block_in_order() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for round in 0..3_000 {
            let text = random_text(&mut random);
            let kind = match round % 4 {
                0 => BlockKind::Image,
                1 => BlockKind::Heading(2),
                2 => BlockKind::ListItem,
                _ => BlockKind::Paragraph,
            };

            let chunks = speech_chunks(&block(kind, &text));

            let mut spoken = chunks.join(" ");
            if kind == BlockKind::Image && !chunks.is_empty() {
                assert!(spoken.starts_with(IMAGE_PREFIX), "round {round}");
                spoken = spoken[IMAGE_PREFIX.len()..].to_string();
            }
            // Compared by count and position only, so a failure prints no page-like text.
            let expected = tokens(&cleaned(&text));
            let actual = tokens(&spoken);
            assert_eq!(actual.len(), expected.len(), "round {round}: word count");
            assert!(actual == expected, "round {round}: words differ");
            for chunk in &chunks {
                assert!(
                    chunk.chars().count() <= MAX_SPEECH_CHUNK_CHARS,
                    "round {round}"
                );
                assert!(!chunk.trim().is_empty(), "round {round}");
                assert_eq!(chunk.trim(), chunk, "round {round}");
                assert!(!chunk.chars().any(is_unspeakable), "round {round}");
            }
        }
    }

    #[test]
    fn each_sentence_becomes_one_chunk() {
        assert_eq!(
            chunks("The council met on Tuesday. It approved the bridge! Will it open on time? Nobody knows."),
            vec![
                "The council met on Tuesday.",
                "It approved the bridge!",
                "Will it open on time?",
                "Nobody knows."
            ]
        );
    }

    #[test]
    fn decimals_abbreviations_initials_and_list_numbers_do_not_end_a_sentence() {
        assert_eq!(
            chunks("Pi is about 3.14 and Dr. Lee agrees, e.g. in her notes. Done."),
            vec![
                "Pi is about 3.14 and Dr. Lee agrees, e.g. in her notes.",
                "Done."
            ]
        );
        assert_eq!(
            chunks("It was written by J. R. Smith in 1990. It is short."),
            vec!["It was written by J. R. Smith in 1990.", "It is short."]
        );
        assert_eq!(
            chunks("1. Boil the water. Add the pasta."),
            vec!["1. Boil the water.", "Add the pasta."]
        );
        assert_eq!(
            chunks("Visit example.com today."),
            vec!["Visit example.com today."]
        );
    }

    #[test]
    fn closing_quotes_and_brackets_stay_with_their_sentence() {
        assert_eq!(
            chunks("She said \"stop.\" Then she left (quickly.) Nobody followed."),
            vec![
                "She said \"stop.\"",
                "Then she left (quickly.)",
                "Nobody followed."
            ]
        );
        assert_eq!(
            chunks("He asked “why?” And nobody answered."),
            vec!["He asked “why?”", "And nobody answered."]
        );
    }

    #[test]
    fn a_sentence_longer_than_the_limit_is_cut_at_a_clause_mark() {
        let first = "word ".repeat(30);
        let text = format!("{}and so on, {}until the end.", first, "more ".repeat(30));

        let chunks = chunks(&text);

        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].ends_with("and so on,"));
        assert!(chunks[1].starts_with("more more"));
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
    }

    #[test]
    fn a_sentence_without_clause_marks_is_cut_at_a_space() {
        let text = "word ".repeat(120);

        let chunks = chunks(&text);

        assert!(chunks.len() >= 3);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert!(chunks
            .iter()
            .all(|chunk| chunk.split(' ').all(|word| word == "word")));
        assert_eq!(chunks.join(" ").split_whitespace().count(), 120);
    }

    #[test]
    fn a_single_word_longer_than_the_limit_is_cut_inside_and_loses_no_character() {
        let word = "x".repeat(1_000);

        let chunks = chunks(&format!("Before {word} after."));

        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        let letters: String = chunks.concat().split_whitespace().collect();
        assert_eq!(letters, format!("Before{word}after."));

        let alone = super::speech_chunks(&block(BlockKind::Paragraph, &word));
        assert_eq!(alone.len(), 5);
        assert_eq!(alone.concat(), word);
    }

    #[test]
    fn multi_byte_text_is_measured_in_characters_and_never_split_inside_one() {
        let text = "日本語のテキスト".repeat(100);

        let chunks = chunks(&text);

        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert_eq!(chunks.concat(), text);
    }

    #[test]
    fn invisible_and_control_characters_are_removed_and_nothing_else() {
        let hidden = "Read\u{200B} this\u{202E} now\u{2060}.\u{FEFF} Tag\u{E0049}\u{E0067} gone\u{7}. Caf\u{E9} \u{1F600} stays.";

        assert_eq!(
            chunks(hidden),
            vec!["Read this now.", "Tag gone.", "Caf\u{E9} \u{1F600} stays."]
        );
    }

    #[test]
    fn punctuation_symbols_and_markup_characters_are_kept_as_written() {
        let text =
            "Use **bold**, `code`, #tags & <b>angle</b> brackets; cost: $5.00 (approx. 10%).";

        assert_eq!(chunks(text), vec![text]);
    }

    #[test]
    fn an_image_description_is_announced_once() {
        let image = block(
            BlockKind::Image,
            "A bridge at dusk. Lights reflect on the water.",
        );

        assert_eq!(
            speech_chunks(&image),
            vec!["Image: A bridge at dusk.", "Lights reflect on the water."]
        );
    }

    #[test]
    fn the_image_prefix_counts_towards_the_length_limit() {
        let image = block(BlockKind::Image, &"word ".repeat(100));

        let chunks = speech_chunks(&image);

        assert!(chunks[0].starts_with("Image: word"));
        assert!(chunks
            .iter()
            .all(|chunk| chunk.chars().count() <= MAX_SPEECH_CHUNK_CHARS));
        assert_eq!(chunks.join(" ").split_whitespace().count(), 101);
    }

    #[test]
    fn a_piece_with_nothing_to_pronounce_joins_the_sentence_before_it() {
        assert_eq!(chunks("Wait. ... Go on."), vec!["Wait. ...", "Go on."]);
        assert_eq!(chunks("... and then"), vec!["...", "and then"]);
    }

    #[test]
    fn text_with_nothing_left_after_cleaning_gives_no_chunks() {
        assert!(chunks("").is_empty());
        assert!(chunks("\u{200B}\u{FEFF} \u{E0041}").is_empty());
        assert!(speech_chunks(&block(BlockKind::Image, "\u{200B}")).is_empty());
    }

    #[test]
    fn a_short_block_is_one_chunk_exactly_as_written() {
        assert_eq!(
            chunks("Council approves new harbour bridge"),
            vec!["Council approves new harbour bridge"]
        );
        assert_eq!(
            speech_chunks(&block(BlockKind::Heading(1), "What happens next?")),
            vec!["What happens next?"]
        );
    }
}
