import { describe, expect, it } from "vitest";
import {
  cleanFolderInput,
  describeAgent,
  examplePhrases,
  formatSpokenNames,
  parseSpokenNames,
  quickFolderProblem,
  spokenNameKey,
  type AgentStatus,
} from "../src/domain/codingAgents";
import { defaultSettings } from "../src/domain/defaults";
import { normalizeSettings } from "../src/domain/settings";
import type { LocalFlowSettings } from "../src/domain/types";

function status(patch: Partial<AgentStatus>): AgentStatus {
  return {
    agent: "claudeCode",
    name: "Claude Code",
    found: true,
    path: "C:\\npm\\claude.exe",
    version: "2.1.170",
    canOpen: true,
    canAsk: true,
    detail: "Can open in Windows Terminal and answer questions.",
    ...patch,
  };
}

describe("coding agent folders", () => {
  it("strips the quotes Explorer's Copy as path adds", () => {
    expect(cleanFolderInput('  "C:\\work\\app"  ')).toBe("C:\\work\\app");
    expect(cleanFolderInput("C:\\work\\app")).toBe("C:\\work\\app");
  });

  it("explains why Windows Terminal characters are refused", () => {
    expect(quickFolderProblem("C:\\work\\a;b")).toMatch(/new command/);
    expect(quickFolderProblem("C:\\work\\100%")).toMatch(/environment variable/);
    expect(quickFolderProblem('C:\\work\\a"b')).toMatch(/quotes/);
  });

  it("asks for a full path", () => {
    expect(quickFolderProblem("")).toMatch(/full path/);
    expect(quickFolderProblem("work\\app")).toMatch(/drive letter/);
    expect(quickFolderProblem("\\\\?\\C:\\work")).toMatch(/drive letter/);
    expect(quickFolderProblem("C:\\work\\app")).toBeNull();
    expect(quickFolderProblem("D:/work/app")).toBeNull();
    expect(quickFolderProblem("\\\\server\\share\\app")).toBeNull();
  });
});

describe("spoken names", () => {
  it("splits on commas and lines and drops repeats as they would be heard", () => {
    expect(parseSpokenNames(" local  flow, LocalFlow,\nthe app ,, ")).toEqual([
      "local flow",
      "the app",
    ]);
    expect(formatSpokenNames(["local flow", "the app"])).toBe("local flow, the app");
  });

  it("compares names without spaces or punctuation", () => {
    expect(spokenNameKey("Local-Flow 2")).toBe("localflow2");
    expect(parseSpokenNames("!!!")).toEqual([]);
  });

  it("uses the first project's name in the examples", () => {
    expect(examplePhrases([])[0]).toBe("Open Claude Code in my project");
    expect(
      examplePhrases([{ id: "p", path: "C:\\work\\localflow", spokenNames: ["localflow"] }]),
    ).toEqual([
      "Open Claude Code in localflow",
      "Ask Claude what the tests in localflow check",
      "Open localflow in Cursor",
    ]);
  });
});

describe("agent detection view", () => {
  it("shows a fully working Claude Code with its version and path", () => {
    expect(describeAgent(status({}))).toEqual({
      name: "Claude Code",
      state: "Found",
      tone: "ok",
      location: "2.1.170 at C:\\npm\\claude.exe",
      detail: "Can open in Windows Terminal and answer questions.",
    });
  });

  it("warns when only part of it works", () => {
    expect(describeAgent(status({ canOpen: false })).tone).toBe("warning");
  });

  it("marks missing and not-yet-supported agents", () => {
    expect(describeAgent(status({ found: false, path: null, version: null }))).toMatchObject({
      state: "Not found",
      tone: "blocked",
      location: null,
    });
    expect(
      describeAgent(
        status({ agent: "codex", name: "Codex", found: false, canOpen: false, canAsk: false }),
      ),
    ).toMatchObject({ state: "Not set up yet", tone: "blocked" });
    expect(
      describeAgent(status({ agent: "cursor", name: "Cursor", version: null, canAsk: false })),
    ).toMatchObject({ state: "Found", tone: "ok", location: "C:\\npm\\claude.exe" });
  });
});

describe("settings with coding agents", () => {
  it("defaults to no projects", () => {
    expect(defaultSettings.codingAgents.projects).toEqual([]);
  });

  it("fills in the section for settings saved before it existed", () => {
    const older = { ...defaultSettings } as Partial<LocalFlowSettings>;
    delete older.codingAgents;
    expect(normalizeSettings(older as LocalFlowSettings).codingAgents).toEqual({ projects: [] });
  });

  it("keeps projects through normalisation", () => {
    const settings: LocalFlowSettings = {
      ...defaultSettings,
      codingAgents: {
        projects: [{ id: "p1", path: "C:\\work\\app", spokenNames: ["app"] }],
      },
    };
    expect(normalizeSettings(settings).codingAgents.projects).toEqual([
      { id: "p1", path: "C:\\work\\app", spokenNames: ["app"] },
    ]);
  });
});
