import { describe, expect, it } from "vitest";
import { parseRefinementResponse } from "../src/domain/refinement";

describe("refinement response validation", () => {
  it("accepts strict JSON contract", () => {
    const parsed = parseRefinementResponse(
      JSON.stringify({
        text: "Send it to Sarah.",
        confidence: 0.87,
        resolved_corrections: ["James -> Sarah"],
        warnings: [],
      }),
    );

    expect(parsed.ok).toBe(true);
  });

  it("rejects non-JSON model chatter", () => {
    const parsed = parseRefinementResponse("Sure, here is the cleaned text: Send it.");

    expect(parsed.ok).toBe(false);
  });
});
