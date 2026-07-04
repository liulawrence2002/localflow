import { describe, expect, it } from "vitest";
import { defaultTarget } from "../src/domain/defaults";
import { transition } from "../src/domain/stateMachine";
import type { WorkflowState } from "../src/domain/types";

describe("workflow state machine", () => {
  it("runs the insert-after-release path", () => {
    let state: WorkflowState = { phase: "Idle" };
    const sessionId = "session-1";
    state = transition(state, {
      type: "BeginActivation",
      sessionId,
      mode: "push_to_talk",
      target: defaultTarget,
      timestamp: "2026-07-04T00:00:00.000Z",
    });
    state = transition(state, { type: "CaptureStarted", sessionId });
    state = transition(state, { type: "RecordingStopped", sessionId });
    state = transition(state, {
      type: "TranscriptReady",
      sessionId,
      transcript: "hello comma world",
    });
    state = transition(state, { type: "DeterministicTextReady", sessionId, text: "hello, world" });
    state = transition(state, {
      type: "RefinementReady",
      sessionId,
      text: "Hello, world.",
      confidence: 0.91,
    });
    state = transition(state, {
      type: "Inserted",
      sessionId,
      timestamp: "2026-07-04T00:00:02.000Z",
    });

    expect(state.phase).toBe("Complete");
    expect(state.lastCompleted?.finalText).toBe("Hello, world.");
  });

  it("prevents overlapping sessions", () => {
    let state: WorkflowState = { phase: "Idle" };
    state = transition(state, {
      type: "BeginActivation",
      sessionId: "session-1",
      mode: "push_to_talk",
      target: defaultTarget,
      timestamp: "2026-07-04T00:00:00.000Z",
    });
    state = transition(state, {
      type: "BeginActivation",
      sessionId: "session-2",
      mode: "toggle",
      target: defaultTarget,
      timestamp: "2026-07-04T00:00:01.000Z",
    });

    expect(state.activeSession?.id).toBe("session-1");
    expect(state.warning).toContain("already active");
  });

  it("rejects stale session results", () => {
    let state: WorkflowState = { phase: "Idle" };
    state = transition(state, {
      type: "BeginActivation",
      sessionId: "current",
      mode: "push_to_talk",
      target: defaultTarget,
      timestamp: "2026-07-04T00:00:00.000Z",
    });
    state = transition(state, { type: "CaptureStarted", sessionId: "current" });
    state = transition(state, { type: "RecordingStopped", sessionId: "current" });
    state = transition(state, {
      type: "TranscriptReady",
      sessionId: "previous",
      transcript: "stale transcript",
    });

    expect(state.phase).toBe("Transcribing");
    expect(state.activeSession?.rawTranscript).toBeUndefined();
    expect(state.warning).toContain("stale");
  });

  it("cancels without completing or inserting text", () => {
    let state: WorkflowState = { phase: "Idle" };
    state = transition(state, {
      type: "BeginActivation",
      sessionId: "cancel-me",
      mode: "toggle",
      target: defaultTarget,
      timestamp: "2026-07-04T00:00:00.000Z",
    });
    state = transition(state, { type: "CaptureStarted", sessionId: "cancel-me" });
    state = transition(state, { type: "Cancel", reason: "User cancelled." });

    expect(state.phase).toBe("Cancelled");
    expect(state.activeSession).toBeUndefined();
    expect(state.lastCompleted).toBeUndefined();
  });
});
