//! What the assistant remembers of one conversation.
//!
//! Held in memory only and cleared when the conversation ends: nothing here is written to
//! disk or to a log. The history sent to the model is bounded so it always fits the fixed
//! context size, and it records what was actually spoken, so an interrupted reply is
//! remembered only as far as the user heard it.

use crate::llm::chat::{ChatMessage, ChatRole};

/// How the assistant is told to behave. Kept byte-for-byte stable so the model server can
/// reuse its cached evaluation of this prefix on every turn.
pub const SYSTEM_PROMPT: &str = "You are LocalFlow, a voice assistant running entirely on the user's own computer. \
Your words are spoken aloud, so answer the way a person talks: one to three short sentences, plain words, no lists, \
no markdown, no emoji, and no code unless asked. Say numbers and symbols the way they are spoken. \
You cannot browse the internet, open apps, or see the screen; if asked for something you cannot do, say so briefly. \
If the request is unclear, ask one short question. The user's speech is transcribed and may contain small errors.";

/// Characters of history kept. The context is 4,096 tokens; this leaves room for the system
/// prompt and the reply even when text runs to few characters per token.
const HISTORY_BUDGET_CHARS: usize = 8_000;

/// One thing said, and the turn it belongs to.
#[derive(Debug, Clone)]
struct Entry {
    turn: u64,
    role: ChatRole,
    text: String,
}

/// What was said in one conversation, in turn order.
///
/// Every entry carries the turn it belongs to and is kept in turn order however late it is
/// recorded. A reply is recorded when it stops, which for an interrupted one can be after
/// the user's next words have been recorded; it still goes before them. Consecutive user
/// entries become one message only when the messages are read, so whether they run together
/// depends on what was finally heard between them, not on what arrived first.
#[derive(Debug, Default)]
pub struct ConversationMemory {
    entries: Vec<Entry>,
    /// The most recent reply that was spoken, and its turn.
    last_reply: Option<(u64, String)>,
}

impl ConversationMemory {
    /// Record what the user said in `turn`. If the reply before it was never heard (the user
    /// interrupted before any audio), the two run together as one message.
    pub fn push_user(&mut self, turn: u64, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        self.insert(Entry {
            turn,
            role: ChatRole::User,
            text: text.to_string(),
        });
    }

    /// Record `text`, said in `turn`, as the rest of `earlier`: the user carried on speaking
    /// while the reply to `earlier` was being prepared, so that reply was cancelled unheard
    /// and the two are one message.
    ///
    /// The turns of `earlier` may each have reached the history or not, depending on what
    /// they were (only questions are recorded) and on when they were cancelled. Whatever part
    /// of `earlier` the history already ends with is completed rather than repeated.
    pub fn push_user_continuing(&mut self, turn: u64, earlier: &str, text: &str) {
        let earlier = earlier.trim();
        let recorded = self.trailing_user_text();
        let already = recorded_start(&recorded, earlier).map_or(0, |start| recorded.len() - start);
        let missing = earlier[already..].trim();
        let text = text.trim();
        let rest = match (missing.is_empty(), text.is_empty()) {
            (true, _) => text.to_string(),
            (false, true) => missing.to_string(),
            (false, false) => format!("{missing} {text}"),
        };
        self.push_user(turn, &rest);
    }

    /// Record the part of `turn`'s reply that was spoken. Nothing is recorded when nothing
    /// was heard.
    pub fn push_assistant(&mut self, turn: u64, spoken: &str) {
        let spoken = spoken.trim();
        if spoken.is_empty() {
            return;
        }
        if self
            .last_reply
            .as_ref()
            .is_none_or(|(latest, _)| turn >= *latest)
        {
            self.last_reply = Some((turn, spoken.to_string()));
        }
        self.insert(Entry {
            turn,
            role: ChatRole::Assistant,
            text: spoken.to_string(),
        });
    }

    /// The most recent reply that was spoken, for "repeat that".
    pub fn last_reply(&self) -> Option<&str> {
        self.last_reply.as_ref().map(|(_, text)| text.as_str())
    }

