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
    /// The user asked for quiet. Said over a reply it stopped the reply (see `barge_in`); as
    /// a turn of its own there is nothing more to do, so just keep listening.
    Quiet,
    /// Say the previous reply again.
    Repeat,
    /// Read what is in the focused window aloud, word for word.
    Read(ReadWhat),
    /// Anything else: answer it.
    Ask(String),
}

/// Which part of the focused window to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadWhat {
    /// Carry on from where reading last stopped, or start with what is on screen.
    FromHere,
    /// Start again from the top of the page.
    FromStart,
    /// Just the next paragraph.
    Next,
    /// Just the paragraph before the last one read.
    Previous,
    /// The last paragraph read, again.
    Again,
}

const READ_HERE_PHRASES: &[&str] = &[
    "read this page",
    "read the page",
    "read this page to me",
    "read the page to me",
    "read this",
    "read this to me",
    "read this aloud",
    "read this out loud",
    "read it to me",
    "read it aloud",
    "read that",
    "read that to me",
    "read this article",
    "read the article",
    "read this window",
    "read the screen",
    "read whats on the screen",
    "read what is on the screen",
    "can you read this page",
    "can you read this page to me",
    "can you read this to me",
    "can you read this",
    "could you read this page",
    "could you read this page to me",
    "start reading",
    "keep reading",
    "continue reading",
    "carry on reading",
    "continue",
    "go on",
    "read on",
];
const READ_START_PHRASES: &[&str] = &[
    "read from the top",
    "read from the start",
    "read from the beginning",
    "read it from the top",
    "read it from the start",
    "read it from the beginning",
    "read this page from the top",
    "read this page from the start",
    "read this page from the beginning",
    "start from the top",
    "start from the beginning",
    "start over",
    "from the top",
];
const READ_NEXT_PHRASES: &[&str] = &[
    "next",
    "next paragraph",
    "the next paragraph",
    "read the next paragraph",
    "read next paragraph",
    "read the next one",
    "next one",
    "read the next part",
    "next part",
    "skip",
];
const READ_PREVIOUS_PHRASES: &[&str] = &[
    "previous",
    "previous paragraph",
    "the previous paragraph",
    "read the previous paragraph",
    "go back",
    "back",
    "go back one",
    "go back a paragraph",
    "read the last paragraph again",
    "the one before",
];
const READ_AGAIN_PHRASES: &[&str] = &[
    "read that again",
    "read it again",
    "read this again",
    "read that paragraph again",
    "again",
    "one more time",
];

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
    "stop that",
    "cancel that",
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
const MAX_COMMAND_WORDS: usize = 8;

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
            for (phrases, what) in [
                (READ_START_PHRASES, ReadWhat::FromStart),
                (READ_NEXT_PHRASES, ReadWhat::Next),
                (READ_PREVIOUS_PHRASES, ReadWhat::Previous),
                (READ_AGAIN_PHRASES, ReadWhat::Again),
                (READ_HERE_PHRASES, ReadWhat::FromHere),
            ] {
                if phrases.contains(&candidate) {
                    return TurnIntent::Read(what);
                }
            }
        }
    }

    TurnIntent::Ask(transcript.trim().to_string())
}

/// Lower case, apostrophes removed, every other non-alphanumeric run turned into one space.
pub(crate) fn normalize(text: &str) -> String {
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
        // As the coding-agent dialog has it ("say stop that to cancel").
        assert_eq!(route_turn("Stop that."), TurnIntent::Quiet);
        assert_eq!(route_turn("Cancel that, please."), TurnIntent::Quiet);
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
    fn reading_requests_are_recognised_with_what_to_read() {
        for (phrase, what) in [
            ("Read this page to me.", ReadWhat::FromHere),
            ("Can you read this page to me, please?", ReadWhat::FromHere),
            ("Keep reading.", ReadWhat::FromHere),
            ("Read from the top", ReadWhat::FromStart),
            ("Read the next paragraph.", ReadWhat::Next),
            ("Next.", ReadWhat::Next),
            ("Go back.", ReadWhat::Previous),
            ("Read that again, please.", ReadWhat::Again),
        ] {
            assert_eq!(route_turn(phrase), TurnIntent::Read(what), "{phrase}");
        }
    }

    #[test]
    fn talking_about_reading_is_still_a_question() {
        for question in [
            "What should I read next?",
            "Can you read me the news from this morning and tell me what you think about it?",
            "How do I go back to the previous commit in git?",
            "Is it worth reading this book again?",
        ] {
            assert!(
                matches!(route_turn(question), TurnIntent::Ask(_)),
                "{question}"
            );
        }
    }

    #[test]
    fn once_the_user_has_the_floor_one_word_is_an_answer() {
        // Over a reply these would be backchannels (see `barge_in`); said in the user's own
        // turn, after the reply has ended, they answer it and go to the model.
        for answer in ["Yeah.", "Okay.", "Sure", "Paris."] {
            assert_eq!(
                route_turn(answer),
                TurnIntent::Ask(answer.to_string()),
                "{answer}"
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
