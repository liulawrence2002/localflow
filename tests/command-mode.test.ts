import { describe, expect, it } from "vitest";
import {
  classifyCommandIntent,
  parseCommandModeResponse,
  planCommandMode,
} from "../src/domain/commandMode";

describe("command mode", () => {
  it("requires selected text for destructive transforms", () => {
    expect(
      planCommandMode({
        selectedText: "",
        instruction: "Make this more concise.",
        appCategory: "work_messaging",
        previewThresholdChars: 120,
      }),
    ).toEqual({ ok: false, reason: "selected_text_required" });
  });

  it("rejects operating-system command execution", () => {
    expect(
      planCommandMode({
        selectedText: "Please review this.",
        instruction: "Run powershell and delete files.",
        appCategory: "document",
        previewThresholdChars: 120,
      }),
    ).toEqual({ ok: false, reason: "operating_system_command_rejected" });
  });

  it("classifies common editing intents and requires preview for large changes", () => {
    const plan = planCommandMode({
      selectedText: "One. Two. Three.".repeat(20),
      instruction: "Turn this into bullet points.",
      appCategory: "document",
      previewThresholdChars: 120,
    });

    expect(classifyCommandIntent("Fix the grammar.")).toBe("fix_grammar");
    expect(plan).toMatchObject({
      ok: true,
      intent: "bullet_points",
      previewRequired: true,
    });
  });

  it("parses strict command response and keeps undo text", () => {
    const plan = planCommandMode({
      selectedText: "hello there",
      instruction: "Make this more professional.",
      appCategory: "email",
      previewThresholdChars: 120,
    });
    if (!plan.ok) {
      throw new Error("Expected command plan");
    }

    const result = parseCommandModeResponse(
      JSON.stringify({
        text: "Hello,",
        confidence: 0.88,
        resolved_corrections: [],
        warnings: [],
      }),
      plan,
    );

    expect(result).toMatchObject({
      text: "Hello,",
      undoText: "hello there",
      previewRequired: false,
    });
  });
});
