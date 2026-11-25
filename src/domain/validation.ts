/** Checks for what the user types into settings fields, kept apart from the UI so they are tested. */

export type BoundedIntegerResult =
  | {
      ok: true;
      value: number;
      /** True when the typed number was rounded or moved into range. */
      adjusted: boolean;
    }
  | { ok: false };

/**
 * Read a whole number typed into a field and move it into `min..max`. Empty or non-numeric
 * text is refused, so the caller can keep the value there was instead of saving nonsense.
 */
export function parseBoundedInteger(text: string, min: number, max: number): BoundedIntegerResult {
  const trimmed = text.trim();
  if (!/^[+-]?(\d+(\.\d*)?|\.\d+)$/.test(trimmed)) {
    return { ok: false };
  }
  const parsed = Number(trimmed);
  if (!Number.isFinite(parsed)) {
    return { ok: false };
  }
  const value = Math.min(max, Math.max(min, Math.round(parsed)));
  return { ok: true, value, adjusted: value !== parsed };
}

/** The sentence shown under a number field when what was typed was changed or refused. */
export function boundedIntegerMessage(
  result: BoundedIntegerResult,
  min: number,
  max: number,
  unit: string,
): string | null {
  const range = `${min} to ${max}${unit ? ` ${unit}` : ""}`;
  if (!result.ok) {
    return `Enter a whole number from ${range}.`;
  }
  if (result.adjusted) {
    return `Set to ${result.value}${unit ? ` ${unit}` : ""}: the allowed range is ${range}.`;
  }
  return null;
}

/** Trimmed text, or `null` when nothing but spaces was typed. */
export function requiredText(text: string): string | null {
  const trimmed = text.trim();
  return trimmed ? trimmed : null;
}
