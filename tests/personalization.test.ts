import { describe, expect, it } from "vitest";
import {
  resolveExplicitSelfCorrections,
  runDeterministicPersonalization,
} from "../src/domain/personalization";

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

  it("resolves explicit self-correction examples conservatively", () => {
    expect(resolveExplicitSelfCorrections("Meet me Tuesday no Wednesday")).toBe(
      "Meet me Wednesday",
    );
    expect(resolveExplicitSelfCorrections("The total was fifteen sorry fifty dollars")).toBe(
      "The total was fifty dollars",
    );
    expect(resolveExplicitSelfCorrections("Send it to James actually send it to Sarah")).toBe(
      "send it to Sarah",
    );
    expect(
      resolveExplicitSelfCorrections(
        "We should deploy Friday let me restart We should deploy Monday after testing",
      ),
    ).toBe("We should deploy Monday after testing");
  });
});
