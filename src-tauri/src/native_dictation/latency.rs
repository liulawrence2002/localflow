//! Per-dictation timing snapshots and the diagnostics rows built from them.

use std::{collections::VecDeque, time::Instant};

#[derive(Debug, Clone)]
pub(crate) struct NativeLatencySnapshot {
    pub(crate) session_id: u64,
    pub(crate) ollama_model: String,
    pub(crate) low_resource_mode: bool,
    /// Engine that produced the transcript, for example `whisper-server`.
    pub(crate) asr_provider: String,
    /// File name of the ASR model (never the full path).
    pub(crate) asr_model: String,
    /// Whether the primary engine failed and the local fallback produced the transcript.
    pub(crate) asr_fallback_used: bool,
    pub(crate) audio_duration_ms: u128,
    pub(crate) hotkey_to_recording_start_ms: Option<u128>,
    pub(crate) vad_tail_ms: Option<u128>,
    pub(crate) capture_stop_ms: Option<u128>,
    pub(crate) resample_ms: Option<u128>,
    pub(crate) whisper_ms: Option<u128>,
    pub(crate) deterministic_ms: Option<u128>,
    pub(crate) paste_set_clipboard_ms: Option<u128>,
    pub(crate) paste_send_ms: Option<u128>,
    pub(crate) clipboard_restore_delay_ms: Option<u128>,
    pub(crate) speech_end_to_visible_ms: Option<u128>,
    pub(crate) recording_start_to_insert_ms: Option<u128>,
    pub(crate) ollama_cleanup_ms: Option<u128>,
    pub(crate) background_cleanup_status: BackgroundCleanupStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackgroundCleanupStatus {
    Pending,
    Completed,
    Failed,
    Skipped,
}

impl BackgroundCleanupStatus {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

pub(crate) const MAX_LATENCY_SNAPSHOTS: usize = 5;
pub(crate) const TARGET_VISIBLE_TEXT_MS: u128 = 1_000;
pub(crate) const SLOW_VISIBLE_TEXT_MS: u128 = 2_000;
pub(crate) const WHISPER_BOTTLENECK_MS: u128 = 700;

pub(crate) fn push_latency_snapshot(
    snapshots: &mut VecDeque<NativeLatencySnapshot>,
    snapshot: NativeLatencySnapshot,
) {
    while snapshots.len() >= MAX_LATENCY_SNAPSHOTS {
        snapshots.pop_front();
    }
    snapshots.push_back(snapshot);
}

pub(crate) fn latency_diagnostics_for(
    latest: Option<&NativeLatencySnapshot>,
) -> Vec<crate::app_state::DiagnosticMetric> {
    let Some(snapshot) = latest else {
        return vec![
            diagnostic_metric(
                "Latency: speech end to visible text",
                "Not measured",
                "warning",
            ),
            diagnostic_metric(
                "Latency: recording start to insert",
                "Not measured",
                "warning",
            ),
            diagnostic_metric("Latency: Whisper sidecar", "Not measured", "warning"),
            diagnostic_metric("Latency: Ollama cleanup", "Not measured", "warning"),
        ];
    };

    vec![
        diagnostic_metric(
            "Latency: speech end to visible text",
            format_optional_ms(snapshot.speech_end_to_visible_ms),
            visible_latency_status(snapshot.speech_end_to_visible_ms),
        ),
        diagnostic_metric(
            "Latency: recording start to insert",
            format_optional_ms(snapshot.recording_start_to_insert_ms),
            optional_ms_status(snapshot.recording_start_to_insert_ms),
        ),
        diagnostic_metric(
            "Latency: VAD tail",
            format_optional_ms(snapshot.vad_tail_ms),
            optional_ms_status(snapshot.vad_tail_ms),
        ),
        diagnostic_metric(
            "Latency: capture stop",
            format_optional_ms(snapshot.capture_stop_ms),
            optional_ms_status(snapshot.capture_stop_ms),
        ),
        diagnostic_metric(
            "Latency: resample",
            format_optional_ms(snapshot.resample_ms),
            optional_ms_status(snapshot.resample_ms),
        ),
        diagnostic_metric(
            "ASR engine",
            format!(
                "{} / {}{}",
                snapshot.asr_provider,
                snapshot.asr_model,
                if snapshot.asr_fallback_used {
                    " / fallback engine used"
                } else {
                    ""
                }
            ),
            if snapshot.asr_fallback_used {
                "warning"
            } else {
                "ok"
            },
        ),
        diagnostic_metric(
            "Latency: Whisper sidecar",
            format_optional_ms(snapshot.whisper_ms),
            whisper_latency_status(snapshot.whisper_ms),
        ),
        diagnostic_metric(
            "Latency: deterministic formatting",
            format_optional_ms(snapshot.deterministic_ms),
            optional_ms_status(snapshot.deterministic_ms),
        ),
        diagnostic_metric(
            "Latency: paste send",
            format!(
                "clipboard {} / send {}",
                format_optional_ms(snapshot.paste_set_clipboard_ms),
                format_optional_ms(snapshot.paste_send_ms)
            ),
            paste_latency_status(snapshot.paste_set_clipboard_ms, snapshot.paste_send_ms),
        ),
        diagnostic_metric(
            "Latency: clipboard restore",
            format_optional_ms(snapshot.clipboard_restore_delay_ms),
            optional_ms_status(snapshot.clipboard_restore_delay_ms),
        ),
        diagnostic_metric(
            "Latency: Ollama cleanup",
            format_background_cleanup(snapshot),
            background_cleanup_status(snapshot.background_cleanup_status),
        ),
        diagnostic_metric(
            "Latency: latest audio",
            format!(
                "{} ms audio / model {} / cleanup {}",
                snapshot.audio_duration_ms,
                snapshot.ollama_model,
                snapshot.background_cleanup_status.label()
            ),
            "ok",
        ),
        diagnostic_metric(
            "Latency: hotkey to recording",
            format_optional_ms(snapshot.hotkey_to_recording_start_ms),
            optional_ms_status(snapshot.hotkey_to_recording_start_ms),
        ),
    ]
}

pub(crate) fn diagnostic_metric(
    label: impl Into<String>,
    value: impl Into<String>,
    status: impl Into<String>,
) -> crate::app_state::DiagnosticMetric {
    crate::app_state::DiagnosticMetric {
        label: label.into(),
        value: value.into(),
        status: status.into(),
    }
}

pub(crate) fn format_optional_ms(value: Option<u128>) -> String {
    value
        .map(|value| format!("{value} ms"))
        .unwrap_or_else(|| "Not measured".to_string())
}

pub(crate) fn visible_latency_status(value: Option<u128>) -> &'static str {
    match value {
        Some(value) if value < TARGET_VISIBLE_TEXT_MS => "ok",
        Some(value) if value < SLOW_VISIBLE_TEXT_MS => "warning",
        Some(_) => "error",
        None => "warning",
    }
}

pub(crate) fn whisper_latency_status(value: Option<u128>) -> &'static str {
    match value {
        Some(value) if value <= WHISPER_BOTTLENECK_MS => "ok",
        Some(_) => "warning",
        None => "warning",
    }
}

