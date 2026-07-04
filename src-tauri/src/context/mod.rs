use serde::{Deserialize, Serialize};

use crate::workflow::AppCategory;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextContext {
    pub active_application_name: Option<String>,
    pub window_title: Option<String>,
    pub category: Option<AppCategory>,
    pub selected_text: Option<String>,
    pub before_cursor: Option<String>,
    pub after_cursor: Option<String>,
    pub at_sentence_start: bool,
    pub code_mode: bool,
    pub protected_field: bool,
}

pub trait ContextProvider: Send + Sync {
    fn snapshot(&self) -> Result<TextContext, String>;
}

pub struct EmptyContextProvider;

impl ContextProvider for EmptyContextProvider {
    fn snapshot(&self) -> Result<TextContext, String> {
        Ok(TextContext::default())
    }
}
