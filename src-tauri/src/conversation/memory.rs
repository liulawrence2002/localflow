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

#[derive(Debug, Default)]
pub struct ConversationMemory {
    turns: Vec<ChatMessage>,
    last_reply: Option<String>,
}

impl ConversationMemory {
    /// Record what the user said. If the previous reply was never heard (the user interrupted
    /// before any audio), this continues their earlier message rather than starting a new one.
    pub fn push_user(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        match self.turns.last_mut() {
            Some(last) if last.role == ChatRole::User => {
                last.content.push(' ');
                last.content.push_str(text);
            }
            _ => self.turns.push(ChatMessage::new(ChatRole::User, text)),
        }
        self.trim();
    }

    /// Record the part of a reply that was spoken. Nothing is recorded when nothing was heard.
    pub fn push_assistant(&mut self, spoken: &str) {
        let spoken = spoken.trim();
        if spoken.is_empty() {
            return;
        }
        self.last_reply = Some(spoken.to_string());
        self.turns
            .push(ChatMessage::new(ChatRole::Assistant, spoken));
        self.trim();
    }

    /// The most recent reply that was spoken, for "repeat that".
    pub fn last_reply(&self) -> Option<&str> {
        self.last_reply.as_deref()
    }

    /// The messages to send to the model: the system prompt, then the history.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::with_capacity(self.turns.len() + 1);
        messages.push(ChatMessage::new(ChatRole::System, SYSTEM_PROMPT));
        messages.extend(self.turns.iter().cloned());
        messages
    }

    pub fn clear(&mut self) {
        self.turns.clear();
        self.last_reply = None;
    }

    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// Drop the oldest messages until the history fits, always leaving the newest message
    /// and never starting the history with an assistant message.
    fn trim(&mut self) {
        let size = |turns: &[ChatMessage]| {
            turns
                .iter()
                .map(|turn| turn.content.chars().count())
                .sum::<usize>()
        };
        while self.turns.len() > 1 && size(&self.turns) > HISTORY_BUDGET_CHARS {
            self.turns.remove(0);
        }
        while self.turns.len() > 1 && self.turns[0].role == ChatRole::Assistant {
            self.turns.remove(0);
        }
        // One message longer than the whole budget: keep its end, which is the newest speech.
        if let Some(only) = self.turns.first_mut() {
            let length = only.content.chars().count();
            if self.turns.len() == 1 && length > HISTORY_BUDGET_CHARS {
                let keep: String = self.turns[0]
                    .content
                    .chars()
                    .skip(length - HISTORY_BUDGET_CHARS)
                    .collect();
                self.turns[0].content = keep;
            }
        }
    }
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

    #[test]
    fn history_alternates_after_the_system_prompt() {
        let mut memory = ConversationMemory::default();
        memory.push_user("What's a mutex?");
        memory.push_assistant("A lock that one thread holds at a time.");
        memory.push_user("And a semaphore?");

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
        memory.push_user("Tell me about Rust.");
        // The model wrote three sentences; the user interrupted during the first.
        memory.push_assistant("Rust is a systems language.");

        assert_eq!(
            memory.messages().last().unwrap().content,
            "Rust is a systems language."
        );
    }

    #[test]
    fn a_reply_nobody_heard_is_not_remembered_and_the_user_message_continues() {
        let mut memory = ConversationMemory::default();
        memory.push_user("Tell me about");
        memory.push_assistant("   ");
        memory.push_user("Rust, briefly.");

        let messages = memory.messages();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "Tell me about Rust, briefly.");
        assert_eq!(memory.last_reply(), None);
    }

    #[test]
    fn old_turns_are_dropped_to_stay_inside_the_budget() {
        let mut memory = ConversationMemory::default();
        for index in 0..40 {
            memory.push_user(&format!("question {index} {}", "x".repeat(300)));
            memory.push_assistant(&format!("answer {index} {}", "y".repeat(300)));
        }
        memory.push_user("the newest question");

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
        memory.push_user(&format!("{} the end", "a".repeat(HISTORY_BUDGET_CHARS * 2)));

        let content = &memory.messages()[1].content;
        assert_eq!(content.chars().count(), HISTORY_BUDGET_CHARS);
        assert!(content.ends_with("the end"));
    }

    #[test]
    fn ending_the_conversation_forgets_everything() {
        let mut memory = ConversationMemory::default();
        memory.push_user("hello");
        memory.push_assistant("hi");

        memory.clear();

        assert!(memory.is_empty());
        assert_eq!(memory.messages().len(), 1);
        assert_eq!(memory.last_reply(), None);
    }
}