pub(crate) fn optional_ms_status(value: Option<u128>) -> &'static str {
    if value.is_some() {
        "ok"
    } else {
        "warning"
    }
}

pub(crate) fn paste_latency_status(
    set_clipboard_ms: Option<u128>,
    send_paste_ms: Option<u128>,
) -> &'static str {
    if set_clipboard_ms.is_some() && send_paste_ms.is_some() {
        "ok"
    } else {
        "warning"
    }
}

pub(crate) fn background_cleanup_status(status: BackgroundCleanupStatus) -> &'static str {
    match status {
        BackgroundCleanupStatus::Pending => "warning",
        BackgroundCleanupStatus::Completed | BackgroundCleanupStatus::Skipped => "ok",
        BackgroundCleanupStatus::Failed => "error",
    }
}

pub(crate) fn format_background_cleanup(snapshot: &NativeLatencySnapshot) -> String {
    match snapshot.background_cleanup_status {
        BackgroundCleanupStatus::Pending => format!("Pending on {}", snapshot.ollama_model),
        BackgroundCleanupStatus::Skipped if snapshot.low_resource_mode => {
            "Skipped by low-resource mode".to_string()
        }
        BackgroundCleanupStatus::Skipped => "Skipped".to_string(),
        BackgroundCleanupStatus::Completed => format_optional_ms(snapshot.ollama_cleanup_ms),
        BackgroundCleanupStatus::Failed => format!(
            "Failed after {}",
            format_optional_ms(snapshot.ollama_cleanup_ms)
        ),
    }
}

