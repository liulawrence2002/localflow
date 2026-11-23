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
        headline: "Thinking. The microphone is open but ignored.",
        buttonLabel: `Interrupt${hotkey}`,
        buttonDisabled: false,
        microphoneOpen: true,
      };
    case "speaking":
      return {
        headline: "Speaking. The microphone is open but ignored.",
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
