import type { PrivacySettings, SessionHistoryItem } from "./types";

export function applyHistoryRetention(
  history: SessionHistoryItem[],
  settings: PrivacySettings,
  now: Date,
): SessionHistoryItem[] {
  if (settings.historyRetention === "off") {
    return [];
  }

  const cutoffMs = retentionCutoffMs(settings.deleteAfter, now);

  return history
    .filter((item) => cutoffMs === undefined || Date.parse(item.completedAt) >= cutoffMs)
    .map((item) => {
      if (settings.historyRetention === "transcript_only") {
        return {
          ...item,
          rawTranscript: "",
          deterministicText: undefined,
          refinedText: undefined,
        };
      }

      return item;
    });
}

export function shouldRetainContext(settings: PrivacySettings): boolean {
  return (
    settings.contextRetention &&
    (settings.activeAppDetection ||
      settings.accessibilityContext ||
      settings.selectedTextTransforms)
  );
}

export function redactSensitiveText(text: string): string {
  const trimmed = text.trim();
  return trimmed ? `[redacted:${trimmed.length} chars]` : "[empty]";
}

function retentionCutoffMs(
  deleteAfter: PrivacySettings["deleteAfter"],
  now: Date,
): number | undefined {
  if (deleteAfter === "never") {
    return undefined;
  }

  const ageMs = deleteAfter === "24h" ? 24 * 60 * 60 * 1000 : 7 * 24 * 60 * 60 * 1000;
  return now.getTime() - ageMs;
}