    /// The messages to send to the model: the system prompt, then the history, with
    /// consecutive messages of the same speaker run together.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut history: Vec<ChatMessage> = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            match history.last_mut() {
                Some(last) if last.role == entry.role => {
                    last.content.push(' ');
                    last.content.push_str(&entry.text);
                }
                _ => history.push(ChatMessage::new(entry.role, &entry.text)),
            }
        }
        let mut messages = Vec::with_capacity(history.len() + 1);
        messages.push(ChatMessage::new(ChatRole::System, SYSTEM_PROMPT));
        messages.extend(history);
        messages
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.last_reply = None;
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Put `entry` after everything of earlier turns, and a turn's reply after its question.
    fn insert(&mut self, entry: Entry) {
        let rank = |entry: &Entry| (entry.turn, entry.role == ChatRole::Assistant);
        let key = rank(&entry);
        let at = self
            .entries
            .iter()
            .rposition(|existing| rank(existing) <= key)
            .map_or(0, |index| index + 1);
        self.entries.insert(at, entry);
        self.trim();
    }

    /// The user's words since the last reply, as the model would see them.
    fn trailing_user_text(&self) -> String {
        let start = self
            .entries
            .iter()
            .rposition(|entry| entry.role != ChatRole::User)
            .map_or(0, |index| index + 1);
        self.entries[start..]
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Drop the oldest entries until the history fits, always leaving the newest one and
    /// never starting the history with an assistant message.
    fn trim(&mut self) {
        let size = |entries: &[Entry]| {
            entries
                .iter()
                .map(|entry| entry.text.chars().count())
                .sum::<usize>()
        };
        while self.entries.len() > 1 && size(&self.entries) > HISTORY_BUDGET_CHARS {
            self.entries.remove(0);
        }
        while self.entries.len() > 1 && self.entries[0].role == ChatRole::Assistant {
            self.entries.remove(0);
        }
        // One entry longer than the whole budget: keep its end, which is the newest speech.
        if let [only] = self.entries.as_mut_slice() {
            let length = only.text.chars().count();
            if length > HISTORY_BUDGET_CHARS {
                only.text = only
                    .text
                    .chars()
                    .skip(length - HISTORY_BUDGET_CHARS)
                    .collect();
            }
        }
    }
}

