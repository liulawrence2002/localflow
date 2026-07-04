import type { SessionHistoryItem } from "./types";

export function restorePreCleanupText(
  item: SessionHistoryItem,
  preference: "deterministic" | "raw" = "deterministic",
): string {
  if (preference === "raw") {
    return item.rawTranscript;
  }

  return item.deterministicText || item.rawTranscript;
}

export function canUndoCleanup(item?: SessionHistoryItem): boolean {
  if (!item) {
    return false;
  }

  const restored = restorePreCleanupText(item);
  return Boolean(restored) && restored !== item.finalText;
}
