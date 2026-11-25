//! Spoken forms for project names: how a folder called `my-API_v2` might be said aloud.
//!
//! Ported from Talon community's `core/create_spoken_forms.py`
//! (https://github.com/talonhub/community, MIT licence, Copyright (c) 2021 Jeff Knaus,
//! Ryan Hileman, Zac Roberts, Tony Grosinger, Talon community contributors). What is kept:
//! splitting on symbols and on case and digit boundaries, digit-by-digit, whole-number and
//! year forms for numbers, and spelled forms for acronyms and short words. What differs:
//!
//! - Talon does not split all-lowercase compounds ("localflow"). Here names are compared with
//!   spaces removed ([`match_key`]), so "local flow" matches "localflow" without guessing
//!   where the word breaks are.
//! - Talon resolves two names sharing a spoken form by picking the shortest. Here the form is
//!   marked ambiguous and the user is asked (see `projects::ProjectIndex`).
//! - Talon also generates sub-sequences ("community" for "talon community"). Those are left
//!   out: a shorter form would make more projects collide, and these names launch programs.

/// The regular expression Talon uses, `[A-Z]?[a-z]+|[A-Z]+(?![a-z])|[0-9]+`, written out by
/// hand because the `regex` crate has no look-ahead. Characters outside the three classes
/// separate pieces, which covers `_`, `-`, `.` and spaces. Letters are judged with Unicode
/// case rules, so "Café" is one piece.
pub fn split_pieces(source: &str) -> Vec<String> {
    let chars: Vec<char> = source.chars().collect();
    let mut pieces = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            pieces.push(chars[start..i].iter().collect());
        } else if c.is_lowercase() {
            // `[a-z]+` without a leading capital.
            let start = i;
            while i < chars.len() && chars[i].is_lowercase() {
                i += 1;
            }
            pieces.push(chars[start..i].iter().collect());
        } else if c.is_uppercase() {
            let start = i;
            while i < chars.len() && chars[i].is_uppercase() {
                i += 1;
            }
            let run_end = i;
            if run_end < chars.len() && chars[run_end].is_lowercase() {
                // "APIClient": the last capital starts the next word ("API", "Client"), and a
                // single capital is just the start of a capitalised word ("Local").
                if run_end - start > 1 {
                    pieces.push(chars[start..run_end - 1].iter().collect());
                }
                let word_start = run_end - 1;
                i = run_end;
                while i < chars.len() && chars[i].is_lowercase() {
                    i += 1;
                }
                pieces.push(chars[word_start..i].iter().collect());
            } else {
                pieces.push(chars[start..run_end].iter().collect());
            }
        } else {
            // A separator or a symbol; letters without case (CJK) are kept as their own run.
            if c.is_alphabetic() {
                let start = i;
                while i < chars.len()
                    && chars[i].is_alphabetic()
                    && !chars[i].is_lowercase()
                    && !chars[i].is_uppercase()
                {
                    i += 1;
                }
                pieces.push(chars[start..i].iter().collect());
            } else {
                i += 1;
            }
        }
    }
    pieces
}

/// Talon's `create_spoken_form_for_number`: "123" as "one hundred twenty three".
pub fn number_in_words(mut n: u64) -> String {
    const ONES: [&str; 20] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
    ];
    const TENS: [&str; 10] = [
        "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
    ];
    const SCALES: [&str; 7] = [
        "",
        "thousand",
        "million",
        "billion",
        "trillion",
        "quadrillion",
        "quintillion",
    ];

    fn below_thousand(n: u64, words: &mut Vec<&'static str>) {
        let hundreds = n / 100;
        let rest = n % 100;
        if hundreds > 0 {
            words.push(ONES[hundreds as usize]);
            words.push("hundred");
        }
        if rest >= 20 {
            words.push(TENS[(rest / 10) as usize]);
            if !rest.is_multiple_of(10) {
                words.push(ONES[(rest % 10) as usize]);
            }
        } else if rest > 0 {
            words.push(ONES[rest as usize]);
        }
    }

    if n == 0 {
        return "zero".to_string();
    }
    let mut groups = Vec::new();
    while n > 0 {
        groups.push(n % 1000);
        n /= 1000;
    }
    let mut words = Vec::new();
    for (scale, group) in groups.iter().enumerate().rev() {
        if *group == 0 {
            continue;
        }
        below_thousand(*group, &mut words);
        if scale > 0 {
            words.push(SCALES[scale]);
        }
    }
    words.join(" ")
}

