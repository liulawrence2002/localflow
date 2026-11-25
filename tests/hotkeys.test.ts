import { describe, expect, it } from "vitest";
import { describeHotkeys } from "../src/domain/hotkeys";

const registered = [
  {
    label: "Registered desktop hotkeys",
    value: "Ctrl+Alt+Space, Ctrl+Alt+Shift+Space",
    status: "ok" as const,
  },
];

describe("describeHotkeys", () => {
  it("lists the real hotkeys in order", () => {
    const rows = describeHotkeys({
      diagnostics: registered,
      conversationHotkey: "Ctrl+Alt+T",
      readingHotkey: "Ctrl+Alt+R",
    });

    expect(rows.map((row) => row.keys)).toEqual([
      "Ctrl+Alt+Space",
      "Ctrl+Alt+Shift+Space",
      "Ctrl+Alt+T",
      "Ctrl+Alt+R",
      "Escape",
    ]);
    expect(rows[0]?.action).toContain("Tap");
    expect(rows[0]?.action).toContain("hold");
    expect(rows.slice(0, 4).every((row) => row.status === "Registered")).toBe(true);
  });

  it("says when the Shift variant is used because another program holds a hotkey", () => {
    const rows = describeHotkeys({
      diagnostics: [
        { label: "Registered desktop hotkeys", value: "Ctrl+Alt+Shift+Space", status: "ok" },
      ],
      conversationHotkey: "Ctrl+Alt+Shift+T",
      readingHotkey: null,
    });

    expect(rows[0]).toMatchObject({ tone: "warning" });
    expect(rows[1]).toMatchObject({ status: "Registered", tone: "ok" });
    expect(rows[2]?.status).toContain("Ctrl+Alt+Shift+T works instead");
    expect(rows[3]).toMatchObject({ tone: "error" });
  });

  it("says Unknown rather than guessing outside the desktop app", () => {
    const rows = describeHotkeys({
      diagnostics: null,
      conversationHotkey: undefined,
      readingHotkey: undefined,
    });

    expect(rows.slice(0, 4).every((row) => row.status === "Unknown")).toBe(true);
  });
});
