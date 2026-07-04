import { describe, expect, it } from "vitest";
import { describeLastTranscript } from "../src/domain/recovery";

describe("describeLastTranscript", () => {
  it("reports no transcript for empty or missing input", () => {
    for (const value of [null, undefined, ""]) {
      const state = describeLastTranscript(value);
      expect(state.available).toBe(false);
      expect(state.charCount).toBe(0);
      expect(state.preview).toBeNull();
    }
  });

  it("reports availability and length without revealing content by default", () => {
    const state = describeLastTranscript("hello world");
    expect(state.available).toBe(true);
    expect(state.charCount).toBe(11);
    expect(state.preview).toBeNull();
  });

  it("reveals content only when explicitly requested", () => {
    const state = describeLastTranscript("secret note", true);
    expect(state.available).toBe(true);
    expect(state.preview).toBe("secret note");
  });
});
