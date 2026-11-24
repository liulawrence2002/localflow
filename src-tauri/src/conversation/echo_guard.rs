//! Tells the assistant's own words, heard back through the microphone, from the user's.
//!
//! Echo cancellation removes most of the assistant's voice from the microphone, but not
//! always all of it. If what is left is taken for the user interrupting, the assistant cuts
//! itself off and then answers its own words. This is the last line of defence: the text of
//! an apparent interruption is compared with what the assistant was saying at the time, and
//! an interruption that is just a stretch of that is ignored.

/// Fewer words than this cannot be judged: "yes" and "stop" occur in almost any reply.
const MIN_WORDS: usize = 3;
/// Share of the heard words that must line up with the spoken words, in order.
const MATCH_SHARE: f32 = 0.7;

/// Whether `heard` is the assistant's own speech coming back, given what it was `speaking`.
///
/// True when at least 70 % of the heard words appear, in the same order and close together,
/// somewhere in what was being spoken. Recognition of an echo is rough (words are dropped
/// and misheard), so an exact match is not required; a real interruption shares few words in
/// order with the reply it interrupts.
pub fn is_echo_of(heard: &str, speaking: &str) -> bool {
    let heard = words(heard);
    let speaking = words(speaking);
    if heard.len() < MIN_WORDS || speaking.len() < MIN_WORDS {
        return false;
    }

    // Try each place in the spoken text where the first few heard words could start, and
    // count how many heard words can be matched in order within a window not much longer
    // than the heard text itself.
    let window = heard.len() * 2;
    let needed = (heard.len() as f32 * MATCH_SHARE).ceil() as usize;
    for start in 0..speaking.len() {
        let end = (start + window).min(speaking.len());
        if end - start < needed {
            break;
        }
        if matched_in_order(&heard, &speaking[start..end]) >= needed {
            return true;
        }
    }
    false
}

/// Whether `heard`, too short for [`is_echo_of`] to judge, is word for word a stretch of what
/// was being `spoken`: the same words, in the same order, side by side.
///
/// Only for deciding whether a few words over a reply should stop it. It cannot tell the
/// user's "a lock" from the assistant's, so it is used where wrongly ignoring the user costs
/// little: the reply carries on and they can say more, or press the hotkey.
pub fn is_fragment_of(heard: &str, speaking: &str) -> bool {
    let heard = words(heard);
    let speaking = words(speaking);
    if heard.is_empty() || heard.len() >= MIN_WORDS {
        return false;
    }
    speaking
        .windows(heard.len())
        .any(|window| window == heard.as_slice())
}

/// How many of `heard`, in order, can be found in `spoken` (longest common subsequence).
fn matched_in_order(heard: &[String], spoken: &[String]) -> usize {
    let mut previous = vec![0usize; spoken.len() + 1];
    let mut current = vec![0usize; spoken.len() + 1];
    for heard_word in heard {
        for (index, spoken_word) in spoken.iter().enumerate() {
            current[index + 1] = if heard_word == spoken_word {
                previous[index] + 1
            } else {
                previous[index + 1].max(current[index])
            };
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[spoken.len()]
}

/// Lower-case words with punctuation removed.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|character| character.is_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str =
        "A mutex is a lock that lets only one thread touch a piece of data at a time, \
        which keeps two threads from changing it at once and leaving it in a broken state.";

    #[test]
    fn a_stretch_of_the_reply_heard_back_is_an_echo() {
        assert!(is_echo_of(
            "lets only one thread touch a piece of data",
            REPLY
        ));
        assert!(is_echo_of(
            "Which keeps two threads from changing it at once.",
            REPLY
        ));
    }

    #[test]
    fn an_echo_is_still_recognised_when_words_are_dropped_or_misheard() {
        // "touch" misheard, "of" dropped.
        assert!(is_echo_of(
            "lets only one thread such a piece data at a time",
            REPLY
        ));
    }

    #[test]
    fn a_real_interruption_is_not_an_echo() {
        for interruption in [
            "Wait, what about deadlocks?",
            "Hold on, can you say that more simply?",
            "No, I meant the other kind of lock.",
            "Okay that is enough, thanks.",
        ] {
            assert!(!is_echo_of(interruption, REPLY), "{interruption}");
        }
    }

    #[test]
    fn the_same_words_in_another_order_are_not_an_echo() {
        assert!(!is_echo_of(
            "data thread lock mutex time piece state broken",
            REPLY
        ));
    }

    #[test]
    fn short_utterances_are_never_treated_as_echo() {
        // "a lock" is in the reply, but so short an utterance could be anyone's.
        assert!(!is_echo_of("a lock", REPLY));
        assert!(!is_echo_of("stop", REPLY));
        assert!(!is_echo_of("", REPLY));
        assert!(!is_echo_of("lets only one thread", ""));
    }

    #[test]
    fn a_word_or_two_of_the_reply_side_by_side_is_a_fragment_of_it() {
        assert!(is_fragment_of("a lock", REPLY));
        assert!(is_fragment_of("Thread.", REPLY));
        // The same words apart, or in another order, are not.
        assert!(!is_fragment_of("mutex thread", REPLY));
        assert!(!is_fragment_of("lock a", REPLY));
        assert!(!is_fragment_of("deadlocks", REPLY));
        // Nothing, and anything long enough for the full check, is not judged here.
        assert!(!is_fragment_of("", REPLY));
        assert!(!is_fragment_of("a lock that", REPLY));
        assert!(!is_fragment_of("a lock", ""));
    }

    #[test]
    fn a_question_that_quotes_the_reply_at_length_is_treated_as_echo() {
        // The known cost of the rule: repeating most of a sentence back is indistinguishable
        // from hearing it back. The user can rephrase or use the hotkey.
        assert!(is_echo_of(
            "only one thread touch a piece of data at a time",
            REPLY
        ));
    }
}
