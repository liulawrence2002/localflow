import { describe, expect, it } from "vitest";
import { runDeterministicPersonalization } from "../src/domain/personalization";

describe("deterministic personalization", () => {
  it("applies boundary-aware replacements without touching unrelated words", () => {
    const result = runDeterministicPersonalization(
      "open pie torch but do not change magpie torchlight",
      [{ id: "r1", incorrect: "pie torch", correct: "PyTorch", enabled: true }],
      [],
    );

    expect(result.text).toBe("open PyTorch but do not change magpie torchlight");
    expect(result.appliedReplacements).toEqual(["r1"]);
  });

  it("expands exact snippets before cleanup", () => {
    const result = runDeterministicPersonalization(
      "insert my signature",
      [],
      [
        {
          id: "sig",
          trigger: "insert my signature",
          expansion: "Best,\nAlex",
          enabled: true,
          allowCleanup: false,
        },
      ],
    );

    expect(result.text).toBe("Best,\nAlex");
    expect(result.expandedSnippets).toEqual(["sig"]);
  });

  it("converts spoken punctuation commands", () => {
    const result = runDeterministicPersonalization(
      "hello comma new paragraph bullet point ship it",
      [],
      [],
    );

    expect(result.text).toBe("hello,\n\n- ship it");
  });
});
