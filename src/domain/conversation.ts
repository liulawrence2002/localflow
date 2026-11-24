import type { OllamaConnectionStatus } from "./ollama";

/** Phases of conversation mode, as the native layer reports them. */
export type ConversationPhase =
  "off" | "starting" | "listening" | "user_speaking" | "thinking" | "speaking";

export interface ConversationStatus {
  phase: ConversationPhase;
  /** Whether everything conversation mode needs is installed. */
  available: boolean;
  /** What is missing, or what will be used. */
  detail: string;
  /** The hotkey that toggles conversation mode, when one could be registered. */
  hotkey: string | null;
}

/** Reading the focused window aloud, as the native layer reports it. */
export interface ScreenReadingStatus {
  available: boolean;
  detail: string;
  hotkey: string | null;
  reading: boolean;
}

/** One sentence telling the user how to have a window read aloud. */
export function describeScreenReading(status: ScreenReadingStatus | null): string {
  if (!status) {
    return "Reading a window aloud needs the desktop app.";
  }
  if (status.reading) {
    return status.hotkey
      ? `Reading a window aloud. Press ${status.hotkey} to stop.`
      : "Reading a window aloud.";
  }
  if (status.hotkey) {
    return (
      `Press ${status.hotkey} over any window to hear it read word for word, ` +
      'or say "read this page" in a conversation.'
    );
  }
  return 'Say "read this page" in a conversation to hear the focused window read aloud.';
}

export interface ConversationView {
  /** One line saying what is happening now. */
  headline: string;
  /** What the button and the hotkey will do if used now. */
  buttonLabel: string;
  buttonDisabled: boolean;
  /** True while the microphone is open. */
  microphoneOpen: boolean;
}

/**
 * What the settings window shows for a conversation status. The button does what the hotkey
 * does: start when off, interrupt while the assistant is thinking or speaking, end otherwise.
 */
export function describeConversation(status: ConversationStatus | null): ConversationView {
  if (!status) {
    return {
      headline: "Conversation mode needs the desktop app.",
      buttonLabel: "Start conversation",
      buttonDisabled: true,
      microphoneOpen: false,
    };
  }

  const hotkey = status.hotkey ? ` (${status.hotkey})` : "";
  switch (status.phase) {
    case "off":
      return {
        headline: status.available
          ? "Off. The microphone is closed."
          : "Not available yet. The microphone is closed.",
        buttonLabel: `Start conversation${hotkey}`,
        buttonDisabled: !status.available,
        microphoneOpen: false,
      };
    case "starting":
      return {
        headline: "Starting: opening the microphone and loading models.",
        buttonLabel: `Cancel${hotkey}`,
        buttonDisabled: false,
        microphoneOpen: false,
      };
    case "listening":
    case "user_speaking":
      return {
        headline: "Listening. The microphone is open.",
        buttonLabel: `End conversation${hotkey}`,
        buttonDisabled: false,
        microphoneOpen: true,
      };
    case "thinking":
      return {
        headline: "Thinking. The microphone is open: keep talking to add to it.",
        buttonLabel: `Interrupt${hotkey}`,
        buttonDisabled: false,
        microphoneOpen: true,
      };
    case "speaking":
      return {
        headline: "Speaking. The microphone is open.",
        buttonLabel: `Interrupt${hotkey}`,
        buttonDisabled: false,
        microphoneOpen: true,
      };
  }
}

/** Shape the native list of installed local models like the browser-side Ollama check. */
export function localModelsToStatus(names: string[], baseUrl: string): OllamaConnectionStatus {
  return {
    ok: true,
    baseUrl,
    models: names.map((name) => ({ name, model: name })),
  };
}
