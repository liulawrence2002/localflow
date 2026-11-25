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

/**
 * Change one dictionary entry. Text is trimmed, but a row is never deleted by an edit: a patch
 * that would leave the phrase empty keeps the phrase there was. Callers commit text when the
 * user leaves the field, so typing ("pie torch") is never trimmed mid-word.
 */
export function updateDictionaryEntry(
  settings: LocalFlowSettings,
  id: string,
  patch: Partial<DraftDictionaryEntry>,
): LocalFlowSettings {
  return {
    ...settings,
    dictionary: settings.dictionary.map((entry) => {
      if (entry.id !== id) {
        return entry;
      }
      const next = sanitizeDictionaryEntry({ ...entry, ...patch });
      return next.phrase ? next : { ...next, phrase: entry.phrase };
    }),
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
    replacements: settings.replacements.map((rule) => {
      if (rule.id !== id) {
        return rule;
      }
      const next = sanitizeReplacementRule({ ...rule, ...patch });
      return {
        ...next,
        incorrect: next.incorrect || rule.incorrect,
        correct: next.correct || rule.correct,
      };
    }),
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
    snippets: settings.snippets.map((snippet) => {
      if (snippet.id !== id) {
        return snippet;
      }
      const next = sanitizeSnippet({ ...snippet, ...patch });
      return {
        ...next,
        trigger: next.trigger || snippet.trigger,
        expansion: next.expansion || snippet.expansion,
      };
    }),
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
    styles: settings.styles.map((style) => {
      if (style.id !== id) {
        return style;
      }
      const next = sanitizeStyleProfile({ ...style, ...patch });
      return next.name ? next : { ...next, name: style.name };
    }),
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

/** The end-of-speech wait the native layer accepts, in milliseconds. */
export const endOfSpeechRange = { min: 300, max: 3000 } as const;

/**
 * Settings with every number in the range its native field holds (`u8`/`u16` in Rust), so a
 * save can never be refused by the native layer for a value the UI let through.
 */
export function clampForNative(settings: LocalFlowSettings): LocalFlowSettings {
  return {
    ...settings,
    models: {
      ...settings.models,
      asrThreads: clampInteger(settings.models.asrThreads, 1, 255, 4),
    },
    microphone: {
      ...settings.microphone,
      endOfSpeechMs: clampInteger(
        settings.microphone.endOfSpeechMs,
        endOfSpeechRange.min,
        endOfSpeechRange.max,
        900,
      ),
      maxRecordingSeconds: clampInteger(settings.microphone.maxRecordingSeconds, 1, 65_535, 120),
    },
    styles: settings.styles.map(sanitizeStyleProfile),
  };
}

function clampInteger(value: number, min: number, max: number, fallback: number): number {
  if (!Number.isFinite(value)) {
    return fallback;
  }
  return Math.min(max, Math.max(min, Math.round(value)));
}

/**
 * Fill in anything missing from settings that came from storage or the native layer. Tolerates
 * absent sections and lists, so an older or partial payload still loads.
 */
export function normalizeSettings(settings: LocalFlowSettings): LocalFlowSettings {
  const listOf = <T>(value: T[] | undefined): T[] => (Array.isArray(value) ? value : []);
  const styles = listOf(settings.styles);
  return {
    ...defaultSettings,
    ...settings,
    hotkeys: { ...defaultSettings.hotkeys, ...settings.hotkeys },
    models: { ...defaultSettings.models, ...settings.models },
    microphone: { ...defaultSettings.microphone, ...settings.microphone },
    privacy: { ...defaultSettings.privacy, ...settings.privacy },
    dictionary: listOf(settings.dictionary).map(sanitizeDictionaryEntry),
    replacements: listOf(settings.replacements).map(sanitizeReplacementRule),
    snippets: listOf(settings.snippets).map(sanitizeSnippet),
    styles: styles.length
      ? styles.map((style) =>
          sanitizeStyleProfile({
            ...createCustomStyleDraft(style.name),
            ...style,
          }),
        )
      : defaultSettings.styles,
    codingAgents: {
      projects: (settings.codingAgents?.projects ?? []).map((project) => ({
        id: project.id,
        path: project.path,
        spokenNames: [...(project.spokenNames ?? [])],
      })),
    },
  };
}

function sanitizeDictionaryEntry(entry: DictionaryEntry): DictionaryEntry {
  return {
    ...entry,
    phrase: (entry.phrase ?? "").trim(),
    pronunciationHint: entry.pronunciationHint?.trim() || undefined,
  };
}

function sanitizeReplacementRule(rule: ReplacementRule): ReplacementRule {
  return {
    ...rule,
    incorrect: (rule.incorrect ?? "").trim(),
    correct: (rule.correct ?? "").trim(),
  };
}

function sanitizeSnippet(snippet: Snippet): Snippet {
  return {
    ...snippet,
    trigger: (snippet.trigger ?? "").trim(),
    expansion: (snippet.expansion ?? "").trim(),
  };
}

function sanitizeStyleProfile(style: StyleProfile): StyleProfile {
  return {
    ...style,
    name: (style.name ?? "").trim(),
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
