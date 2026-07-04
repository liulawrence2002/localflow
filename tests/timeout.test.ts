import { afterEach, describe, expect, it, vi } from "vitest";
import { runWithTimeout } from "../src/domain/timeout";

describe("timeout guard", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("resolves when the operation completes in time", async () => {
    await expect(runWithTimeout(Promise.resolve("ok"), 100, "ASR")).resolves.toBe("ok");
  });

  it("rejects when the operation exceeds the timeout", async () => {
    vi.useFakeTimers();

    const result = runWithTimeout(new Promise<string>(() => undefined), 50, "ASR");
    const assertion = expect(result).rejects.toThrow("ASR timed out after 50 ms.");
    await vi.advanceTimersByTimeAsync(50);

    await assertion;
  });
});
