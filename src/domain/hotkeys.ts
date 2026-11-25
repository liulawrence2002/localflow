/**
 * The real global hotkeys (registered in `src-tauri/src/hotkeys/mod.rs`) and whether each one
 * could be registered. They cannot be changed yet, so the settings window only shows them.
 */

import type { DiagnosticMetric } from "./types";

export const dictationHotkey = "Ctrl+Alt+Space";
export const dictationFallbackHotkey = "Ctrl+Alt+Shift+Space";
export const conversationHotkeys = ["Ctrl+Alt+T", "Ctrl+Alt+Shift+T"] as const;
export const readingHotkeys = ["Ctrl+Alt+R", "Ctrl+Alt+Shift+R"] as const;

export type HotkeyTone = "ok" | "warning" | "error" | "neutral";

export interface HotkeyRow {
  keys: string;
  action: string;
  status: string;
  tone: HotkeyTone;
}

export interface HotkeySources {
  /** `null` outside the desktop app. */
  diagnostics: DiagnosticMetric[] | null;
  /** The registered conversation hotkey; `undefined` while not known yet. */
  conversationHotkey: string | null | undefined;
  readingHotkey: string | null | undefined;
}

/** The diagnostics row the native layer writes after registering the dictation hotkeys. */
const registeredRowLabel = "Registered desktop hotkeys";

function registeredDictationHotkeys(diagnostics: DiagnosticMetric[] | null): string[] | null {
  const row = diagnostics?.find((metric) => metric.label === registeredRowLabel);
  if (!row) {
    return null;
  }
  return row.value
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value.startsWith("Ctrl"));
}

function dictationStatus(
  registered: string[] | null,
  hotkey: string,
): Pick<HotkeyRow, "status" | "tone"> {
  if (registered === null) {
    return { status: "Unknown", tone: "neutral" };
  }
  return registered.includes(hotkey)
    ? { status: "Registered", tone: "ok" }
    : { status: "Not registered: another program may hold it", tone: "warning" };
}

function pairStatus(
  registered: string | null | undefined,
  preferred: string,
  fallback: string,
): Pick<HotkeyRow, "status" | "tone"> {
  if (registered === undefined) {
    return { status: "Unknown", tone: "neutral" };
  }
  if (registered === preferred) {
    return { status: "Registered", tone: "ok" };
  }
  if (registered === fallback) {
    return { status: `Another program holds it; ${fallback} works instead`, tone: "warning" };
  }
  return { status: `Not registered, and neither is ${fallback}`, tone: "error" };
}

/** The hotkey table: each key, what it does, and whether it works now. */
export function describeHotkeys(sources: HotkeySources): HotkeyRow[] {
  const registered = registeredDictationHotkeys(sources.diagnostics);
  const [conversation, conversationFallback] = conversationHotkeys;
  const [reading, readingFallback] = readingHotkeys;
  return [
    {
      keys: dictationHotkey,
      action:
        "Dictation. Tap it, speak, and pause: it stops after the end-of-speech wait. Or hold " +
        "it while you speak and let go when you finish. Press again to stop early.",
      ...dictationStatus(registered, dictationHotkey),
    },
    {
      keys: dictationFallbackHotkey,
      action: `Dictation, the same as ${dictationHotkey}. Both are registered, so this works when another program holds that one.`,
      ...dictationStatus(registered, dictationFallbackHotkey),
    },
    {
      keys: conversation,
      action:
        "Starts a conversation. While the assistant is thinking or speaking, interrupts it; " +
        "otherwise ends the conversation.",
      ...pairStatus(sources.conversationHotkey, conversation, conversationFallback),
    },
    {
      keys: reading,
      action: "Reads the focused window aloud, word for word. Press again to stop.",
      ...pairStatus(sources.readingHotkey, reading, readingFallback),
    },
    {
      keys: "Escape",
      action:
        "Cancels a dictation while it is recording; nothing is pasted. It is registered only " +
        "then, so Escape works as usual the rest of the time.",
      status: "Only while recording",
      tone: "neutral",
    },
  ];
}
