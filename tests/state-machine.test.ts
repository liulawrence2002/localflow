import { describe, expect, it } from "vitest";
import { defaultTarget } from "../src/domain/defaults";
import { transition } from "../src/domain/stateMachine";
import type { WorkflowState } from "../src/domain/types";

describe("workflow state machine", () => {
  it("runs the insert-after-release path", () => {
    let state: WorkflowState = { phase: "Idle" };
    state = transition(state, {
      type: "BeginActivation",
      sessionId: "session-1",
      mode: "push_to_talk",
      target: defaultTarget,
      timestamp: "2026-07-04T00:00:00.000Z",
    });
    state = transition(state, { type: "CaptureStarted" });
    state = transition(state, { type: "RecordingStopped" });
    state = transition(state, { type: "TranscriptReady", transcript: "hello comma world" });
    state = transition(state, { type: "DeterministicTextReady", text: "hello, world" });
    state = transition(state, { type: "RefinementReady", text: "Hello, world.", confidence: 0.91 });
    state = transition(state, { type: "Inserted", timestamp: "2026-07-04T00:00:02.000Z" });

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
});
