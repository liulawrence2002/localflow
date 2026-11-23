//! Decides what a finished user turn is, before any language model sees it.
//!
//! Control phrases ("goodbye", "repeat that") are matched here with fixed rules and never
//! reach the model. That keeps them instant and predictable, and it means nothing the model
//! reads or writes can trigger an action: the model only ever produces words to speak.

/// What to do with one user turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnIntent {
    /// Nothing intelligible was said. Go back to listening without replying.
    Nothing,
    /// End the conversation.
    End,
    /// The user asked for quiet. There is nothing to stop when this is heard (the microphone
    /// is ignored while the assistant speaks), so just keep listening.
    Quiet,
    /// Say the previous reply again.
    Repeat,
    /// Anything else: answer it.
    Ask(String),
}

const END_PHRASES: &[&str] = &[
    "goodbye",
    "good bye",
    "bye",
    "bye bye",
    "end conversation",
    "end the conversation",
    "stop listening",
    "stop the conversation",
    "exit",
    "exit conversation",
    "thats all",
    "that is all",
    "thats all thanks",
    "that will be all",
    "we are done",
    "were done",
];
const QUIET_PHRASES: &[&str] = &[
    "stop",
    "be quiet",
    "quiet",
    "never mind",
    "nevermind",
    "cancel",
    "thats enough",
    "that is enough",
    "ok stop",
    "okay stop",
];
const REPEAT_PHRASES: &[&str] = &[
    "repeat that",
    "repeat",
    "say that again",
    "say it again",
    "can you repeat that",
    "could you repeat that",
    "come again",
    "pardon",
    "what did you say",
];
/// Words dropped from both ends before matching, so "please stop" and "stop, thanks" match.
const POLITE_WORDS: &[&str] = &[
    "please", "thanks", "thank", "you", "ok", "okay", "hey", "um", "uh",
];
/// Control phrases are short. A longer utterance that merely contains one is a question.
const MAX_COMMAND_WORDS: usize = 6;

pub fn route_turn(transcript: &str) -> TurnIntent {
    let normalized = normalize(transcript);
    if normalized.is_empty() {
        return TurnIntent::Nothing;
    }

    let words: Vec<&str> = normalized.split(' ').collect();
    if words.len() <= MAX_COMMAND_WORDS {
        let core = strip_polite(&words).join(" ");
        for candidate in [core.as_str(), normalized.as_str()] {
            if END_PHRASES.contains(&candidate) {
                return TurnIntent::End;
            }
            if QUIET_PHRASES.contains(&candidate) {
                return TurnIntent::Quiet;
            }
            if REPEAT_PHRASES.contains(&candidate) {
                return TurnIntent::Repeat;
            }
        }
    }

    TurnIntent::Ask(transcript.trim().to_string())
}

/// Lower case, apostrophes removed, every other non-alphanumeric run turned into one space.
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        if character.is_alphanumeric() {
            out.extend(character.to_lowercase());
        } else if matches!(character, '\'' | '’') {
            // "that's" -> "thats"
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

fn strip_polite<'a>(words: &[&'a str]) -> Vec<&'a str> {
    let mut start = 0;
    let mut end = words.len();
    while start < end && POLITE_WORDS.contains(&words[start]) {
        start += 1;
    }
    while end > start && POLITE_WORDS.contains(&words[end - 1]) {
        end -= 1;
    }
    words[start..end].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_and_noise_are_nothing() {
        assert_eq!(route_turn(""), TurnIntent::Nothing);
        assert_eq!(route_turn("  ...  "), TurnIntent::Nothing);
    }

    #[test]
    fn ending_phrases_end_the_conversation_however_they_are_punctuated() {
        for phrase in [
            "Goodbye.",
            "goodbye!",
            "OK, that's all, thanks.",
            "End the conversation, please.",
            "Stop listening",
        ] {
            assert_eq!(route_turn(phrase), TurnIntent::End, "{phrase}");
        }
    }

    #[test]
    fn repeat_and_quiet_are_recognised() {
        assert_eq!(route_turn("Could you repeat that?"), TurnIntent::Repeat);
        assert_eq!(route_turn("Say that again, please."), TurnIntent::Repeat);
        assert_eq!(route_turn("Stop."), TurnIntent::Quiet);
        assert_eq!(route_turn("Never mind"), TurnIntent::Quiet);
    }

    #[test]
    fn a_sentence_that_only_contains_a_control_word_is_a_question() {
        for question in [
            "How do I stop a process in PowerShell?",
            "What does goodbye mean in French?",
            "Can you repeat that last number and then add seven to it for me?",
            "When should I say goodbye at a party?",
        ] {
            assert!(
                matches!(route_turn(question), TurnIntent::Ask(_)),
                "{question}"
            );
        }
    }

    #[test]
    fn questions_keep_their_original_wording() {
        assert_eq!(
            route_turn("  What's the capital of France?  "),
            TurnIntent::Ask("What's the capital of France?".to_string())
        );
    }
}
