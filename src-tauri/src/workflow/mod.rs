use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DictationPhase {
    Idle,
    Preparing,
    Listening,
    Transcribing,
    Refining,
    Inserting,
    Complete,
    Cancelled,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActivationMode {
    PushToTalk,
    Toggle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppCategory {
    PersonalMessaging,
    WorkMessaging,
    Email,
    Document,
    CodeEditor,
    Terminal,
    SearchField,
    GenericTextField,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TargetSnapshot {
    pub application_name: String,
    pub window_title: String,
    pub category: AppCategory,
    pub protected_field: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DictationSession {
    pub id: String,
    pub mode: ActivationMode,
    pub started_at: String,
    pub target: TargetSnapshot,
    pub raw_transcript: Option<String>,
    pub deterministic_text: Option<String>,
    pub refined_text: Option<String>,
    pub inserted_text: Option<String>,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowState {
    pub phase: DictationPhase,
    pub active_session: Option<DictationSession>,
    pub last_completed: Option<SessionHistoryItem>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistoryItem {
    pub id: String,
    pub completed_at: String,
    pub target_application: String,
    pub raw_transcript: String,
    pub final_text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkflowEvent {
    BeginActivation {
        session_id: String,
        mode: ActivationMode,
        target: TargetSnapshot,
        timestamp: String,
    },
    CaptureStarted,
    RecordingStopped,
    TranscriptReady {
        transcript: String,
    },
    DeterministicTextReady {
        text: String,
    },
    RefinementReady {
        text: String,
        confidence: f32,
    },
    Inserted {
        timestamp: String,
    },
    Cancel {
        reason: String,
    },
    Fail {
        error: String,
    },
    Reset,
}

impl Default for WorkflowState {
    fn default() -> Self {
        Self {
            phase: DictationPhase::Idle,
            active_session: None,
            last_completed: None,
            warning: None,
            error: None,
        }
    }
}

impl TargetSnapshot {
    pub fn mock() -> Self {
        Self {
            application_name: "Mock target".to_string(),
            window_title: "LocalFlow verification field".to_string(),
            category: AppCategory::GenericTextField,
            protected_field: false,
        }
    }
}

pub fn transition(mut state: WorkflowState, event: WorkflowEvent) -> WorkflowState {
    match event {
        WorkflowEvent::Reset => WorkflowState::default(),
        WorkflowEvent::Fail { error } => {
            state.phase = DictationPhase::Error;
            state.error = Some(error);
            state.warning = None;
            state
        }
        WorkflowEvent::Cancel { reason } => {
            state.phase = DictationPhase::Cancelled;
            state.active_session = None;
            state.warning = Some(reason);
            state
        }
        WorkflowEvent::BeginActivation {
            session_id,
            mode,
            target,
            timestamp,
        } => {
            if is_active(&state.phase) {
                state.warning = Some("A dictation session is already active.".to_string());
                return state;
            }

            state.phase = DictationPhase::Preparing;
            state.active_session = Some(DictationSession {
                id: session_id,
                mode,
                started_at: timestamp,
                target,
                raw_transcript: None,
                deterministic_text: None,
                refined_text: None,
                inserted_text: None,
                confidence: None,
            });
            state.warning = None;
            state.error = None;
            state
        }
        WorkflowEvent::CaptureStarted => guard_phase(state, &[DictationPhase::Preparing], |mut next| {
            next.phase = DictationPhase::Listening;
            next
        }),
        WorkflowEvent::RecordingStopped => guard_phase(state, &[DictationPhase::Listening], |mut next| {
            next.phase = DictationPhase::Transcribing;
            next
        }),
        WorkflowEvent::TranscriptReady { transcript } => {
            guard_phase(state, &[DictationPhase::Transcribing], |mut next| {
                next.phase = DictationPhase::Refining;
                if let Some(session) = next.active_session.as_mut() {
                    session.raw_transcript = Some(transcript);
                }
                next
            })
        }
        WorkflowEvent::DeterministicTextReady { text } => {
            guard_phase(state, &[DictationPhase::Refining], |mut next| {
                if let Some(session) = next.active_session.as_mut() {
                    session.deterministic_text = Some(text);
                }
                next
            })
        }
        WorkflowEvent::RefinementReady { text, confidence } => {
            guard_phase(state, &[DictationPhase::Refining], |mut next| {
                next.phase = DictationPhase::Inserting;
                if let Some(session) = next.active_session.as_mut() {
                    session.refined_text = Some(text);
                    session.confidence = Some(confidence);
                }
                next
            })
        }
        WorkflowEvent::Inserted { timestamp } => {
            guard_phase(state, &[DictationPhase::Inserting], |mut next| {
                if let Some(session) = next.active_session.take() {
                    let final_text = session
                        .refined_text
                        .clone()
                        .or(session.deterministic_text.clone())
                        .or(session.raw_transcript.clone())
                        .unwrap_or_default();
                    next.last_completed = Some(SessionHistoryItem {
                        id: session.id,
                        completed_at: timestamp,
                        target_application: session.target.application_name,
                        raw_transcript: session.raw_transcript.unwrap_or_default(),
                        final_text,
                    });
                }
                next.phase = DictationPhase::Complete;
                next
            })
        }
    }
}

fn is_active(phase: &DictationPhase) -> bool {
    matches!(
        phase,
        DictationPhase::Preparing
            | DictationPhase::Listening
            | DictationPhase::Transcribing
            | DictationPhase::Refining
            | DictationPhase::Inserting
    )
}

fn guard_phase<F>(mut state: WorkflowState, allowed: &[DictationPhase], change: F) -> WorkflowState
where
    F: FnOnce(WorkflowState) -> WorkflowState,
{
    if state.active_session.is_none() {
        state.warning = Some("Ignored event because no session is active.".to_string());
        return state;
    }

    if !allowed.contains(&state.phase) {
        state.warning = Some(format!("Ignored event while {:?}.", state.phase));
        return state;
    }

    let mut next = change(state);
    next.warning = None;
    next.error = None;
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_insert_after_release_path() {
        let mut state = WorkflowState::default();
        state = transition(
            state,
            WorkflowEvent::BeginActivation {
                session_id: "one".to_string(),
                mode: ActivationMode::PushToTalk,
                target: TargetSnapshot::mock(),
                timestamp: "2026-07-04T00:00:00Z".to_string(),
            },
        );
        state = transition(state, WorkflowEvent::CaptureStarted);
        state = transition(state, WorkflowEvent::RecordingStopped);
        state = transition(
            state,
            WorkflowEvent::TranscriptReady {
                transcript: "hello world".to_string(),
            },
        );
        state = transition(
            state,
            WorkflowEvent::RefinementReady {
                text: "Hello world.".to_string(),
                confidence: 0.9,
            },
        );
        state = transition(
            state,
            WorkflowEvent::Inserted {
                timestamp: "2026-07-04T00:00:03Z".to_string(),
            },
        );

        assert_eq!(state.phase, DictationPhase::Complete);
        assert_eq!(state.last_completed.unwrap().final_text, "Hello world.");
    }

    #[test]
    fn rejects_overlapping_sessions() {
        let mut state = WorkflowState::default();
        state = transition(
            state,
            WorkflowEvent::BeginActivation {
                session_id: "one".to_string(),
                mode: ActivationMode::PushToTalk,
                target: TargetSnapshot::mock(),
                timestamp: "2026-07-04T00:00:00Z".to_string(),
            },
        );
        state = transition(
            state,
            WorkflowEvent::BeginActivation {
                session_id: "two".to_string(),
                mode: ActivationMode::Toggle,
                target: TargetSnapshot::mock(),
                timestamp: "2026-07-04T00:00:01Z".to_string(),
            },
        );

        assert_eq!(state.active_session.unwrap().id, "one");
        assert!(state.warning.unwrap().contains("already active"));
    }
}
