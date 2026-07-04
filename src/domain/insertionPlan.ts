import type { TargetSnapshot } from "./types";

export type InsertionMethod = "accessibility" | "keyboard" | "clipboard";

export interface TargetValidation {
  ok: boolean;
  reason?: "protected_field" | "target_changed";
}

export interface ClipboardFallbackPlan {
  generatedText: string;
  previousClipboardText?: string;
  restoreAtMs: number;
  canRestore: boolean;
}

export class DuplicateInsertionGuard {
  private readonly inserted = new Set<string>();

  accept(sessionId: string, text: string): boolean {
    const key = `${sessionId}:${text}`;
    if (this.inserted.has(key)) {
      return false;
    }
    this.inserted.add(key);
    return true;
  }

  reset(sessionId: string): void {
    for (const key of [...this.inserted]) {
      if (key.startsWith(`${sessionId}:`)) {
        this.inserted.delete(key);
      }
    }
  }
}

export function validateInsertionTarget(
  original: TargetSnapshot,
  current: TargetSnapshot,
): TargetValidation {
  if (original.protectedField || current.protectedField) {
    return { ok: false, reason: "protected_field" };
  }

  if (
    original.applicationName !== current.applicationName ||
    original.windowTitle !== current.windowTitle
  ) {
    return { ok: false, reason: "target_changed" };
  }

  return { ok: true };
}

export function chooseInsertionMethods(capabilities: {
  accessibility: boolean;
  keyboard: boolean;
  clipboard: boolean;
}): InsertionMethod[] {
  const methods: InsertionMethod[] = [];
  if (capabilities.accessibility) {
    methods.push("accessibility");
  }
  if (capabilities.keyboard) {
    methods.push("keyboard");
  }
  if (capabilities.clipboard) {
    methods.push("clipboard");
  }
  return methods;
}

export function createClipboardFallbackPlan({
  generatedText,
  previousClipboardText,
  nowMs,
  restoreDelayMs,
}: {
  generatedText: string;
  previousClipboardText?: string;
  nowMs: number;
  restoreDelayMs: number;
}): ClipboardFallbackPlan {
  return {
    generatedText,
    previousClipboardText,
    restoreAtMs: nowMs + restoreDelayMs,
    canRestore: previousClipboardText !== undefined,
  };
}

export function shouldRestoreClipboard(plan: ClipboardFallbackPlan, nowMs: number): boolean {
  return plan.canRestore && nowMs >= plan.restoreAtMs;
}