pub(crate) fn duration_ms_between(start: Instant, end: Instant) -> u128 {
    end.saturating_duration_since(start).as_millis()
}

pub(crate) fn audio_duration_ms(samples: usize, sample_rate: u32) -> u128 {
    if sample_rate == 0 {
        return 0;
    }

    ((samples as f64 / sample_rate as f64) * 1000.0).round() as u128
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn latency_duration_helpers_compute_milliseconds() {
        let started_at = Instant::now();
        let ended_at = started_at + Duration::from_millis(42);

        assert_eq!(duration_ms_between(started_at, ended_at), 42);
        assert_eq!(audio_duration_ms(48_000, 48_000), 1_000);
    }

    #[test]
    fn latency_diagnostics_show_not_measured_before_first_dictation() {
        let rows = latency_diagnostics_for(None);

        let visible = diagnostic_row(&rows, "Latency: speech end to visible text");
        assert_eq!(visible.value, "Not measured");
        assert_eq!(visible.status, "warning");
    }

    #[test]
    fn latency_diagnostics_apply_visible_and_whisper_thresholds() {
        let mut snapshot = latency_snapshot();
        snapshot.speech_end_to_visible_ms = Some(2_100);
        snapshot.whisper_ms = Some(900);

        let rows = latency_diagnostics_for(Some(&snapshot));

        assert_eq!(
            diagnostic_row(&rows, "Latency: speech end to visible text").status,
            "error"
        );
        assert_eq!(
            diagnostic_row(&rows, "Latency: Whisper sidecar").status,
            "warning"
        );
    }

    #[test]
    fn latency_snapshots_keep_only_the_last_five() {
        let mut snapshots = VecDeque::new();

        for session_id in 1..=6 {
            let mut snapshot = latency_snapshot();
            snapshot.session_id = session_id;
            push_latency_snapshot(&mut snapshots, snapshot);
        }

        assert_eq!(snapshots.len(), MAX_LATENCY_SNAPSHOTS);
        assert_eq!(
            snapshots.front().map(|snapshot| snapshot.session_id),
            Some(2)
        );
        assert_eq!(
            snapshots.back().map(|snapshot| snapshot.session_id),
            Some(6)
        );
    }

    #[test]
    fn background_cleanup_failure_does_not_change_visible_latency() {
        let mut snapshot = latency_snapshot();
        snapshot.speech_end_to_visible_ms = Some(850);
        snapshot.ollama_cleanup_ms = Some(2_400);
        snapshot.background_cleanup_status = BackgroundCleanupStatus::Failed;

        let rows = latency_diagnostics_for(Some(&snapshot));

        assert_eq!(
            diagnostic_row(&rows, "Latency: speech end to visible text").value,
            "850 ms"
        );
        assert_eq!(
            diagnostic_row(&rows, "Latency: Ollama cleanup").status,
            "error"
        );
    }

    fn latency_snapshot() -> NativeLatencySnapshot {
        NativeLatencySnapshot {
            session_id: 1,
            ollama_model: "llama3.2:3b".to_string(),
            low_resource_mode: false,
            audio_duration_ms: 1_200,
            hotkey_to_recording_start_ms: Some(30),
            vad_tail_ms: Some(550),
            capture_stop_ms: Some(5),
            asr_provider: "whisper-server".to_string(),
            asr_model: "ggml-base.en-q5_1.bin".to_string(),
            asr_fallback_used: false,
            resample_ms: Some(2),
            whisper_ms: Some(600),
            deterministic_ms: Some(1),
            paste_set_clipboard_ms: Some(2),
            paste_send_ms: Some(4),
            clipboard_restore_delay_ms: Some(400),
            speech_end_to_visible_ms: Some(900),
            recording_start_to_insert_ms: Some(1_500),
            ollama_cleanup_ms: Some(1_100),
            background_cleanup_status: BackgroundCleanupStatus::Completed,
        }
    }

    fn diagnostic_row<'a>(
        rows: &'a [crate::app_state::DiagnosticMetric],
        label: &str,
    ) -> &'a crate::app_state::DiagnosticMetric {
        rows.iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("missing diagnostic row: {label}"))
    }
}