/// Talon's `create_spoken_form_years`: "1999" as "nineteen ninety nine", "2005" as
/// "two thousand five", "1906" as "nineteen oh six". `None` outside 1000-9999.
pub fn year_in_words(n: u64) -> Option<String> {
    if !(1000..=9999).contains(&n) {
        return None;
    }
    let centuries = n / 100;
    let remainder = n % 100;
    let mut words = Vec::new();
    if !centuries.is_multiple_of(10) {
        words.push(number_in_words(centuries));
        if remainder == 0 {
            words.push("hundred".to_string());
        }
    } else if remainder < 9 {
        // 200X -> "two thousand X"
        words.push(number_in_words(centuries / 10));
        words.push("thousand".to_string());
    } else {
        // 20XX -> "twenty XX"
        words.push(number_in_words(centuries));
    }
    if remainder != 0 {
        if remainder < 10 && !centuries.is_multiple_of(10) {
            words.push("oh".to_string());
        }
        words.push(number_in_words(remainder));
    }
    Some(words.join(" "))
}

fn digits_in_words(digits: &str) -> String {
    digits
        .chars()
        .filter_map(|d| d.to_digit(10))
        .map(|d| number_in_words(u64::from(d)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn spelled(word: &str) -> String {
    word.chars()
        .flat_map(char::to_lowercase)
        .map(String::from)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Talon's `EXPLODE_MAX_LEN`: lower-case words up to this length are also spelled out.
const SPELL_MAX_LEN: usize = 3;
/// Whole-number forms are only made for numbers that fit comfortably in a `u64`.
const MAX_NUMBER_DIGITS: usize = 15;

/// The ways one piece of a name can be said.
struct PieceForms {
    /// Lower case, or a number as a whole number ("twelve").
    natural: String,
    /// "a p i" for an acronym, and for a word of up to three letters.
    spelled: Option<String>,
    is_acronym: bool,
    /// "one two" for "12".
    digit_wise: Option<String>,
    /// "twenty twenty four" for "2024".
    year: Option<String>,
}

fn piece_forms(piece: &str) -> PieceForms {
    if piece.chars().all(|c| c.is_ascii_digit()) {
        let value = if piece.len() <= MAX_NUMBER_DIGITS {
            piece.parse::<u64>().ok()
        } else {
            None
        };
        let digit_wise = digits_in_words(piece);
        return PieceForms {
            natural: value
                .map(number_in_words)
                .unwrap_or_else(|| digit_wise.clone()),
            spelled: None,
            is_acronym: false,
            digit_wise: Some(digit_wise),
            year: value.and_then(year_in_words),
        };
    }
    let letters = piece.chars().count();
    let is_acronym = letters > 1
        && piece.chars().all(|c| !c.is_lowercase())
        && piece.chars().any(char::is_uppercase);
    PieceForms {
        natural: piece.to_lowercase(),
        spelled: (letters > 1 && (is_acronym || letters <= SPELL_MAX_LEN)).then(|| spelled(piece)),
        is_acronym,
        digit_wise: None,
        year: None,
    }
}

/// Every spoken form of `source`, most natural first, without duplicates. As in Talon, each
/// rule makes one variant of the whole name rather than every combination:
///
/// 1. split at case, digit and symbol boundaries ("LocalFlow" -> "local flow");
/// 2. split at symbols only ("localflow");
/// 3. acronyms spelled ("my a p i");
/// 4. acronyms and short words spelled ("a p p");
/// 5. numbers digit by digit ("project one two");
/// 6. numbers as years ("site twenty twenty four").
pub fn create_spoken_forms(source: &str) -> Vec<String> {
    let pieces: Vec<PieceForms> = split_pieces(source)
        .iter()
        .map(|piece| piece_forms(piece))
        .collect();
    if pieces.is_empty() {
        return Vec::new();
    }
    let join = |pick: &dyn Fn(&PieceForms) -> String| {
        pieces.iter().map(pick).collect::<Vec<_>>().join(" ")
    };

    let symbol_split = source
        .split(|c: char| !c.is_alphanumeric())
        .filter(|chunk| !chunk.is_empty())
        .map(|chunk| {
            let chunk_pieces = split_pieces(chunk);
            if chunk_pieces
                .iter()
                .any(|piece| piece.chars().all(|c| c.is_ascii_digit()))
            {
                // Keep a number apart from the letters around it so that it is said.
                chunk_pieces
                    .iter()
                    .map(|piece| piece_forms(piece).natural)
                    .collect::<Vec<_>>()
                    .join(" ")
            } else {
                chunk.to_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let candidates = [
        join(&|p| p.natural.clone()),
        symbol_split,
        join(&|p| match (&p.spelled, p.is_acronym) {
            (Some(spelled), true) => spelled.clone(),
            _ => p.natural.clone(),
        }),
        join(&|p| p.spelled.clone().unwrap_or_else(|| p.natural.clone())),
        join(&|p| p.digit_wise.clone().unwrap_or_else(|| p.natural.clone())),
        join(&|p| p.year.clone().unwrap_or_else(|| p.natural.clone())),
    ];

    let mut forms: Vec<String> = Vec::new();
    for form in candidates {
        let form = form.split_whitespace().collect::<Vec<_>>().join(" ");
        if !form.is_empty() && !forms.contains(&form) {
            forms.push(form);
        }
    }
    forms
}

/// A few spoken names to offer for a folder, for the user to keep or edit. Names are
/// compared with spaces removed, so of "local flow" and "localflow" only the first is
/// offered: the second would match exactly the same things.
pub fn suggest_spoken_names(folder_name: &str) -> Vec<String> {
    const SUGGESTIONS: usize = 3;
    let mut keys = Vec::new();
    let mut names = Vec::new();
    for form in create_spoken_forms(folder_name) {
        let key = match_key(&form);
        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
            names.push(form);
        }
        if names.len() == SUGGESTIONS {
            break;
        }
    }
    names
}

/// What a heard phrase or a spoken name is compared by: lower case, digits said as a whole
/// number, and all spaces and punctuation removed. "Local Flow", "localflow" and
/// "local-flow" all give `localflow`; "v2" and "v two" both give `vtwo`.
pub fn match_key(text: &str) -> String {
    let mut key = String::with_capacity(text.len());
    let mut digits = String::new();
    let flush = |digits: &mut String, key: &mut String| {
        if digits.is_empty() {
            return;
        }
        match digits.parse::<u64>() {
            Ok(value) if digits.len() <= MAX_NUMBER_DIGITS => {
                key.extend(number_in_words(value).split(' '));
            }
            _ => key.extend(digits_in_words(digits).split(' ')),
        }
        digits.clear();
    };
    for c in text.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else {
            flush(&mut digits, &mut key);
            if c.is_alphanumeric() {
                key.extend(c.to_lowercase());
            }
        }
    }
    flush(&mut digits, &mut key);
    key
}

/// Every key a spoken name is matched by: the name itself, and the name with each number in
/// it said digit by digit or as a year, so "app 12" matches "app twelve" and "app one two".
pub fn match_keys(spoken_name: &str) -> Vec<String> {
    let mut keys = vec![match_key(spoken_name)];
    for form in create_spoken_forms(spoken_name) {
        let key = match_key(&form);
        if !key.is_empty() && !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.retain(|key| !key.is_empty());
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_talon_expression() {
        assert_eq!(split_pieces("LocalFlow"), ["Local", "Flow"]);
        assert_eq!(split_pieces("APIClient"), ["API", "Client"]);
        assert_eq!(split_pieces("myHTTPServer2"), ["my", "HTTP", "Server", "2"]);
        assert_eq!(split_pieces("my-app_v2.0"), ["my", "app", "v", "2", "0"]);
        assert_eq!(split_pieces("localflow"), ["localflow"]);
        assert_eq!(split_pieces("iOS"), ["i", "OS"]);
        assert_eq!(split_pieces("ABC"), ["ABC"]);
        assert_eq!(split_pieces("  -- "), Vec::<String>::new());
    }

    #[test]
    fn numbers_have_whole_digit_and_year_forms() {
        assert_eq!(number_in_words(0), "zero");
        assert_eq!(number_in_words(7), "seven");
        assert_eq!(number_in_words(42), "forty two");
        assert_eq!(number_in_words(123), "one hundred twenty three");
        assert_eq!(number_in_words(2024), "two thousand twenty four");
        assert_eq!(number_in_words(1_000_001), "one million one");
        assert_eq!(year_in_words(1999).as_deref(), Some("nineteen ninety nine"));
        assert_eq!(year_in_words(1900).as_deref(), Some("nineteen hundred"));
        assert_eq!(year_in_words(1906).as_deref(), Some("nineteen oh six"));
        assert_eq!(year_in_words(2005).as_deref(), Some("two thousand five"));
        assert_eq!(year_in_words(2024).as_deref(), Some("twenty twenty four"));
        assert_eq!(year_in_words(999), None);
        assert_eq!(digits_in_words("12"), "one two");
    }

    #[test]
    fn camel_case_names_get_split_and_joined_forms() {
        let forms = create_spoken_forms("LocalFlow");
        assert_eq!(forms[0], "local flow");
        assert!(forms.contains(&"localflow".to_string()), "{forms:?}");
    }

    #[test]
    fn lowercase_compounds_are_not_split() {
        assert_eq!(create_spoken_forms("localflow"), ["localflow"]);
    }

    #[test]
    fn acronyms_get_lowercase_and_spelled_forms() {
        let forms = create_spoken_forms("API");
        assert_eq!(forms, ["api", "a p i"]);
        let forms = create_spoken_forms("my-API");
        assert_eq!(forms, ["my api", "my a p i", "m y a p i"]);
    }

    #[test]
    fn short_lowercase_words_are_also_spelled() {
        let forms = create_spoken_forms("app");
        assert_eq!(forms, ["app", "a p p"]);
        // Longer words are not.
        assert_eq!(create_spoken_forms("server"), ["server"]);
    }

    #[test]
    fn numbers_in_names_get_every_form() {
        let forms = create_spoken_forms("project12");
        assert_eq!(forms[0], "project twelve");
        assert!(forms.contains(&"project one two".to_string()), "{forms:?}");

        let forms = create_spoken_forms("site2024");
        assert!(forms.contains(&"site two thousand twenty four".to_string()));
        assert!(forms.contains(&"site twenty twenty four".to_string()));
        assert!(forms.contains(&"site two zero two four".to_string()));
    }

    #[test]
    fn suggestions_are_few_most_natural_first_and_match_differently() {
        assert_eq!(suggest_spoken_names("my-API_v2"), ["my api v two"]);
        assert_eq!(suggest_spoken_names("LocalFlow"), ["local flow"]);
        assert_eq!(suggest_spoken_names("localflow"), ["localflow"]);
        assert_eq!(
            suggest_spoken_names("project12"),
            ["project twelve", "project one two"]
        );
        assert!(suggest_spoken_names("...").is_empty());
    }

    #[test]
    fn match_keys_ignore_spacing_case_and_punctuation() {
        assert_eq!(match_key("Local Flow"), "localflow");
        assert_eq!(match_key("local-flow."), "localflow");
        assert_eq!(match_key("localflow"), "localflow");
        assert_eq!(match_key("v2"), "vtwo");
        assert_eq!(match_key("v two"), "vtwo");
        assert_eq!(match_key("A P I"), "api");
    }

    #[test]
    fn a_name_with_a_number_matches_every_way_of_saying_it() {
        let keys = match_keys("app 12");
        for heard in ["app twelve", "app 12", "app one two", "App-12"] {
            assert!(keys.contains(&match_key(heard)), "{heard}: {keys:?}");
        }
        let keys = match_keys("site 2024");
        assert!(keys.contains(&match_key("site twenty twenty four")));
    }

    #[test]
    fn many_numbers_do_not_explode_the_form_count() {
        let forms = create_spoken_forms("a1b2c3d4e5f6g7h8i9j10k11l12");
        assert!(forms.len() <= 6, "{forms:?}");
    }
}
