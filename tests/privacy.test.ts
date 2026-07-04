import { describe, expect, it } from "vitest";
import {
  applyHistoryRetention,
  redactSensitiveText,
  shouldRetainContext,
} from "../src/domain/privacy";
import type { PrivacySettings, SessionHistoryItem } from "../src/domain/types";

const basePrivacy: PrivacySettings = {
  historyRetention: "original_and_cleaned",
  deleteAfter: "never",
  activeAppDetection: true,
  accessibilityContext: false,
  selectedTextTransforms: false,
  contextRetention: false,
  deleteAudioAfterProcessing: true,
};

const history: SessionHistoryItem[] = [
  {
    id: "recent",
    completedAt: "2026-07-04T00:00:00.000Z",
    targetApplication: "Notepad",
    rawTranscript: "raw recent",
    finalText: "Raw recent.",
    cleanupLevel: "balanced",
  },
  {
    id: "old",
    completedAt: "2026-06-20T00:00:00.000Z",
    targetApplication: "Notepad",
    rawTranscript: "raw old",
    finalText: "Raw old.",
    cleanupLevel: "balanced",
  },
];

describe("privacy retention", () => {
  it("drops history completely when disabled", () => {
    expect(
      applyHistoryRetention(
        history,
        { ...basePrivacy, historyRetention: "off" },
        new Date("2026-07-04T01:00:00Z"),
      ),
    ).toEqual([]);
  });

  it("keeps final transcript while clearing raw transcript in transcript-only mode", () => {
    const retained = applyHistoryRetention(
      history.slice(0, 1),
      { ...basePrivacy, historyRetention: "transcript_only" },
      new Date("2026-07-04T01:00:00Z"),
    );

    expect(retained[0]?.rawTranscript).toBe("");
    expect(retained[0]?.finalText).toBe("Raw recent.");
  });

  it("enforces delete-after windows", () => {
    const retained = applyHistoryRetention(
      history,
      { ...basePrivacy, deleteAfter: "7d" },
      new Date("2026-07-04T01:00:00Z"),
    );

    expect(retained.map((item) => item.id)).toEqual(["recent"]);
  });

  it("keeps context only when retention and context collection are both enabled", () => {
    expect(shouldRetainContext(basePrivacy)).toBe(false);
    expect(shouldRetainContext({ ...basePrivacy, contextRetention: true })).toBe(true);
  });

  it("redacts dictated content in diagnostics", () => {
    expect(redactSensitiveText("secret dictated content")).toBe("[redacted:23 chars]");
    expect(redactSensitiveText("   ")).toBe("[empty]");
  });
});
