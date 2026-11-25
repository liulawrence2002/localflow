/** The Home "Setup" checklist, built from the native `readiness_status` command. */

export interface ReadinessItem {
  id: string;
  /** What it is, for example "Speech model (Parakeet)". */
  label: string;
  ready: boolean;
  /** Whether LocalFlow needs it; optional items only improve something. */
  required: boolean;
  /** What to run or do when it is not ready, for example "Run scripts\Get-AsrModel.ps1". */
  fix: string | null;
}

/** The items in a `readiness_status` reply, or `null` when the reply is not a list of them. */
export function parseReadiness(payload: unknown): ReadinessItem[] | null {
  if (!Array.isArray(payload)) {
    return null;
  }
  const items: ReadinessItem[] = [];
  for (const entry of payload) {
    if (
      typeof entry !== "object" ||
      entry === null ||
      typeof entry.id !== "string" ||
      typeof entry.label !== "string" ||
      typeof entry.ready !== "boolean"
    ) {
      return null;
    }
    items.push({
      id: entry.id,
      label: entry.label,
      ready: entry.ready,
      required: entry.required !== false,
      fix: typeof entry.fix === "string" && entry.fix.trim() ? entry.fix.trim() : null,
    });
  }
  return items;
}

export interface SetupChecklist {
  /** Shown only while something required is missing. */
  visible: boolean;
  /** Required items first, then optional ones; within each, what is missing first. */
  items: ReadinessItem[];
  missingRequired: number;
}

export function buildSetupChecklist(items: ReadinessItem[] | null): SetupChecklist {
  if (!items || items.length === 0) {
    return { visible: false, items: [], missingRequired: 0 };
  }
  const missingRequired = items.filter((item) => item.required && !item.ready).length;
  const rank = (item: ReadinessItem) => (item.required ? 0 : 2) + (item.ready ? 1 : 0);
  const sorted = items
    .map((item, index) => ({ item, index }))
    .sort((a, b) => rank(a.item) - rank(b.item) || a.index - b.index)
    .map(({ item }) => item);
  return { visible: missingRequired > 0, items: sorted, missingRequired };
}
