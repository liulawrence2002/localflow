//! Text helpers shared by every whisper.cpp-backed provider: vocabulary prompt building, JSON
//! output parsing, and detection of whisper's "nothing was said" markers.

/// Build a bounded whisper initial prompt from the user's dictionary terms. Whisper's prompt
/// window is small, so the joined length is capped.
pub fn build_whisper_prompt(terms: &[String]) -> Option<String> {
    const MAX_PROMPT_CHARS: usize = 800;

    let mut joined = String::new();
    for term in terms
        .iter()
        .map(|term| term.trim())
        .filter(|term| !term.is_empty())
    {
        let addition = if joined.is_empty() {
            term.to_string()
        } else {
            format!(", {term}")
        };
        if joined.len() + addition.len() > MAX_PROMPT_CHARS {
            break;
        }
        joined.push_str(&addition);
    }

    if joined.is_empty() {
        None
    } else {
        Some(format!("Glossary: {joined}."))
    }
}

/// Extract transcript text from whisper.cpp JSON. Accepts both the server shape
/// (`{"text": "..."}`) and the CLI `-oj` shape (`{"transcription": [{"text": "..."}]}`).
pub fn parse_whisper_json(payload: &str) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| format!("Could not parse whisper.cpp JSON output: {error}"))?;

    if let Some(message) = value.get("error").and_then(|item| item.as_str()) {
        return Err(format!("whisper.cpp reported an error: {message}"));
    }

    if let Some(text) = value.get("text").and_then(|item| item.as_str()) {
        return Ok(collapse_whitespace(text));
    }

    if let Some(segments) = value.get("transcription").and_then(|item| item.as_array()) {
        let text = segments
            .iter()
            .filter_map(|segment| segment.get("text").and_then(|item| item.as_str()))
            .collect::<Vec<_>>()
            .join(" ");
        return Ok(collapse_whitespace(&text));
    }

    Err("whisper.cpp JSON output did not include transcript text.".to_string())
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `transcript` is empty or one of whisper's non-speech markers such as
/// `[BLANK_AUDIO]` or `(silence)`.
pub fn is_blank_transcript(transcript: &str) -> bool {
    let normalized = transcript
        .trim()
        .trim_matches(|character: char| {
            character.is_whitespace()
                || matches!(
                    character,
                    '[' | ']' | '(' | ')' | '{' | '}' | '.' | '!' | '?'
                )
        })
        .to_ascii_lowercase()
        .replace(['_', '-'], " ");

    matches!(
        normalized.as_str(),
        "" | "blank audio" | "silence" | "no speech" | "inaudible"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_server_json_shape_and_collapses_whitespace() {
        let text = parse_whisper_json("{\"text\":\" Hello  there.\\n\"}").unwrap();
        assert_eq!(text, "Hello there.");
    }

    #[test]
    fn parses_the_cli_json_shape() {
        let payload = r#"{"transcription":[{"text":" Hello"},{"text":" world."}]}"#;
        assert_eq!(parse_whisper_json(payload).unwrap(), "Hello world.");
    }

    #[test]
    fn surfaces_a_server_error_field_instead_of_treating_it_as_text() {
        let error = parse_whisper_json(r#"{"error":"failed to read audio data"}"#).unwrap_err();
        assert!(error.contains("failed to read audio data"));
    }

    #[test]
    fn rejects_json_without_transcript_text() {
        assert!(parse_whisper_json("{}").is_err());
        assert!(parse_whisper_json("not json").is_err());
    }

    #[test]
    fn detects_blank_markers() {
        for marker in [
            "",
            " [BLANK_AUDIO] ",
            "(silence)",
            "[no speech]",
            "[ Inaudible ]",
        ] {
            assert!(is_blank_transcript(marker), "{marker:?}");
        }
        assert!(!is_blank_transcript("Silence is golden."));
    }

    #[test]
    fn prompt_is_bounded_and_skips_empty_terms() {
        assert_eq!(build_whisper_prompt(&[]), None);
        assert_eq!(build_whisper_prompt(&["  ".to_string()]), None);
        assert_eq!(
            build_whisper_prompt(&["Tauri".to_string(), " ".to_string(), "cpal".to_string()]),
            Some("Glossary: Tauri, cpal.".to_string())
        );

        let many: Vec<String> = (0..500).map(|index| format!("term{index}")).collect();
        assert!(build_whisper_prompt(&many).unwrap().len() <= 820);
    }
}
