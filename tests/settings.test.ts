import { describe, expect, it } from "vitest";
import { defaultSettings } from "../src/domain/defaults";
import {
  addDictionaryEntry,
  addReplacementRule,
  addSnippet,
  addStyleProfile,
  createCustomStyleDraft,
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
  it("pins the default Ollama model to the local gemma4:12b-it-qat model", () => {
    expect(defaultSettings.models.ollamaModel).toBe("gemma4:12b-it-qat");
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
