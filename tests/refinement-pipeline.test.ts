import { describe, expect, it } from "vitest";
import { runLocalCleanup, type DictationCleanupInput } from "../src/domain/refinementPipeline";

const input: DictationCleanupInput = {
  rawTranscript: "send it to james actually send it to sarah",
  deterministicText: "send it to Sarah",
  appCategory: "work_messaging",
  beforeCursor: "",
  afterCursor: "",
  cleanupLevel: "balanced",
  codeMode: false,
  dictionary: [],
  replacements: [],
  styleName: "Work messages",
};

describe("local cleanup JSON contract", () => {
  it("uses valid strict JSON from the model", async () => {
    const result = await runLocalCleanup(
      {
        complete: async () =>
          JSON.stringify({
            text: "Send it to Sarah.",
            confidence: 0.9,
            resolved_corrections: ["James -> Sarah"],
            warnings: [],
          }),
      },
      input,
    );

    expect(result.source).toBe("model");
    expect(result.text).toBe("Send it to Sarah.");
  });

  it("retries once with a repair prompt when the first response is invalid", async () => {
    const payloads = [
      "Sure, here is the cleaned text: Send it to Sarah.",
      JSON.stringify({
        text: "Send it to Sarah.",
        confidence: 0.81,
        resolved_corrections: [],
        warnings: ["Repaired JSON."],
      }),
    ];

    const result = await runLocalCleanup(
      {
        complete: async () => payloads.shift() ?? "",
      },
      input,
    );

    expect(result.source).toBe("repaired_model");
    expect(result.text).toBe("Send it to Sarah.");
  });

  it("falls back to deterministic text without losing the raw transcript", async () => {
    const result = await runLocalCleanup({ complete: async () => "not json" }, input);

    expect(result.source).toBe("deterministic_fallback");
    expect(result.text).toBe(input.deterministicText);
    expect(result.rawTranscript).toBe(input.rawTranscript);
    expect(result.warnings[0]).toContain("invalid JSON twice");
  });
});
