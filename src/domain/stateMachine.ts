import type {
  ActivationMode,
  DictationSession,
  SessionHistoryItem,
  TargetSnapshot,
  WorkflowState,
} from "./types";

export type WorkflowEvent =
  | {
      type: "BeginActivation";
      sessionId: string;
      mode: ActivationMode;
      target: TargetSnapshot;
      timestamp: string;
    }
  | { type: "CaptureStarted" }
  | { type: "RecordingStopped" }
  | { type: "TranscriptReady"; transcript: string }
  | { type: "DeterministicTextReady"; text: string }
  | { type: "RefinementReady"; text: string; confidence: number }
  | { type: "InsertionStarted" }
  | { type: "Inserted"; timestamp: string }
  | { type: "Cancel"; reason: string }
  | { type: "Fail"; error: string }
  | { type: "Reset" };

const activePhases = new Set(["Preparing", "Listening", "Transcribing", "Refining", "Inserting"]);

export function isActiveSession(state: WorkflowState): boolean {
  return activePhases.has(state.phase);
}

export function transition(state: WorkflowState, event: WorkflowEvent): WorkflowState {
  if (event.type === "Reset") {
    return { phase: "Idle" };
  }

  if (event.type === "Fail") {
    return {
      ...state,
      phase: "Error",
      error: event.error,
      warning: undefined,
    };
  }

  if (event.type === "Cancel") {
    return {
      ...state,
      phase: "Cancelled",
      warning: event.reason,
      activeSession: undefined,
    };
  }

  if (event.type === "BeginActivation") {
    if (isActiveSession(state)) {
      return {
        ...state,
        warning: "A dictation session is already active.",
      };
    }

    const activeSession: DictationSession = {
      id: event.sessionId,
      mode: event.mode,
      startedAt: event.timestamp,
      target: event.target,
    };

    return {
      phase: "Preparing",
      activeSession,
      warning: undefined,
      error: undefined,
      lastCompleted: state.lastCompleted,
    };
  }

  if (!state.activeSession) {
    return {
      ...state,
      warning: `Ignored ${event.type} because no session is active.`,
    };
  }

  switch (event.type) {
    case "CaptureStarted":
      return guardPhase(state, ["Preparing"], { ...state, phase: "Listening" }, event.type);
    case "RecordingStopped":
      return guardPhase(state, ["Listening"], { ...state, phase: "Transcribing" }, event.type);
    case "TranscriptReady":
      return guardPhase(
        state,
        ["Transcribing"],
        {
          ...state,
          phase: "Refining",
          activeSession: { ...state.activeSession, rawTranscript: event.transcript },
        },
        event.type,
      );
    case "DeterministicTextReady":
      return guardPhase(
        state,
        ["Refining"],
        {
          ...state,
          activeSession: { ...state.activeSession, deterministicText: event.text },
        },
        event.type,
      );
    case "RefinementReady":
      return guardPhase(
        state,
        ["Refining"],
        {
          ...state,
          phase: "Inserting",
          activeSession: {
            ...state.activeSession,
            refinedText: event.text,
            confidence: event.confidence,
          },
        },
        event.type,
      );
    case "InsertionStarted":
      return guardPhase(
        state,
        ["Refining", "Inserting"],
        { ...state, phase: "Inserting" },
        event.type,
      );
    case "Inserted": {
      const finalText =
        state.activeSession.refinedText ??
        state.activeSession.deterministicText ??
        state.activeSession.rawTranscript ??
        "";
      const completed: SessionHistoryItem = {
        id: state.activeSession.id,
        completedAt: event.timestamp,
        targetApplication: state.activeSession.target.applicationName,
        rawTranscript: state.activeSession.rawTranscript ?? "",
        finalText,
        cleanupLevel: "balanced",
      };
      return guardPhase(
        state,
        ["Inserting"],
        {
          phase: "Complete",
          activeSession: undefined,
          lastCompleted: completed,
        },
        event.type,
      );
    }
    default:
      return state;
  }
}

function guardPhase(
  current: WorkflowState,
  allowed: WorkflowState["phase"][],
  next: WorkflowState,
  eventType: string,
): WorkflowState {
  if (!allowed.includes(current.phase)) {
    return {
      ...current,
      warning: `Ignored ${eventType} while ${current.phase}.`,
    };
  }

  return {
    ...next,
    warning: undefined,
    error: undefined,
  };
}
