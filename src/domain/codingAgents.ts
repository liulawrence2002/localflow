/** Voice commands for coding agents: the projects the user can name, and which agents exist. */

export interface CodingProject {
  id: string;
  /** An existing absolute folder, checked by the native layer when saved. */
  path: string;
  /** What the user says to mean this project, most preferred first. */
  spokenNames: string[];
}

export interface CodingAgentSettings {
  projects: CodingProject[];
}

/** A project as the settings screen shows it. */
export interface CodingProjectView extends CodingProject {
  folderName: string;
  /** Why the folder can no longer be used, if it cannot. */
  folderProblem: string | null;
  /** Spoken names another project also has; saying one is asked about, never guessed. */
  sharedNames: string[];
}

/** A project to add (no `id`) or change. */
export interface CodingProjectInput {
  id?: string;
  path: string;
  spokenNames: string[];
}

export interface SpokenNameSuggestion {
  /** The folder as it would be stored, when it is usable. */
  folder: string | null;
  folderName: string;
  suggestions: string[];
  /** Why the folder cannot be used. */
  problem: string | null;
}

export type AgentKind = "claudeCode" | "cursor" | "codex" | "cursorAgent";

export interface AgentStatus {
  agent: AgentKind;
  name: string;
  found: boolean;
  path: string | null;
  version: string | null;
  canOpen: boolean;
  canAsk: boolean;
  detail: string;
}

export interface CodingAgentsStatus {
  agents: AgentStatus[];
  windowsTerminal: string | null;
}

export const defaultCodingAgentSettings: CodingAgentSettings = { projects: [] };

/** Characters Windows Terminal would misread in a folder path, with the reason for each. */
const terminalCharacters: Array<[string, string]> = [
  [
    ";",
    "The path contains a semicolon. Windows Terminal treats ; as the start of a new command, " +
      "even inside quotes, so LocalFlow cannot open this folder safely. Rename the folder or " +
      "choose another.",
  ],
  [
    "%",
    "The path contains a percent sign. Windows Terminal replaces %NAME% with an environment " +
      "variable and cannot be told not to, so LocalFlow cannot open this folder safely. Rename " +
      "the folder or choose another.",
  ],
  [
    '"',
    'The path contains ", which cannot be passed to Windows Terminal safely. Remove any quotes ' +
      "around the path.",
  ],
];

/**
 * Tidy a pasted folder path. Explorer's "Copy as path" wraps it in quotes, which are removed
 * here; the native layer checks everything else.
 */
export function cleanFolderInput(text: string): string {
  const trimmed = text.trim();
  if (trimmed.length >= 2 && trimmed.startsWith('"') && trimmed.endsWith('"')) {
    return trimmed.slice(1, -1).trim();
  }
  return trimmed;
}

/**
 * What can be said about a folder path without asking the native layer: empty, a character
 * Windows Terminal would misread, or not a full path. `null` when it looks usable.
 */
export function quickFolderProblem(path: string): string | null {
  if (!path) {
    return "Enter the full path of a project folder.";
  }
  for (const [character, reason] of terminalCharacters) {
    if (path.includes(character)) {
      return reason;
    }
  }
  if (!/^[A-Za-z]:[\\/]/.test(path) && !/^\\\\[^\\?.]/.test(path)) {
    return "Enter the full path, starting with a drive letter such as C:\\.";
  }
  return null;
}

/** What spoken names are compared by: lower case, letters and digits only. */
export function spokenNameKey(name: string): string {
  return name.toLowerCase().replace(/[^\p{L}\p{N}]+/gu, "");
}

/** Spoken names typed as a comma- or line-separated list, tidied, without repeats as heard. */
export function parseSpokenNames(text: string): string[] {
  const seen = new Set<string>();
  const names: string[] = [];
  for (const raw of text.split(/[,\n]/)) {
    const name = raw.trim().replace(/\s+/g, " ");
    const key = spokenNameKey(name);
    if (!key || seen.has(key)) {
      continue;
    }
    seen.add(key);
    names.push(name);
  }
  return names;
}

export function formatSpokenNames(names: string[]): string {
  return names.join(", ");
}

export type AgentTone = "ok" | "warning" | "blocked";

export interface AgentView {
  name: string;
  /** "Found", "Not found", "Not set up yet". */
  state: string;
  tone: AgentTone;
  /** Version and path, when known. */
  location: string | null;
  detail: string;
}

/** One agent's detection result as the settings screen shows it. */
export function describeAgent(status: AgentStatus): AgentView {
  const notSetUp = status.agent === "codex" || status.agent === "cursorAgent";
  const fullyUsable =
    status.agent === "claudeCode" ? status.canOpen && status.canAsk : status.canOpen;
  const location = status.path
    ? status.version
      ? `${status.version} at ${status.path}`
      : status.path
    : null;
  return {
    name: status.name,
    state: notSetUp ? "Not set up yet" : status.found ? "Found" : "Not found",
    tone: notSetUp || !status.found ? "blocked" : fullyUsable ? "ok" : "warning",
    location,
    detail: status.detail,
  };
}

/** The example phrases shown in settings, using the first project's name when there is one. */
export function examplePhrases(projects: CodingProject[]): string[] {
  const name = projects[0]?.spokenNames[0] ?? "my project";
  return [
    `Open Claude Code in ${name}`,
    `Ask Claude what the tests in ${name} check`,
    `Open ${name} in Cursor`,
  ];
}
