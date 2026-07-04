import { defaultTarget } from "./defaults";
import { composeInsertion } from "./insertion";
import { runDeterministicPersonalization } from "./personalization";
import { parseRefinementResponse } from "./refinement";
import { transition } from "./stateMachine";
import type { LocalFlowSettings, WorkflowState } from "./types";

export interface MockPipelineResult {
  workflow: WorkflowState;
  finalText: string;
}

export function runMockDictation(
  settings: LocalFlowSettings,
  rawTranscript: string,
): MockPipelineResult {
  const sessionId = `mock-${Date.now()}`;
  const startedAt = new Date().toISOString();
  const deterministic = runDeterministicPersonalization(
    rawTranscript,
    settings.replacements,
    settings.snippets,
  );

  const refinedPayload = JSON.stringify({
    text: conservativeCleanup(deterministic.text),
    confidence: 0.82,
    resolved_corrections: [],
    warnings: ["Mock provider used; no local LLM was contacted."],
  });
  const refined = parseRefinementResponse(refinedPayload);
  const finalText = refined.ok ? refined.response.text : deterministic.text;
  const inserted = composeInsertion(finalText, {
    beforeCursor: "Status:",
    afterCursor: "",
    atSentenceStart: false,
    codeMode: false,
  });

  let workflow: WorkflowState = { phase: "Idle" };
  workflow = transition(workflow, {
    type: "BeginActivation",
    sessionId,
    mode: settings.hotkeys.activationMode,
    target: defaultTarget,
    timestamp: startedAt,
  });
  workflow = transition(workflow, { type: "CaptureStarted" });
  workflow = transition(workflow, { type: "RecordingStopped" });
  workflow = transition(workflow, { type: "TranscriptReady", transcript: rawTranscript });
  workflow = transition(workflow, { type: "DeterministicTextReady", text: deterministic.text });
  workflow = transition(workflow, {
    type: "RefinementReady",
    text: inserted,
    confidence: refined.ok ? refined.response.confidence : 0,
  });
  workflow = transition(workflow, { type: "Inserted", timestamp: new Date().toISOString() });

  return {
    workflow,
    finalText: inserted,
  };
}

function conservativeCleanup(text: string): string {
  const trimmed = text.trim();
  if (!trimmed) {
    return trimmed;
  }

  const capitalized = trimmed.replace(/^([a-z])/, (match) => match.toUpperCase());
  return /[.!?]$/.test(capitalized) ? capitalized : `${capitalized}.`;
}
