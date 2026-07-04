pub fn redact_for_log(value: &str) -> String {
    if value.trim().is_empty() {
        return "[empty]".to_string();
    }

    format!("[redacted:{} chars]", value.chars().count())
}

#[cfg(test)]
mod tests {
    use super::redact_for_log;

    #[test]
    fn does_not_log_dictated_content() {
        assert_eq!(redact_for_log("secret dictated content"), "[redacted:23 chars]");
    }
}
