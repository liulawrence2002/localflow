//! Dictation cleanup through the local model: prompt, strict JSON contract, one repair.

use std::time::Duration;

use serde::Deserialize;

use super::ollama::request_ollama_generate;

#[derive(Debug, Deserialize)]
pub(crate) struct CleanupModelResponse {
    pub(crate) text: String,
}

pub(crate) fn refine_with_pinned_ollama(
    raw_transcript: &str,
    deterministic_text: &str,
    model: &str,
) -> Result<String, String> {
    let first_prompt = build_cleanup_prompt(raw_transcript, deterministic_text);
    let first_payload = request_ollama_cleanup(&first_prompt, model)?;

    match parse_cleanup_text(&first_payload) {
        Ok(text) => Ok(text),
        Err(first_error) => {
            let repair_prompt = build_repair_prompt(&first_payload, &first_error);
            let repaired_payload = request_ollama_cleanup(&repair_prompt, model)?;
            parse_cleanup_text(&repaired_payload).map_err(|repair_error| {
                format!(
                    "{model} returned invalid cleanup JSON twice: {first_error}; {repair_error}"
                )
            })
        }
    }
}

pub(crate) fn request_ollama_cleanup(prompt: &str, model: &str) -> Result<String, String> {
    // Bounded so a slow/oversized model falls back to the deterministic transcript quickly
    // instead of freezing the overlay on "refining".
    request_ollama_generate(prompt, Duration::from_secs(20), 0.1, model)
}

pub(crate) fn parse_cleanup_text(payload: &str) -> Result<String, String> {
    let value = parse_json_value(payload)?;
    // The serde message can quote values from the payload, which is model output derived
    // from dictated text, and this error is logged. Report only the kind of mismatch.
    let cleanup: CleanupModelResponse = serde_json::from_value(value).map_err(|error| {
        format!(
            "Cleanup payload did not match the strict contract ({:?} error).",
            error.classify()
        )
    })?;
    let text = cleanup.text.trim();

    if text.is_empty() {
        Err("Cleanup payload returned empty text.".to_string())
    } else {
        Ok(text.to_string())
    }
}

pub(crate) fn parse_json_value(payload: &str) -> Result<serde_json::Value, String> {
    let trimmed = payload.trim();
    serde_json::from_str(trimmed).or_else(|first_error| {
        let start = trimmed.find('{');
        let end = trimmed.rfind('}');

        match (start, end) {
            (Some(start), Some(end)) if start < end => serde_json::from_str(&trimmed[start..=end])
                .map_err(|second_error| {
                    format!("Invalid JSON: {first_error}; extraction failed: {second_error}")
                }),
            _ => Err(format!("Invalid JSON: {first_error}")),
        }
    })
}

pub(crate) fn build_cleanup_prompt(raw_transcript: &str, deterministic_text: &str) -> String {
    serde_json::json!({
        "task": "localflow.dictation_cleanup",
        "contract": "Return only strict JSON with text, confidence, resolved_corrections, and warnings.",
        "rules": [
            "Start from deterministicText: it already has spoken punctuation, self-corrections, and filler cleanup applied. Preserve its wording and formatting unless it is clearly wrong.",
            "Preserve deterministicText capitalization, punctuation, line breaks, and technical casing unless rawTranscript clearly proves they are wrong.",
            "Preserve meaning, facts, names, numbers, uncertainty, and intent.",
            "Never answer the dictated content.",
            "Never add new claims.",
            "Remove filler words only when meaning is unchanged.",
            "Resolve explicit self-corrections in favor of the latest correction.",
            "Keep code identifiers, file paths, URLs, and email addresses verbatim.",
            "Add punctuation and capitalization conservatively.",
            "Return only JSON."
        ],
        "cleanupLevel": "balanced",
        "deterministicText": deterministic_text,
        "rawTranscript": raw_transcript
    })
    .to_string()
}

pub(crate) fn build_repair_prompt(invalid_payload: &str, error: &str) -> String {
    serde_json::json!({
        "task": "localflow.repair_cleanup_json",
        "instruction": "Convert the invalid payload to strict JSON only. Do not change the intended cleaned text.",
        "error": error,
        "invalidPayload": invalid_payload,
        "requiredShape": {
            "text": "final text",
            "confidence": 0.0,
            "resolved_corrections": [],
            "warnings": []
        }
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_contract_errors_do_not_echo_model_output() {
        let error =
            parse_cleanup_text(r#"{"text": 12345, "secret": "dictated words"}"#).unwrap_err();

        assert!(error.contains("strict contract"));
        assert!(!error.contains("12345"));
        assert!(!error.contains("dictated"));
    }

    #[test]
    fn cleanup_prompt_preserves_seeded_casing_and_punctuation_contract() {
        let deterministic_text = "Email Sarah about PyTorch at 3.14 p.m.\n- Ship LocalFlow.";
        let prompt = build_cleanup_prompt(
            "email sarah about pie torch at three point one four p m bullet ship local flow",
            deterministic_text,
        );
        let value: serde_json::Value =
            serde_json::from_str(&prompt).expect("cleanup prompt should be valid JSON");
        let rules = value
            .get("rules")
            .and_then(|rules| rules.as_array())
            .expect("cleanup prompt should include rules");

        assert_eq!(
            value
                .get("deterministicText")
                .and_then(|text| text.as_str()),
            Some(deterministic_text)
        );
        assert!(rules.iter().filter_map(|rule| rule.as_str()).any(|rule| {
            rule.contains("Preserve deterministicText capitalization, punctuation")
                && rule.contains("technical casing")
        }));
    }
}
