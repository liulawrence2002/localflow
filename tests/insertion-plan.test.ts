import { describe, expect, it } from "vitest";
import {
  chooseInsertionMethods,
  createClipboardFallbackPlan,
  DuplicateInsertionGuard,
  shouldRestoreClipboard,
  validateInsertionTarget,
} from "../src/domain/insertionPlan";
import { defaultTarget } from "../src/domain/defaults";

describe("insertion planning", () => {
  it("validates protected and changed targets before insertion", () => {
    expect(validateInsertionTarget(defaultTarget, defaultTarget)).toEqual({ ok: true });
    expect(
      validateInsertionTarget(defaultTarget, {
        ...defaultTarget,
        protectedField: true,
      }),
    ).toEqual({ ok: false, reason: "protected_field" });
    expect(
      validateInsertionTarget(defaultTarget, {
        ...defaultTarget,
        windowTitle: "Another window",
      }),
    ).toEqual({ ok: false, reason: "target_changed" });
  });

  it("orders Windows insertion methods from safest to fallback", () => {
    expect(
      chooseInsertionMethods({ accessibility: true, keyboard: true, clipboard: true }),
    ).toEqual(["accessibility", "keyboard", "clipboard"]);
    expect(
      chooseInsertionMethods({ accessibility: false, keyboard: false, clipboard: true }),
    ).toEqual(["clipboard"]);
  });

  it("plans delayed clipboard restoration", () => {
    const plan = createClipboardFallbackPlan({
      generatedText: "Inserted text",
      previousClipboardText: "old clipboard",
      nowMs: 1000,
      restoreDelayMs: 750,
    });

    expect(plan).toMatchObject({
      generatedText: "Inserted text",
      previousClipboardText: "old clipboard",
      restoreAtMs: 1750,
      canRestore: true,
    });
    expect(shouldRestoreClipboard(plan, 1749)).toBe(false);
    expect(shouldRestoreClipboard(plan, 1750)).toBe(true);
  });

  it("rejects duplicate insertion for the same session and text", () => {
    const guard = new DuplicateInsertionGuard();

    expect(guard.accept("session", "hello")).toBe(true);
    expect(guard.accept("session", "hello")).toBe(false);
    expect(guard.accept("session", "hello again")).toBe(true);
  });
});
