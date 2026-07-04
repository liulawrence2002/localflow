//! ASR quality metrics for the benchmark harness (spec §8).
//!
//! Word Error Rate is the standard accuracy measure: the token-level edit distance between a
//! reference transcript and a hypothesis, normalized by reference length. Lower is better;
//! 0.0 is a perfect match. This is a pure, testable building block the benchmark harness uses
//! to compare ASR providers on the documented corpus.
//!
//! Streaming ASR / benchmark foundation (spec Phase 3 / §8): exercised by the unit tests
//! below; not yet wired into a runnable harness, so `dead_code` is allowed.
#![allow(dead_code)]

/// Case-insensitive word error rate between `reference` and `hypothesis`.
/// Returns 0.0 when the reference is empty and the hypothesis is also empty, else 1.0 when
/// only one side is empty.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let reference_tokens = normalize_tokens(reference);
    let hypothesis_tokens = normalize_tokens(hypothesis);

    if reference_tokens.is_empty() {
        return if hypothesis_tokens.is_empty() {
            0.0
        } else {
            1.0
        };
    }

    let distance = token_edit_distance(&reference_tokens, &hypothesis_tokens);
    distance as f64 / reference_tokens.len() as f64
}

fn normalize_tokens(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|token| !token.is_empty())
        .collect()
}

/// Levenshtein distance over token sequences (substitutions/insertions/deletions).
fn token_edit_distance(reference: &[String], hypothesis: &[String]) -> usize {
    let mut previous: Vec<usize> = (0..=hypothesis.len()).collect();
    let mut current = vec![0usize; hypothesis.len() + 1];

    for (i, reference_token) in reference.iter().enumerate() {
        current[0] = i + 1;
        for (j, hypothesis_token) in hypothesis.iter().enumerate() {
            let substitution_cost = if reference_token == hypothesis_token {
                0
            } else {
                1
            };
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + substitution_cost);
        }
        std::mem::swap(&mut previous, &mut current);
    }

    previous[hypothesis.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_transcripts_have_zero_error() {
        assert_eq!(
            word_error_rate("the quick brown fox", "the quick brown fox"),
            0.0
        );
    }

    #[test]
    fn ignores_case_and_punctuation() {
        assert_eq!(word_error_rate("Hello, world.", "hello world"), 0.0);
    }

    #[test]
    fn one_substitution_in_four_words_is_one_quarter() {
        assert_eq!(
            word_error_rate("the quick brown fox", "the quick red fox"),
            0.25
        );
    }

    #[test]
    fn counts_insertions_and_deletions() {
        // One deletion out of three reference words.
        let wer = word_error_rate("one two three", "one three");
        assert!((wer - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn empty_reference_edge_cases() {
        assert_eq!(word_error_rate("", ""), 0.0);
        assert_eq!(word_error_rate("", "unexpected"), 1.0);
    }
}
