import { describe, expect, it } from "vitest";
import { TranscriptStabilizer } from "../src/domain/transcriptStabilizer";

describe("TranscriptStabilizer", () => {
  it("commits only stable prefixes", () => {
    const stabilizer = new TranscriptStabilizer();

    expect(stabilizer.update("hello world this").newCommit).toBe("");
    expect(stabilizer.update("hello world there").newCommit).toBe("hello world");
    expect(stabilizer.update("hello world there friend").newCommit).toBe("there");
  });

  it("finalizes uncommitted text without duplication", () => {
    const stabilizer = new TranscriptStabilizer();
    stabilizer.update("deploy friday after");
    stabilizer.update("deploy friday after tests");

    const final = stabilizer.finalize("deploy friday after tests");

    expect(final.committedText).toBe("deploy friday after tests");
    expect(final.newCommit).toBe("tests");
  });
});
