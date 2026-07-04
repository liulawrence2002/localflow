// Pure helpers for the last-transcript recovery UI. The transcript itself is sensitive,
// so by default the UI shows only whether one is available and its length — never the
// content — unless the user explicitly reveals it (spec: no transcript display by default).

export interface RecoveryState {
  /** Whether a recoverable transcript exists. */
  available: boolean;
  /** Character count of the transcript (0 when none). Safe to display. */
  charCount: number;
  /** The transcript text, only when the caller explicitly opts to reveal it; else null. */
  preview: string | null;
}

export function describeLastTranscript(
  transcript: string | null | undefined,
  reveal = false,
): RecoveryState {
  const text = typeof transcript === "string" ? transcript : "";
  const available = text.length > 0;

  return {
    available,
    charCount: text.length,
    preview: available && reveal ? text : null,
  };
}
