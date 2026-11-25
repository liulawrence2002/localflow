import {
  Activity,
  BookOpenText,
  Bot,
  Braces,
  ClipboardList,
  Gauge,
  Info,
  Keyboard,
  Mic,
  RotateCcw,
  Shield,
} from "lucide-react";

export type ScreenId =
  | "home"
  | "models"
  | "dictation"
  | "hotkeys"
  | "dictionary"
  | "replacements"
  | "snippets"
  | "agents"
  | "privacy"
  | "diagnostics"
  | "about";

export interface ScreenEntry {
  id: ScreenId;
  label: string;
  icon: typeof Activity;
  /** One line under the screen's title. */
  description: string;
}

/**
 * The settings window's sections, in navigation order. Styles and History are not listed:
 * the native app does not use them yet, and a setting that does nothing is not shown.
 */
export const screens: readonly ScreenEntry[] = [
  { id: "home", label: "Home", icon: Activity, description: "What LocalFlow is doing now." },
  {
    id: "models",
    label: "Models",
    icon: Braces,
    description: "The speech model and the local language model.",
  },
  {
    id: "dictation",
    label: "Dictation",
    icon: Mic,
    description: "How dictation starts and stops.",
  },
  {
    id: "hotkeys",
    label: "Hotkeys",
    icon: Keyboard,
    description: "The keys LocalFlow listens for, and whether each one works.",
  },
  {
    id: "dictionary",
    label: "Dictionary",
    icon: BookOpenText,
    description: "Words and names the speech recogniser should expect.",
  },
  {
    id: "replacements",
    label: "Replacements",
    icon: RotateCcw,
    description: "Phrases that are always written differently.",
  },
  {
    id: "snippets",
    label: "Snippets",
    icon: ClipboardList,
    description: "Short phrases that expand into longer text.",
  },
  {
    id: "agents",
    label: "Coding agents",
    icon: Bot,
    description: "Projects you can name in a voice command, and the agents found.",
  },
  {
    id: "privacy",
    label: "Privacy",
    icon: Shield,
    description: "What LocalFlow keeps, and what leaves this computer.",
  },
  {
    id: "diagnostics",
    label: "Diagnostics",
    icon: Gauge,
    description: "Engine, timings and hotkey health, and an export for troubleshooting.",
  },
  { id: "about", label: "About", icon: Info, description: "Version and licence." },
];

export function findScreen(id: ScreenId): ScreenEntry {
  return screens.find((screen) => screen.id === id) ?? (screens[0] as ScreenEntry);
}
