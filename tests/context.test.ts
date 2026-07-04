import { describe, expect, it } from "vitest";
import {
  classifyApplicationCategory,
  contextForCleanup,
  prepareContextSnapshot,
} from "../src/domain/context";
import type { PrivacySettings } from "../src/domain/types";

const privacy: PrivacySettings = {
  historyRetention: "original_and_cleaned",
  deleteAfter: "never",
  activeAppDetection: true,
  accessibilityContext: true,
  selectedTextTransforms: true,
  contextRetention: false,
  deleteAudioAfterProcessing: true,
};

describe("context awareness policy", () => {
  it("never collects app, selected, or cursor context from protected fields", () => {
    const snapshot = prepareContextSnapshot(
      {
        applicationName: "Outlook",
        windowTitle: "Payroll password reset",
        selectedText: "confidential selected text",
        beforeCursor: "sensitive before",
        afterCursor: "sensitive after",
        passwordField: true,
      },
      privacy,
    );

    expect(snapshot).toMatchObject({
      applicationName: "",
      windowTitle: "",
      category: "generic_text_field",
      selectedText: "",
      beforeCursor: "",
      afterCursor: "",
      protectedField: true,
      collected: {
        activeApp: false,
        accessibilityText: false,
        selectedText: false,
      },
    });
  });

  it("applies separate privacy toggles for app, accessibility, and selected text", () => {
    const snapshot = prepareContextSnapshot(
      {
        applicationName: "Slack",
        windowTitle: "Project launch",
        selectedText: "rewrite this sentence",
        beforeCursor: "Please",
        afterCursor: "soon.",
        cursorAtSentenceStart: false,
      },
      {
        ...privacy,
        activeAppDetection: false,
        accessibilityContext: true,
        selectedTextTransforms: false,
      },
    );

    expect(snapshot.applicationName).toBe("");
    expect(snapshot.category).toBe("generic_text_field");
    expect(snapshot.selectedText).toBe("");
    expect(snapshot.beforeCursor).toBe("Please");
    expect(snapshot.afterCursor).toBe("soon.");
    expect(snapshot.collected).toEqual({
      activeApp: false,
      accessibilityText: true,
      selectedText: false,
    });
  });

  it("limits surrounding context to narrow before/after windows", () => {
    const snapshot = prepareContextSnapshot(
      {
        beforeCursor: "0123456789",
        afterCursor: "abcdefghij",
        selectedText: "selected text that is too long",
      },
      privacy,
      {
        beforeCursorChars: 4,
        afterCursorChars: 3,
        selectedTextChars: 8,
      },
    );

    expect(snapshot.beforeCursor).toBe("6789");
    expect(snapshot.afterCursor).toBe("abc");
    expect(snapshot.selectedText).toBe("selected");
  });

  it("classifies application categories without assuming confidentiality", () => {
    expect(
      classifyApplicationCategory({
        applicationName: "Visual Studio Code",
        windowTitle: "localflow - App.tsx",
      }),
    ).toBe("code_editor");
    expect(classifyApplicationCategory({ applicationName: "Windows Terminal" })).toBe("terminal");
    expect(classifyApplicationCategory({ applicationName: "Microsoft Teams" })).toBe(
      "work_messaging",
    );
    expect(classifyApplicationCategory({ url: "https://docs.google.com/document/d/abc" })).toBe(
      "document",
    );
    expect(classifyApplicationCategory({ fieldRole: "search box" })).toBe("search_field");
  });

  it("maps sanitized context to cleanup prompt inputs", () => {
    const snapshot = prepareContextSnapshot(
      {
        applicationName: "Visual Studio Code",
        beforeCursor: "const name =",
        afterCursor: ";",
        codeMode: true,
      },
      privacy,
    );

    expect(contextForCleanup(snapshot)).toEqual({
      appCategory: "code_editor",
      beforeCursor: "const name =",
      afterCursor: ";",
      codeMode: true,
    });
  });

  it("rejects invalid context limits", () => {
    expect(() =>
      prepareContextSnapshot({}, privacy, {
        beforeCursorChars: -1,
        afterCursorChars: 0,
        selectedTextChars: 0,
      }),
    ).toThrow(/Context limits/);
  });
});
