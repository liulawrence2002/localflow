import { invoke } from "@tauri-apps/api/core";
import {
  quickFolderProblem,
  type CodingAgentsStatus,
  type CodingProjectInput,
  type CodingProjectView,
  type SpokenNameSuggestion,
} from "../domain/codingAgents";
import {
  localModelsToStatus,
  type ConversationStatus,
  type ScreenReadingStatus,
} from "../domain/conversation";
import { defaultStatus } from "../domain/defaults";
import { runMockDictation } from "../domain/mockPipeline";
import {
  isRecord,
  parseAppStatus,
  parseCodingAgents,
  parseCodingProjects,
  parseConversationStatus,
  parseOptionalText,
  parseScreenReadingStatus,
  parseSpokenNameSuggestion,
  parseStringList,
  parseTtsStatus,
  type TtsStatus,
} from "../domain/nativePayloads";
import {
  checkOllamaStatus,
  defaultOllamaApiBaseUrl,
  type OllamaConnectionStatus,
} from "../domain/ollama";
import { applyHistoryRetention } from "../domain/privacy";
import { parseReadiness, type ReadinessItem } from "../domain/readiness";
import { clampForNative, normalizeSettings } from "../domain/settings";
import { transition } from "../domain/stateMachine";
import type { AppStatus, LocalFlowSettings, WorkflowState } from "../domain/types";

export type { TtsStatus } from "../domain/nativePayloads";

/**
 * The Tauri command adapter.
 *
 * In the desktop app every command goes to the native layer, and a failure is thrown as an
 * `Error` carrying the native message: nothing falls back to browser data there, so a failed
 * save is never shown as saved. The browser fallbacks exist only for frontend development
 * with `npm run dev`, outside the desktop app.
 */

const settingsStorageKey = "localflow.settings.v1";

/** A native command that failed; `message` is the native layer's own sentence. */
export class NativeCommandError extends Error {
  readonly command: string;

  constructor(command: string, message: string) {
    super(message);
    this.name = "NativeCommandError";
    this.command = command;
  }
}

