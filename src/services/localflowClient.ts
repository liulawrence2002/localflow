import { invoke } from "@tauri-apps/api/core";
import { defaultStatus } from "../domain/defaults";
import { runMockDictation } from "../domain/mockPipeline";
import { transition } from "../domain/stateMachine";
import type { AppStatus, WorkflowState } from "../domain/types";

let fallbackStatus: AppStatus = structuredClone(defaultStatus);

export async function getStatus(): Promise<AppStatus> {
  return invokeOrFallback("get_status", {}, () => fallbackStatus);
}

export async function beginMockSession(): Promise<WorkflowState> {
  return invokeOrFallback("begin_mock_session", {}, () => {
    fallbackStatus.workflow = transition(fallbackStatus.workflow, {
      type: "BeginActivation",
      sessionId: `web-${Date.now()}`,
      mode: fallbackStatus.settings.hotkeys.activationMode,
      target: {
        applicationName: "Browser preview",
        windowTitle: "LocalFlow dev UI",
        category: "generic_text_field",
        protectedField: false,
      },
      timestamp: new Date().toISOString(),
    });
    fallbackStatus.workflow = transition(fallbackStatus.workflow, { type: "CaptureStarted" });
    return fallbackStatus.workflow;
  });
}

export async function finishMockSession(rawTranscript: string): Promise<AppStatus> {
  return invokeOrFallback("finish_mock_session", { rawTranscript }, () => {
    const result = runMockDictation(fallbackStatus.settings, rawTranscript);
    fallbackStatus = {
      ...fallbackStatus,
      workflow: result.workflow,
      history: result.workflow.lastCompleted
        ? [result.workflow.lastCompleted, ...fallbackStatus.history]
        : fallbackStatus.history,
    };
    return fallbackStatus;
  });
}

export async function cancelSession(): Promise<WorkflowState> {
  return invokeOrFallback("cancel_session", {}, () => {
    fallbackStatus.workflow = transition(fallbackStatus.workflow, {
      type: "Cancel",
      reason: "Cancelled from UI.",
    });
    return fallbackStatus.workflow;
  });
}

async function invokeOrFallback<T>(
  command: string,
  args: Record<string, unknown>,
  fallback: () => T,
): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch {
    return fallback();
  }
}