/// Where in `recorded` the longest whole-word start of `earlier` begins, if `recorded` ends
/// with one.
fn recorded_start(recorded: &str, earlier: &str) -> Option<usize> {
    // Ends of whole-word starts of `earlier`, longest first.
    let mut ends: Vec<usize> = earlier
        .char_indices()
        .filter(|(_, character)| character.is_whitespace())
        .map(|(index, _)| index)
        .collect();
    ends.push(earlier.len());
    ends.into_iter().rev().find_map(|end| {
        let start_of_earlier = earlier[..end].trim_end();
        if start_of_earlier.is_empty() || !recorded.ends_with(start_of_earlier) {
            return None;
        }
        let start = recorded.len() - start_of_earlier.len();
        // Only at a word boundary: "his" must not match the end of "this".
        let at_boundary = recorded[..start]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        at_boundary.then_some(start)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roles(memory: &ConversationMemory) -> Vec<ChatRole> {
        memory
            .messages()
            .iter()
            .map(|message| message.role)
            .collect()
    }

    fn contents(memory: &ConversationMemory) -> Vec<String> {
        memory.messages()[1..]
            .iter()
            .map(|message| message.content.clone())
            .collect()
    }

    #[test]
    fn history_alternates_after_the_system_prompt() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "What's a mutex?");
        memory.push_assistant(1, "A lock that one thread holds at a time.");
        memory.push_user(2, "And a semaphore?");

        assert_eq!(
            roles(&memory),
            vec![
                ChatRole::System,
                ChatRole::User,
                ChatRole::Assistant,
                ChatRole::User
            ]
        );
        assert_eq!(memory.messages()[0].content, SYSTEM_PROMPT);
        assert_eq!(
            memory.last_reply(),
            Some("A lock that one thread holds at a time.")
        );
    }

    #[test]
    fn an_interrupted_reply_is_remembered_only_as_far_as_it_was_heard() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Tell me about Rust.");
        // The model wrote three sentences; the user interrupted during the first.
        memory.push_assistant(1, "Rust is a systems language.");

        assert_eq!(
            memory.messages().last().unwrap().content,
            "Rust is a systems language."
        );
    }

    #[test]
    fn a_reply_nobody_heard_is_not_remembered_and_the_user_message_continues() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Tell me about");
        memory.push_assistant(1, "   ");
        memory.push_user(3, "Rust, briefly.");

        let messages = memory.messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "Tell me about Rust, briefly.");
        assert_eq!(memory.last_reply(), None);
    }

    #[test]
    fn a_reply_recorded_after_the_next_question_still_comes_before_it() {
        // The interrupted reply's worker was slow to stop: the user's next words were
        // recorded first.
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Tell me a story.");
        memory.push_user(3, "Actually, what time is it?");
        memory.push_assistant(1, "Once upon a time.");

        assert_eq!(
            contents(&memory),
            [
                "Tell me a story.",
                "Once upon a time.",
                "Actually, what time is it?"
            ]
        );
        assert_eq!(
            roles(&memory)[1..],
            [ChatRole::User, ChatRole::Assistant, ChatRole::User]
        );
    }

    #[test]
    fn a_late_reply_never_replaces_a_newer_one_for_repeating() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "One?");
        memory.push_user(3, "Two?");
        memory.push_assistant(3, "Two.");
        memory.push_assistant(1, "One.");

        assert_eq!(memory.last_reply(), Some("Two."));
        assert_eq!(contents(&memory), ["One?", "One.", "Two?", "Two."]);
    }

    #[test]
    fn carrying_on_after_a_question_completes_it_as_one_message() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Hello.");
        memory.push_assistant(1, "Hi there.");
        // The first part was asked; its reply was cancelled before anything was heard.
        memory.push_user(2, "What is");
        memory.push_assistant(2, "");
        memory.push_user_continuing(4, "What is", "the capital of France?");

        let messages = memory.messages();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[3].content, "What is the capital of France?");
    }

    #[test]
    fn carrying_on_after_something_that_was_not_recorded_records_the_whole_message() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Hello.");
        memory.push_assistant(1, "Hi there.");
        // "Read this page" is not a question, so it never reached the history.
        memory.push_user_continuing(3, "Read this page", "from the top");

        let messages = memory.messages();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[3].content, "Read this page from the top");
    }

    #[test]
    fn carrying_on_twice_completes_whatever_part_was_recorded() {
        let mut memory = ConversationMemory::default();
        // Only the first part was recorded before the user carried on twice.
        memory.push_user(1, "Tell me");
        memory.push_user_continuing(5, "Tell me about", "Paris");

        assert_eq!(memory.messages()[1].content, "Tell me about Paris");
        // An unheard message from before is kept in front of it.
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Hmm.");
        memory.push_user(3, "Tell me");
        memory.push_user_continuing(5, "Tell me", "about Paris");
        assert_eq!(memory.messages()[1].content, "Hmm. Tell me about Paris");
        assert_eq!(memory.messages().len(), 2);
    }

    #[test]
    fn a_continuation_only_matches_whole_words() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "Read this");
        // "his" is the end of "this", but not the earlier message.
        memory.push_user_continuing(3, "his", "name");

        assert_eq!(memory.messages()[1].content, "Read this his name");
    }

    #[test]
    fn old_turns_are_dropped_to_stay_inside_the_budget() {
        let mut memory = ConversationMemory::default();
        for index in 0..40 {
            memory.push_user(index, &format!("question {index} {}", "x".repeat(300)));
            memory.push_assistant(index, &format!("answer {index} {}", "y".repeat(300)));
        }
        memory.push_user(40, "the newest question");

        let messages = memory.messages();
        let history: usize = messages[1..]
            .iter()
            .map(|m| m.content.chars().count())
            .sum();
        assert!(history <= HISTORY_BUDGET_CHARS);
        assert_eq!(messages.last().unwrap().content, "the newest question");
        assert_eq!(
            messages[1].role,
            ChatRole::User,
            "history must not open with a reply"
        );
        assert!(!messages
            .iter()
            .any(|m| m.content.starts_with("question 0 ")));
    }

    #[test]
    fn one_enormous_message_keeps_its_most_recent_part() {
        let mut memory = ConversationMemory::default();
        memory.push_user(
            1,
            &format!("{} the end", "a".repeat(HISTORY_BUDGET_CHARS * 2)),
        );

        let content = &memory.messages()[1].content;
        assert_eq!(content.chars().count(), HISTORY_BUDGET_CHARS);
        assert!(content.ends_with("the end"));
    }

    #[test]
    fn ending_the_conversation_forgets_everything() {
        let mut memory = ConversationMemory::default();
        memory.push_user(1, "hello");
        memory.push_assistant(1, "hi");

        memory.clear();

        assert!(memory.is_empty());
        assert_eq!(memory.messages().len(), 1);
        assert_eq!(memory.last_reply(), None);
    }
}