/** Whether this page runs inside the Tauri desktop app (rather than a plain browser). */
export function isDesktopApp(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** The message of anything a command or the UI threw, for showing to the user. */
export function errorMessage(reason: unknown, fallback: string): string {
  if (reason instanceof Error && reason.message.trim()) {
    return reason.message;
  }
  if (typeof reason === "string" && reason.trim()) {
    return reason;
  }
  return fallback;
}

function toError(command: string, reason: unknown): Error {
  if (reason instanceof Error) {
    return reason;
  }
  if (typeof reason === "string" && reason.trim()) {
    return new NativeCommandError(command, reason);
  }
  if (isRecord(reason) && typeof reason.message === "string" && reason.message.trim()) {
    return new NativeCommandError(command, reason.message);
  }
  return new NativeCommandError(command, `LocalFlow could not complete ${command}.`);
}

async function callNative<T>(
  command: string,
  args: Record<string, unknown>,
  parse: (reply: unknown) => T,
): Promise<T> {
  let reply: unknown;
  try {
    reply = await invoke<unknown>(command, args);
  } catch (reason) {
    throw toError(command, reason);
  }
  return parse(reply);
}

/** The native command in the desktop app; `browserFallback` only outside it. */
async function desktopOrBrowser<T>(
  command: string,
  args: Record<string, unknown>,
  parse: (reply: unknown) => T,
  browserFallback: () => T | Promise<T>,
): Promise<T> {
  if (!isDesktopApp()) {
    return browserFallback();
  }
  return callNative(command, args, parse);
}

const ignoreReply = (): void => undefined;

let fallbackStatus: AppStatus = createFallbackStatus();

export async function getStatus(): Promise<AppStatus> {
  return desktopOrBrowser("get_status", {}, parseAppStatus, () => {
    fallbackStatus = createFallbackStatus(fallbackStatus);
    return fallbackStatus;
  });
}

/** Save settings. Rejects with the native reason when they could not be stored. */
export async function saveSettings(settings: LocalFlowSettings): Promise<AppStatus> {
  const storable = clampForNative(settings);
  return desktopOrBrowser(
    "save_settings",
    { settings: storable },
    (reply) => parseAppStatus(reply, "save_settings"),
    () => {
      fallbackStatus = { ...fallbackStatus, settings: storable };
      writeFallbackSettings(storable);
      return fallbackStatus;
    },
  );
}

export async function beginMockSession(): Promise<WorkflowState> {
  return desktopOrBrowser(
    "begin_mock_session",
    {},
    (reply) => reply as WorkflowState,
    () => {
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
    },
  );
}

export async function finishMockSession(rawTranscript: string): Promise<AppStatus> {
  return desktopOrBrowser(
    "finish_mock_session",
    { rawTranscript },
    (reply) => parseAppStatus(reply, "finish_mock_session"),
    () => {
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
    },
  );
}

export async function cancelSession(): Promise<WorkflowState> {
  return desktopOrBrowser(
    "cancel_session",
    {},
    (reply) => reply as WorkflowState,
    () => {
      fallbackStatus.workflow = transition(fallbackStatus.workflow, {
        type: "Cancel",
        reason: "Cancelled from UI.",
      });
      return fallbackStatus.workflow;
    },
  );
}

export async function getLastTranscript(): Promise<string | null> {
  return desktopOrBrowser(
    "get_last_transcript",
    {},
    (reply) => parseOptionalText(reply, "get_last_transcript"),
    () => null,
  );
}

/** Put the last transcript on the clipboard. Rejects when there is none or it failed. */
export async function copyLastTranscript(): Promise<void> {
  return desktopOrBrowser("copy_last_transcript", {}, ignoreReply, () => {
    throw new Error("Copying the last transcript needs the desktop app.");
  });
}

/** Whether the optional Fish S2 read-aloud voice is installed, and what is missing if not. */
export async function getTtsStatus(): Promise<TtsStatus> {
  return desktopOrBrowser("tts_status", {}, parseTtsStatus, () => ({
    available: false,
    detail: "Reading aloud needs the desktop app.",
    speaking: false,
  }));
}

/** Read the last transcript aloud. Rejects with the reason it cannot. */
export async function speakLastTranscript(): Promise<void> {
  return desktopOrBrowser("speak_last_transcript", {}, ignoreReply, () => {
    throw new Error("Reading aloud needs the desktop app.");
  });
}

export async function stopSpeaking(): Promise<void> {
  return desktopOrBrowser("stop_speaking", {}, ignoreReply, ignoreReply);
}

/** Conversation mode status, or `null` outside the desktop app. */
export async function getConversationStatus(): Promise<ConversationStatus | null> {
  return desktopOrBrowser("conversation_status", {}, parseConversationStatus, () => null);
}

/** Whether windows can be read aloud and with which hotkey, or `null` outside the desktop app. */
export async function getScreenReadingStatus(): Promise<ScreenReadingStatus | null> {
  return desktopOrBrowser("screen_reading_status", {}, parseScreenReadingStatus, () => null);
}

/** Start, interrupt or end the conversation: the same action as the conversation hotkey. */
export async function toggleConversation(): Promise<void> {
  return desktopOrBrowser("toggle_conversation", {}, ignoreReply, ignoreReply);
}

/**
 * What LocalFlow needs and whether each part is installed, for the Home setup checklist.
 * `null` outside the desktop app, and when this native build has no `readiness_status`
 * command: the checklist is then simply not shown.
 */
export async function getReadiness(): Promise<ReadinessItem[] | null> {
  if (!isDesktopApp()) {
    return null;
  }
  try {
    return await callNative("readiness_status", {}, parseReadiness);
  } catch {
    return null;
  }
}

/**
 * Which local Ollama models are installed. In the desktop app the native layer asks Ollama,
 * because Ollama refuses requests that come from the app window's own address; in a browser
 * the page asks directly.
 */
export async function checkLocalModels(): Promise<OllamaConnectionStatus> {
  if (!isDesktopApp()) {
    return checkOllamaStatus();
  }
  try {
    const names = await callNative("list_ollama_models", {}, (reply) =>
      parseStringList(reply, "list_ollama_models"),
    );
    return localModelsToStatus(names, defaultOllamaApiBaseUrl);
  } catch (error) {
    return {
      ok: false,
      baseUrl: defaultOllamaApiBaseUrl,
      code: "unavailable",
      message: errorMessage(error, "Local Ollama could not be reached."),
    };
  }
}

/** The coding-agent projects, with what settings should warn about. Empty in a browser. */
export async function listCodingProjects(): Promise<CodingProjectView[]> {
  return desktopOrBrowser(
    "list_coding_projects",
    {},
    (reply) => parseCodingProjects(reply, "list_coding_projects"),
    () => [],
  );
}

/**
 * Add a project (no `id`) or change one. Rejects with a sentence saying what is wrong with the
 * folder or the names. Projects only exist in the desktop app.
 */
export async function saveCodingProject(project: CodingProjectInput): Promise<CodingProjectView[]> {
  return desktopOrBrowser(
    "save_coding_project",
    { project },
    (reply) => parseCodingProjects(reply, "save_coding_project"),
    () => {
      throw new Error("Coding agent projects can only be added in the desktop app.");
    },
  );
}

export async function removeCodingProject(id: string): Promise<CodingProjectView[]> {
  return desktopOrBrowser(
    "remove_coding_project",
    { id },
    (reply) => parseCodingProjects(reply, "remove_coding_project"),
    () => [],
  );
}

/** Suggested spoken names for a folder, or why the folder cannot be used. */
export async function suggestProjectSpokenNames(path: string): Promise<SpokenNameSuggestion> {
  return desktopOrBrowser(
    "suggest_project_spoken_names",
    { path },
    parseSpokenNameSuggestion,
    () => ({
      folder: null,
      folderName: "",
      suggestions: [],
      problem: quickFolderProblem(path) ?? "Checking a folder needs the desktop app.",
    }),
  );
}

/**
 * Look for Claude Code, Windows Terminal and Cursor. Runs `claude --version` and `--help`, and
 * nothing else. `null` outside the desktop app.
 */
export async function detectCodingAgents(): Promise<CodingAgentsStatus | null> {
  return desktopOrBrowser("detect_coding_agents", {}, parseCodingAgents, () => null);
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
  if (typeof window === "undefined" || isDesktopApp()) {
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
  if (typeof window === "undefined" || isDesktopApp()) {
    return;
  }

  try {
    window.localStorage.setItem(settingsStorageKey, JSON.stringify(settings));
  } catch {
    // Settings remain in memory if browser storage is unavailable.
  }
}
