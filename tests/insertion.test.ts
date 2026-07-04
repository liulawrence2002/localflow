import { describe, expect, it } from "vitest";
import { composeInsertion } from "../src/domain/insertion";

describe("composeInsertion", () => {
  it("adds a leading space while continuing a sentence", () => {
    expect(
      composeInsertion("Review this.", {
        beforeCursor: "Please",
        afterCursor: "",
        atSentenceStart: false,
        codeMode: false,
      }),
    ).toBe(" review this.");
  });

  it("does not add a space before punctuation", () => {
    expect(
      composeInsertion(",", {
        beforeCursor: "Hello",
        afterCursor: "",
        atSentenceStart: false,
        codeMode: false,
      }),
    ).toBe(",");
  });
});
