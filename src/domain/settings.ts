import type {
  AppCategory,
  CleanupLevel,
  DictionaryEntry,
  LocalFlowSettings,
  ReplacementRule,
  Snippet,
  StyleProfile,
} from "./types";
import { defaultSettings } from "./defaults";

type DraftDictionaryEntry = Omit<DictionaryEntry, "id">;
type DraftReplacementRule = Omit<ReplacementRule, "id">;
type DraftSnippet = Omit<Snippet, "id">;
type DraftStyleProfile = Omit<StyleProfile, "id">;

export function addDictionaryEntry(
  settings: LocalFlowSettings,
  draft: DraftDictionaryEntry,
  id = createId("dict"),
): LocalFlowSettings {
  const entry = sanitizeDictionaryEntry({ ...draft, id });
  if (!entry.phrase) {
    return settings;
  }

  return {
    ...settings,
    dictionary: [...settings.dictionary, entry],
  };
}

export function updateDictionaryEntry(
  settings: LocalFlowSettings,
  id: string,
  patch: Partial<DraftDictionaryEntry>,
): LocalFlowSettings {
  return {
    ...settings,
    dictionary: settings.dictionary
      .map((entry) => (entry.id === id ? sanitizeDictionaryEntry({ ...entry, ...patch }) : entry))
      .filter((entry) => entry.phrase),
  };
}

export function removeDictionaryEntry(settings: LocalFlowSettings, id: string): LocalFlowSettings {
  return {
    ...settings,
    dictionary: settings.dictionary.filter((entry) => entry.id !== id),
  };
}

export function addReplacementRule(
  settings: LocalFlowSettings,
  draft: DraftReplacementRule,
  id = createId("replace"),
): LocalFlowSettings {
  const rule = sanitizeReplacementRule({ ...draft, id });
  if (!rule.incorrect || !rule.correct) {
    return settings;
  }

  return {
    ...settings,
    replacements: [...settings.replacements, rule],
  };
}

export function updateReplacementRule(
  settings: LocalFlowSettings,
  id: string,
  patch: Partial<DraftReplacementRule>,
): LocalFlowSettings {
  return {
    ...settings,
    replacements: settings.replacements
      .map((rule) => (rule.id === id ? sanitizeReplacementRule({ ...rule, ...patch }) : rule))
      .filter((rule) => rule.incorrect && rule.correct),
  };
}

export function removeReplacementRule(settings: LocalFlowSettings, id: string): LocalFlowSettings {
  return {
    ...settings,
    replacements: settings.replacements.filter((rule) => rule.id !== id),
  };
}

export function addSnippet(
  settings: LocalFlowSettings,
  draft: DraftSnippet,
  id = createId("snippet"),
): LocalFlowSettings {
  const snippet = sanitizeSnippet({ ...draft, id });
  if (!snippet.trigger || !snippet.expansion) {
    return settings;
  }

  return {
    ...settings,
    snippets: [...settings.snippets, snippet],
  };
}

export function updateSnippet(
  settings: LocalFlowSettings,
  id: string,
  patch: Partial<DraftSnippet>,
): LocalFlowSettings {
  return {
    ...settings,
    snippets: settings.snippets
      .map((snippet) => (snippet.id === id ? sanitizeSnippet({ ...snippet, ...patch }) : snippet))
      .filter((snippet) => snippet.trigger && snippet.expansion),
  };
}

export function removeSnippet(settings: LocalFlowSettings, id: string): LocalFlowSettings {
  return {
    ...settings,
    snippets: settings.snippets.filter((snippet) => snippet.id !== id),
  };
}

export function addStyleProfile(
  settings: LocalFlowSettings,
  draft: DraftStyleProfile,
  id = createId("style"),
): LocalFlowSettings {
  const style = sanitizeStyleProfile({ ...draft, id });
  if (!style.name) {
    return settings;
  }

  return {
    ...settings,
    styles: [...settings.styles, style],
  };
}

export function updateStyleProfile(
  settings: LocalFlowSettings,
  id: string,
  patch: Partial<DraftStyleProfile>,
): LocalFlowSettings {
  return {
    ...settings,
    styles: settings.styles
      .map((style) => (style.id === id ? sanitizeStyleProfile({ ...style, ...patch }) : style))
      .filter((style) => style.name),
  };
}

export function removeStyleProfile(settings: LocalFlowSettings, id: string): LocalFlowSettings {
  if (settings.styles.length <= 1) {
    return settings;
  }

  return {
    ...settings,
    styles: settings.styles.filter((style) => style.id !== id),
  };
}

export function createCustomStyleDraft(name = "Custom application"): DraftStyleProfile {
  return {
    name,
    category: "generic_text_field",
    cleanupLevel: "balanced",
    conciseness: 5,
    formality: 5,
    contractions: true,
    emoji: "preserve",
    paragraphLength: "medium",
    bulletPreference: "preserve",
    greetingBehavior: "preserve",
    signOffBehavior: "preserve",
    aggressiveFillerRemoval: false,
    allowSentenceFragments: true,
  };
}

export function normalizeSettings(settings: LocalFlowSettings): LocalFlowSettings {
  return {
    ...defaultSettings,
    ...settings,
    hotkeys: { ...defaultSettings.hotkeys, ...settings.hotkeys },
    models: { ...defaultSettings.models, ...settings.models },
    microphone: { ...defaultSettings.microphone, ...settings.microphone },
    privacy: { ...defaultSettings.privacy, ...settings.privacy },
    dictionary: settings.dictionary.map(sanitizeDictionaryEntry),
    replacements: settings.replacements.map(sanitizeReplacementRule),
    snippets: settings.snippets.map(sanitizeSnippet),
    styles: settings.styles.length
      ? settings.styles.map((style) =>
          sanitizeStyleProfile({
            ...createCustomStyleDraft(style.name),
            ...style,
          }),
        )
      : defaultSettings.styles,
  };
}

function sanitizeDictionaryEntry(entry: DictionaryEntry): DictionaryEntry {
  return {
    ...entry,
    phrase: entry.phrase.trim(),
    pronunciationHint: entry.pronunciationHint?.trim() || undefined,
  };
}

function sanitizeReplacementRule(rule: ReplacementRule): ReplacementRule {
  return {
    ...rule,
    incorrect: rule.incorrect.trim(),
    correct: rule.correct.trim(),
  };
}

function sanitizeSnippet(snippet: Snippet): Snippet {
  return {
    ...snippet,
    trigger: snippet.trigger.trim(),
    expansion: snippet.expansion.trim(),
  };
}

function sanitizeStyleProfile(style: StyleProfile): StyleProfile {
  return {
    ...style,
    name: style.name.trim(),
    conciseness: clampSetting(style.conciseness),
    formality: clampSetting(style.formality),
  };
}

function clampSetting(value: number): number {
  if (!Number.isFinite(value)) {
    return 5;
  }

  return Math.min(10, Math.max(1, Math.round(value)));
}

function createId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

export const appCategories: AppCategory[] = [
  "personal_messaging",
  "work_messaging",
  "email",
  "document",
  "code_editor",
  "terminal",
  "search_field",
  "generic_text_field",
];

export const cleanupLevels: CleanupLevel[] = ["verbatim", "light", "balanced", "strong"];
