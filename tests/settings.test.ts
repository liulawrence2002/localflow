import { describe, expect, it } from "vitest";
import { defaultSettings } from "../src/domain/defaults";
import {
  addDictionaryEntry,
  addReplacementRule,
  addSnippet,
  addStyleProfile,
  clampForNative,
  createCustomStyleDraft,
  endOfSpeechRange,
  normalizeSettings,
  removeDictionaryEntry,
  removeReplacementRule,
  removeSnippet,
  removeStyleProfile,
  updateDictionaryEntry,
  updateReplacementRule,
  updateSnippet,
  updateStyleProfile,
} from "../src/domain/settings";

describe("settings mutations", () => {
  it("defaults the Ollama model to the fast local llama3.2:3b model", () => {
    expect(defaultSettings.models.ollamaModel).toBe("llama3.2:3b");
  });

  it("adds, updates, and removes dictionary entries", () => {
    const added = addDictionaryEntry(
      defaultSettings,
      {
        phrase: "  Jon Smythe ",
        pronunciationHint: "John Smith",
        category: "name",
        caseSensitive: false,
      },
      "dict-jon",
    );

    expect(added.dictionary[added.dictionary.length - 1]).toMatchObject({ phrase: "Jon Smythe" });

    const updated = updateDictionaryEntry(added, "dict-jon", { caseSensitive: true });
    expect(updated.dictionary.find((entry) => entry.id === "dict-jon")?.caseSensitive).toBe(true);

    const removed = removeDictionaryEntry(updated, "dict-jon");
    expect(removed.dictionary.some((entry) => entry.id === "dict-jon")).toBe(false);
  });

  it("guards empty replacements while preserving enabled rules", () => {
    expect(
      addReplacementRule(defaultSettings, {
        incorrect: "",
        correct: "Draught",
        enabled: true,
      }),
    ).toBe(defaultSettings);

    const added = addReplacementRule(
      defaultSettings,
      { incorrect: "draft", correct: "Draught", enabled: true },
      "replace-draft",
    );
    const updated = updateReplacementRule(added, "replace-draft", { enabled: false });

    expect(updated.replacements.find((rule) => rule.id === "replace-draft")?.enabled).toBe(false);
    expect(removeReplacementRule(updated, "replace-draft").replacements).toHaveLength(
      defaultSettings.replacements.length,
    );
  });

  it("adds and edits exact snippets", () => {
    const added = addSnippet(
      defaultSettings,
      {
        trigger: " insert support signature ",
        expansion: "Thanks,\nSupport",
        enabled: true,
        allowCleanup: false,
      },
      "snippet-support",
    );
    const updated = updateSnippet(added, "snippet-support", { allowCleanup: true });

    expect(updated.snippets.find((snippet) => snippet.id === "snippet-support")?.allowCleanup).toBe(
      true,
    );
    expect(removeSnippet(updated, "snippet-support").snippets).toHaveLength(
      defaultSettings.snippets.length,
    );
  });

  it("adds complete custom style profiles and clamps numeric settings", () => {
    const added = addStyleProfile(
      defaultSettings,
      { ...createCustomStyleDraft("Issue tracker"), conciseness: 99, formality: -3 },
      "style-issues",
    );
    const style = added.styles.find((item) => item.id === "style-issues");

    expect(style).toMatchObject({
      name: "Issue tracker",
      conciseness: 10,
      formality: 1,
      greetingBehavior: "preserve",
      signOffBehavior: "preserve",
      aggressiveFillerRemoval: false,
      allowSentenceFragments: true,
    });

    const updated = updateStyleProfile(added, "style-issues", { emoji: "never" });
    expect(updated.styles.find((item) => item.id === "style-issues")?.emoji).toBe("never");
    expect(removeStyleProfile(updated, "style-issues").styles).toHaveLength(
      defaultSettings.styles.length,
    );
  });

  it("never deletes a row because an edit emptied a required field", () => {
    const blankPhrase = updateDictionaryEntry(defaultSettings, "dict-pytorch", { phrase: "   " });
    expect(blankPhrase.dictionary).toHaveLength(defaultSettings.dictionary.length);
    expect(blankPhrase.dictionary.find((entry) => entry.id === "dict-pytorch")?.phrase).toBe(
      "PyTorch",
    );

    const blankRule = updateReplacementRule(defaultSettings, "replace-pytorch", { correct: "" });
    expect(blankRule.replacements.find((rule) => rule.id === "replace-pytorch")).toMatchObject({
      incorrect: "pie torch",
      correct: "PyTorch",
    });

    const blankSnippet = updateSnippet(defaultSettings, "snippet-signature", { trigger: "" });
    expect(
      blankSnippet.snippets.find((snippet) => snippet.id === "snippet-signature")?.trigger,
    ).toBe("insert my signature");
  });

  it("keeps the only style when its name is cleared, so styles is never empty", () => {
    const single = { ...defaultSettings, styles: defaultSettings.styles.slice(0, 1) };
    const id = single.styles[0]?.id ?? "";

    const updated = updateStyleProfile(single, id, { name: "" });

    expect(updated.styles).toHaveLength(1);
    expect(updated.styles[0]?.name).toBe(single.styles[0]?.name);
  });

  it("keeps the inner space of a committed phrase and trims only its ends", () => {
    const updated = updateReplacementRule(defaultSettings, "replace-pytorch", {
      incorrect: "  pie  torch ",
    });

    expect(updated.replacements[0]?.incorrect).toBe("pie  torch");
  });

  it("clamps numbers into the ranges the native fields hold", () => {
    const clamped = clampForNative({
      ...defaultSettings,
      models: { ...defaultSettings.models, asrThreads: -3 },
      microphone: {
        ...defaultSettings.microphone,
        endOfSpeechMs: 99_999,
        maxRecordingSeconds: 1.6,
      },
    });

    expect(clamped.models.asrThreads).toBe(1);
    expect(clamped.microphone.endOfSpeechMs).toBe(endOfSpeechRange.max);
    expect(clamped.microphone.maxRecordingSeconds).toBe(2);
    expect(clampForNative(defaultSettings).microphone.endOfSpeechMs).toBe(900);
  });

  it("loads a partial payload without throwing", () => {
    const partial = { models: { ollamaModel: "qwen3:4b" } } as unknown as typeof defaultSettings;

    const normalized = normalizeSettings(partial);

    expect(normalized.models.ollamaModel).toBe("qwen3:4b");
    expect(normalized.dictionary).toEqual([]);
    expect(normalized.styles.length).toBeGreaterThan(0);
    expect(normalized.microphone.endOfSpeechMs).toBe(900);
  });

  it("normalizes saved settings that predate new style fields", () => {
    const legacy = structuredClone(defaultSettings);
    const partialStyle = {
      id: "legacy",
      name: "Legacy",
      category: "email",
      cleanupLevel: "balanced",
      conciseness: 5,
      formality: 6,
      contractions: true,
      emoji: "preserve",
      paragraphLength: "short",
      bulletPreference: "preserve",
    };
    legacy.styles = [partialStyle as (typeof legacy.styles)[number]];

    expect(normalizeSettings(legacy).styles[0]).toMatchObject({
      greetingBehavior: "preserve",
      signOffBehavior: "preserve",
      aggressiveFillerRemoval: false,
      allowSentenceFragments: true,
    });
  });
});
