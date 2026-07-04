import { describe, expect, it } from "vitest";
import { canUndoCleanup, restorePreCleanupText } from "../src/domain/undo";
import type { SessionHistoryItem } from "../src/domain/types";

const item: SessionHistoryItem = {
  id: "one",
  completedAt: "2026-07-04T00:00:00.000Z",
  targetApplication: "Notepad",
  rawTranscript: "send it to james actually sarah",
  deterministicText: "send it to sarah",
  refinedText: "Send it to Sarah.",
  finalText: "Send it to Sarah.",
  cleanupLevel: "balanced",
};

describe("undo cleanup", () => {
  it("restores deterministic text before raw transcript by default", () => {
    expect(restorePreCleanupText(item)).toBe("send it to sarah");
    expect(restorePreCleanupText(item, "raw")).toBe("send it to james actually sarah");
  });

  it("detects whether cleanup can be undone", () => {
    expect(canUndoCleanup(item)).toBe(true);
    expect(canUndoCleanup({ ...item, finalText: "send it to sarah" })).toBe(false);
  });
});
