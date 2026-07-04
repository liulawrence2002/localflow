pub trait TextInserter: Send + Sync {
    fn insert_text(&self, text: &str) -> Result<String, String>;
}

#[derive(Default)]
pub struct MockTextInserter {
    last_text: std::sync::Mutex<Option<String>>,
}

impl TextInserter for MockTextInserter {
    fn insert_text(&self, text: &str) -> Result<String, String> {
        *self
            .last_text
            .lock()
            .map_err(|_| "Mock insertion lock was poisoned.".to_string())? = Some(text.to_string());
        Ok(text.to_string())
    }
}
