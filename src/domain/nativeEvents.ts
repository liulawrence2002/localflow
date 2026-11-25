/**
 * Events the native layer sends to the settings window, checked before use, and what the
 * window makes of them. None of these payloads carries anything that was said or read.
 */

import { conversationPhases, type ConversationPhase } from "./conversation";

export const dictationEventName = "localflow://native-dictation";
export const conversationEventName = "localflow://conversation";
export const ttsEventName = "localflow://tts";

/** A phase change on the overlay channel (dictation, and the phases others borrow). */
export interface DictationPhaseEvent {
  phase: string;
  message: string;
}

/**
 * A phase event from `localflow://native-dictation`, or `null` for an audio-feature event
 * (about 36 a second while listening) or a payload that is not one.
 */
export function parseDictationEvent(payload: unknown): DictationPhaseEvent | null {
  if (!isRecord(payload) || typeof payload.phase !== "string") {
    return null;
  }
  const hasFeatures = ["level", "pitch", "brightness", "bands"].some(
    (key) => payload[key] !== undefined && payload[key] !== null,
  );
  if (hasFeatures) {
    return null;
  }
  return {
    phase: payload.phase,
    message: typeof payload.message === "string" ? payload.message : "",
  };
}

export interface ConversationEventPayload {
  phase: ConversationPhase;
  reason: string | null;
  message: string;
}

export function parseConversationEvent(payload: unknown): ConversationEventPayload | null {
  if (!isRecord(payload) || !isConversationPhase(payload.phase)) {
    return null;
  }
  return {
    phase: payload.phase,
    reason: typeof payload.reason === "string" ? payload.reason : null,
    message: typeof payload.message === "string" ? payload.message : "",
  };
}

export type TtsPhase = "synthesizing" | "speaking" | "idle" | "error";

export interface TtsEventPayload {
  phase: TtsPhase;
  message: string;
}

export function parseTtsEvent(payload: unknown): TtsEventPayload | null {
  if (!isRecord(payload)) {
    return null;
  }
  const phase = payload.phase;
  if (phase !== "synthesizing" && phase !== "speaking" && phase !== "idle" && phase !== "error") {
    return null;
  }
  return { phase, message: typeof payload.message === "string" ? payload.message : "" };
}

export type ProblemAction =
  /** Offer the Copy last transcript button. */
  | { kind: "copy_transcript" }
  /** A sentence saying what to try. */
  | { kind: "hint"; text: string };

/** A problem the native layer reported, as the Home banner shows it. */
export interface NativeProblem {
  message: string;
  atMs: number;
  action: ProblemAction | null;
}

/** Turn an overlay error message into a banner: the message, when, and what to do next. */
export function describeNativeProblem(message: string, atMs: number): NativeProblem {
  const text = message.trim() || "LocalFlow could not finish what it was doing.";
  return { message: text, atMs, action: suggestAction(text) };
}

function suggestAction(message: string): ProblemAction | null {
  const lower = message.toLowerCase();
  if (lower.includes("copy last transcript")) {
    return { kind: "copy_transcript" };
  }
  if (lower.includes("microphone") || lower.includes("input device") || lower.includes("silent")) {
    return {
      kind: "hint",
      text:
        "Check the default microphone in Windows Settings > System > Sound, and that desktop " +
        "apps may use the microphone (Privacy & security > Microphone).",
    };
  }
  if (lower.includes("too short")) {
    return { kind: "hint", text: "Speak for a moment longer before pausing, then try again." };
  }
  if (
    lower.includes("recognised") ||
    lower.includes("recognized") ||
    lower.includes("no transcript")
  ) {
    return {
      kind: "hint",
      text: "Speak a little closer to the microphone, or pause a moment longer before stopping.",
    };
  }
  if (lower.includes("hotkey")) {
    return {
      kind: "hint",
      text: "Another program may hold the hotkey. The Hotkeys screen shows which ones work.",
    };
  }
  // A message that already names its fix ("Run scripts\...", "See Diagnostics") needs no more.
  if (lower.includes("run scripts") || lower.includes("see ") || lower.includes("try again")) {
    return null;
  }
  if (lower.includes("transcri") || lower.includes("whisper") || lower.includes("speech model")) {
    return {
      kind: "hint",
      text:
        "Try again closer to the microphone. If it keeps failing, check the speech model file " +
        "on the Models screen.",
    };
  }
  return null;
}

/** What the Recovery panel shows for read-aloud of the last transcript. */
export interface ReadAloudView {
  /** A sentence about what is happening, or `null` for nothing to say. */
  notice: string | null;
  /** "alert" for a failure, "status" otherwise. */
  tone: "status" | "alert";
  /** Whether Stop does anything now. */
  stopEnabled: boolean;
}

export function describeReadAloud(event: TtsEventPayload | null): ReadAloudView {
  if (!event) {
    return { notice: null, tone: "status", stopEnabled: false };
  }
  switch (event.phase) {
    case "synthesizing":
      return {
        notice:
          "Preparing speech with the Fish S2 voice. The first reading after a while can take " +
          "a minute while the voice loads.",
        tone: "status",
        stopEnabled: true,
      };
    case "speaking":
      return { notice: "Reading the last transcript aloud.", tone: "status", stopEnabled: true };
    case "idle":
      return {
        notice: event.message === "Stopped" ? "Stopped reading." : "Finished reading.",
        tone: "status",
        stopEnabled: false,
      };
    case "error":
      return {
        notice: event.message.trim() || "The transcript could not be read aloud.",
        tone: "alert",
        stopEnabled: false,
      };
  }
}

function isConversationPhase(value: unknown): value is ConversationPhase {
  return typeof value === "string" && (conversationPhases as readonly string[]).includes(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
