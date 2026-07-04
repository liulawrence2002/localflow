import { invoke } from "@tauri-apps/api/core";
import { defaultStatus } from "../domain/defaults";
import { runMockDictation } from "../domain/mockPipeline";
import { applyHistoryRetention } from "../domain/privacy";
import { normalizeSettings } from "../domain/settings";
import { transition } from "../domain/stateMachine";
import type { AppStatus, LocalFlowSettings, WorkflowState } from "../domain/types";

const settingsStorageKey = "localflow.settings.v1";

let fallbackStatus: AppStatus = createFallbackStatus();

export async function getStatus(): Promise<AppStatus> {
  return invokeOrFallback("get_status", {}, () => {
    fallbackStatus = createFallbackStatus(fallbackStatus);
    return fallbackStatus;
  });
}

export async function saveSettings(settings: LocalFlowSettings): Promise<AppStatus> {
  return invokeOrFallback("save_settings", { settings }, () => {
    fallbackStatus = { ...fallbackStatus, settings };
    writeFallbackSettings(settings);
    return fallbackStatus;
  });
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
    const sessionId = fallbackStatus.workflow.activeSession?.id ?? "";
    fallbackStatus.workflow = transition(fallbackStatus.workflow, {
      type: "CaptureStarted",
      sessionId,
    });
    return fallbackStatus.workflow;
  });
}

export async function finishMockSession(rawTranscript: string): Promise<AppStatus> {
  return invokeOrFallback("finish_mock_session", { rawTranscript }, () => {
    const result = runMockDictation(fallbackStatus.settings, rawTranscript);
    const nextHistory = result.workflow.lastCompleted
      ? [result.workflow.lastCompleted, ...fallbackStatus.history]
      : fallbackStatus.history;
    fallbackStatus = {
      ...fallbackStatus,
      workflow: result.workflow,
      history: applyHistoryRetention(nextHistory, fallbackStatus.settings.privacy, new Date()),
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

function createFallbackStatus(current?: AppStatus): AppStatus {
  const settings = normalizeSettings(
    readFallbackSettings() ?? current?.settings ?? defaultStatus.settings,
  );

  return {
    ...(current ?? structuredClone(defaultStatus)),
    settings,
  };
}

function readFallbackSettings(): LocalFlowSettings | undefined {
  if (typeof window === "undefined") {
    return undefined;
  }

  try {
    const stored = window.localStorage.getItem(settingsStorageKey);
    return stored ? (JSON.parse(stored) as LocalFlowSettings) : undefined;
  } catch {
    return undefined;
  }
}

function writeFallbackSettings(settings: LocalFlowSettings): void {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.localStorage.setItem(settingsStorageKey, JSON.stringify(settings));
  } catch {
    // Settings remain in memory if browser storage is unavailable.
  }
}
