import type { AppCategory, PrivacySettings } from "./types";

export interface ContextProvider {
  getSnapshot(): Promise<RawContextSnapshot>;
}

export interface RawContextSnapshot {
  applicationName?: string;
  windowTitle?: string;
  fieldRole?: string;
  url?: string;
  selectedText?: string;
  beforeCursor?: string;
  afterCursor?: string;
  cursorAtSentenceStart?: boolean;
  codeMode?: boolean;
  passwordField?: boolean;
  protectedField?: boolean;
}

export interface ContextLimits {
  beforeCursorChars: number;
  afterCursorChars: number;
  selectedTextChars: number;
}

export interface ContextSnapshot {
  applicationName: string;
  windowTitle: string;
  category: AppCategory;
  selectedText: string;
  beforeCursor: string;
  afterCursor: string;
  cursorAtSentenceStart: boolean;
  codeMode: boolean;
  protectedField: boolean;
  collected: {
    activeApp: boolean;
    accessibilityText: boolean;
    selectedText: boolean;
  };
}

export const defaultContextLimits: ContextLimits = {
  beforeCursorChars: 500,
  afterCursorChars: 250,
  selectedTextChars: 4000,
};

export function prepareContextSnapshot(
  raw: RawContextSnapshot,
  privacy: PrivacySettings,
  limits: ContextLimits = defaultContextLimits,
): ContextSnapshot {
  validateLimits(limits);

  const protectedField = Boolean(raw.protectedField || raw.passwordField);
  const applicationName =
    privacy.activeAppDetection && !protectedField ? clean(raw.applicationName) : "";
  const windowTitle = privacy.activeAppDetection && !protectedField ? clean(raw.windowTitle) : "";
  const category =
    privacy.activeAppDetection && !protectedField
      ? classifyApplicationCategory(raw)
      : "generic_text_field";
  const accessibilityText = privacy.accessibilityContext && !protectedField;
  const selectedTextEnabled = privacy.selectedTextTransforms && !protectedField;

  return {
    applicationName,
    windowTitle,
    category,
    selectedText: selectedTextEnabled
      ? limitContextText(raw.selectedText ?? "", limits.selectedTextChars)
      : "",
    beforeCursor: accessibilityText
      ? keepTail(raw.beforeCursor ?? "", limits.beforeCursorChars)
      : "",
    afterCursor: accessibilityText ? keepHead(raw.afterCursor ?? "", limits.afterCursorChars) : "",
    cursorAtSentenceStart: accessibilityText ? Boolean(raw.cursorAtSentenceStart) : false,
    codeMode: accessibilityText ? Boolean(raw.codeMode) || category === "code_editor" : false,
    protectedField,
    collected: {
      activeApp: Boolean(privacy.activeAppDetection && !protectedField),
      accessibilityText,
      selectedText: selectedTextEnabled,
    },
  };
}

export function classifyApplicationCategory(raw: RawContextSnapshot): AppCategory {
  const haystack = [raw.applicationName, raw.windowTitle, raw.fieldRole, raw.url]
    .filter((value): value is string => Boolean(value))
    .join(" ")
    .toLowerCase();

  if (matchesAny(haystack, ["password", "secure input"])) {
    return "generic_text_field";
  }

  if (
    matchesAny(haystack, [
      "visual studio code",
      "vscode",
      "cursor",
      "jetbrains",
      "intellij",
      "pycharm",
      "webstorm",
      "sublime text",
      "notepad++",
    ])
  ) {
    return "code_editor";
  }

  if (
    matchesAny(haystack, [
      "terminal",
      "powershell",
      "command prompt",
      "cmd.exe",
      "windows terminal",
      "wezterm",
      "alacritty",
    ])
  ) {
    return "terminal";
  }

  if (matchesAny(haystack, ["outlook", "gmail", "thunderbird", "mail", "compose email", "inbox"])) {
    return "email";
  }

  if (
    matchesAny(haystack, [
      "slack",
      "microsoft teams",
      "teams",
      "zoom chat",
      "google chat",
      "linear",
      "jira",
    ])
  ) {
    return "work_messaging";
  }

  if (
    matchesAny(haystack, [
      "messages",
      "imessage",
      "whatsapp",
      "telegram",
      "signal",
      "discord",
      "messenger",
    ])
  ) {
    return "personal_messaging";
  }

  if (
    matchesAny(haystack, [
      "word",
      "google docs",
      "docs.google.com",
      "notion",
      "obsidian",
      "onenote",
      "libreoffice writer",
    ])
  ) {
    return "document";
  }

  if (matchesAny(haystack, ["search", "address bar", "omnibox", "find in page"])) {
    return "search_field";
  }

  return "generic_text_field";
}

export function contextForCleanup(snapshot: ContextSnapshot): {
  appCategory: AppCategory;
  beforeCursor: string;
  afterCursor: string;
  codeMode: boolean;
} {
  return {
    appCategory: snapshot.category,
    beforeCursor: snapshot.beforeCursor,
    afterCursor: snapshot.afterCursor,
    codeMode: snapshot.codeMode,
  };
}

function clean(value: string | undefined): string {
  return (value ?? "").trim();
}

function limitContextText(value: string, maxChars: number): string {
  return keepHead(value.trim(), maxChars);
}

function keepHead(value: string, maxChars: number): string {
  return value.length > maxChars ? value.slice(0, maxChars) : value;
}

function keepTail(value: string, maxChars: number): string {
  return value.length > maxChars ? value.slice(value.length - maxChars) : value;
}

function matchesAny(value: string, needles: string[]): boolean {
  return needles.some((needle) => value.includes(needle));
}

function validateLimits(limits: ContextLimits): void {
  if (
    !Number.isInteger(limits.beforeCursorChars) ||
    !Number.isInteger(limits.afterCursorChars) ||
    !Number.isInteger(limits.selectedTextChars) ||
    limits.beforeCursorChars < 0 ||
    limits.afterCursorChars < 0 ||
    limits.selectedTextChars < 0
  ) {
    throw new Error("Context limits must be non-negative integers.");
  }
}
