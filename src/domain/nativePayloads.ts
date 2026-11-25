/**
 * Light checks on what native commands return, so a payload of the wrong shape (an older or
 * newer native build) fails with a clear error instead of breaking a screen later.
 */

import type { CodingAgentsStatus, CodingProjectView, SpokenNameSuggestion } from "./codingAgents";
import {
  conversationPhases,
  type ConversationStatus,
  type ConversationStop,
  type ScreenReadingStatus,
} from "./conversation";
import { normalizeSettings } from "./settings";
import type { AppStatus, DiagnosticMetric, LocalFlowSettings } from "./types";

export interface TtsStatus {
  available: boolean;
  detail: string;
  speaking: boolean;
}

/** Thrown when a native reply does not have the expected shape. */
export class UnexpectedReplyError extends Error {
  constructor(command: string) {
    super(`LocalFlow's desktop app sent an unexpected reply to ${command}. Restart LocalFlow.`);
    this.name = "UnexpectedReplyError";
  }
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringOrNull(value: unknown): value is string | null {
  return value === null || typeof value === "string";
}

function parseDiagnostics(value: unknown): DiagnosticMetric[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value
    .filter(isRecord)
    .filter((row) => typeof row.label === "string" && typeof row.value === "string")
    .map((row) => ({
      label: row.label as string,
      value: row.value as string,
      status: (["ok", "warning", "blocked", "error"].includes(row.status as string)
        ? row.status
        : "warning") as DiagnosticMetric["status"],
    }));
}

export function parseAppStatus(value: unknown, command = "get_status"): AppStatus {
  if (!isRecord(value) || !isRecord(value.settings) || !isRecord(value.workflow)) {
    throw new UnexpectedReplyError(command);
  }
  const workflow = value.workflow as unknown as AppStatus["workflow"];
  if (typeof workflow.phase !== "string") {
    throw new UnexpectedReplyError(command);
  }
  return {
    workflow,
    settings: normalizeSettings(value.settings as unknown as LocalFlowSettings),
    history: Array.isArray(value.history) ? (value.history as AppStatus["history"]) : [],
    diagnostics: parseDiagnostics(value.diagnostics),
  };
}

function parseStop(value: unknown): ConversationStop | null {
  if (
    !isRecord(value) ||
    typeof value.reason !== "string" ||
    typeof value.message !== "string" ||
    typeof value.atMs !== "number"
  ) {
    return null;
  }
  return { reason: value.reason, message: value.message, atMs: value.atMs };
}

export function parseConversationStatus(value: unknown): ConversationStatus {
  if (
    !isRecord(value) ||
    typeof value.phase !== "string" ||
    !(conversationPhases as readonly string[]).includes(value.phase) ||
    typeof value.available !== "boolean" ||
    typeof value.detail !== "string" ||
    !isStringOrNull(value.hotkey ?? null)
  ) {
    throw new UnexpectedReplyError("conversation_status");
  }
  return {
    phase: value.phase as ConversationStatus["phase"],
    available: value.available,
    detail: value.detail,
    hotkey: (value.hotkey ?? null) as string | null,
    // Left undefined when this native build does not report it, so the window can fall back
    // to what the conversation event said.
    lastStop: value.lastStop === undefined ? undefined : parseStop(value.lastStop),
  };
}

export function parseScreenReadingStatus(value: unknown): ScreenReadingStatus {
  if (
    !isRecord(value) ||
    typeof value.available !== "boolean" ||
    typeof value.detail !== "string" ||
    typeof value.reading !== "boolean" ||
    !isStringOrNull(value.hotkey ?? null)
  ) {
    throw new UnexpectedReplyError("screen_reading_status");
  }
  return {
    available: value.available,
    detail: value.detail,
    reading: value.reading,
    hotkey: (value.hotkey ?? null) as string | null,
  };
}

export function parseTtsStatus(value: unknown): TtsStatus {
  if (
    !isRecord(value) ||
    typeof value.available !== "boolean" ||
    typeof value.detail !== "string" ||
    typeof value.speaking !== "boolean"
  ) {
    throw new UnexpectedReplyError("tts_status");
  }
  return { available: value.available, detail: value.detail, speaking: value.speaking };
}

export function parseOptionalText(value: unknown, command: string): string | null {
  if (!isStringOrNull(value ?? null)) {
    throw new UnexpectedReplyError(command);
  }
  return (value ?? null) as string | null;
}

export function parseStringList(value: unknown, command: string): string[] {
  if (!Array.isArray(value) || !value.every((item) => typeof item === "string")) {
    throw new UnexpectedReplyError(command);
  }
  return value as string[];
}

export function parseCodingProjects(value: unknown, command: string): CodingProjectView[] {
  if (
    !Array.isArray(value) ||
    !value.every(
      (item) =>
        isRecord(item) &&
        typeof item.id === "string" &&
        typeof item.path === "string" &&
        Array.isArray(item.spokenNames),
    )
  ) {
    throw new UnexpectedReplyError(command);
  }
  return (value as Array<Record<string, unknown>>).map((item) => ({
    id: item.id as string,
    path: item.path as string,
    spokenNames: (item.spokenNames as unknown[]).filter(
      (name): name is string => typeof name === "string",
    ),
    folderName: typeof item.folderName === "string" ? item.folderName : (item.path as string),
    folderProblem: typeof item.folderProblem === "string" ? item.folderProblem : null,
    sharedNames: Array.isArray(item.sharedNames)
      ? item.sharedNames.filter((name): name is string => typeof name === "string")
      : [],
  }));
}

export function parseCodingAgents(value: unknown): CodingAgentsStatus {
  if (!isRecord(value) || !Array.isArray(value.agents)) {
    throw new UnexpectedReplyError("detect_coding_agents");
  }
  return {
    agents: value.agents.filter(isRecord).map((agent) => ({
      agent: agent.agent as CodingAgentsStatus["agents"][number]["agent"],
      name: typeof agent.name === "string" ? agent.name : String(agent.agent),
      found: agent.found === true,
      path: typeof agent.path === "string" ? agent.path : null,
      version: typeof agent.version === "string" ? agent.version : null,
      canOpen: agent.canOpen === true,
      canAsk: agent.canAsk === true,
      detail: typeof agent.detail === "string" ? agent.detail : "",
    })),
    windowsTerminal: typeof value.windowsTerminal === "string" ? value.windowsTerminal : null,
  };
}

export function parseSpokenNameSuggestion(value: unknown): SpokenNameSuggestion {
  if (!isRecord(value)) {
    throw new UnexpectedReplyError("suggest_project_spoken_names");
  }
  return {
    folder: typeof value.folder === "string" ? value.folder : null,
    folderName: typeof value.folderName === "string" ? value.folderName : "",
    suggestions: Array.isArray(value.suggestions)
      ? value.suggestions.filter((name): name is string => typeof name === "string")
      : [],
    problem: typeof value.problem === "string" ? value.problem : null,
  };
}
