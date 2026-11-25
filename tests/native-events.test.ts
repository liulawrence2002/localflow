import { describe, expect, it } from "vitest";
import {
  describeNativeProblem,
  describeReadAloud,
  parseConversationEvent,
  parseDictationEvent,
  parseTtsEvent,
} from "../src/domain/nativeEvents";
import { buildSetupChecklist, parseReadiness } from "../src/domain/readiness";

describe("native event payloads", () => {
  it("keeps dictation phase events and drops audio-feature events", () => {
    expect(
      parseDictationEvent({ phase: "error", message: "No speech was recognised.", level: null }),
    ).toEqual({ phase: "error", message: "No speech was recognised." });
    expect(
      parseDictationEvent({ phase: "listening", message: "Listening", level: 0.4, bands: [1, 2] }),
    ).toBeNull();
    expect(parseDictationEvent("error")).toBeNull();
    expect(parseDictationEvent({ message: "no phase" })).toBeNull();
  });

  it("reads conversation and read-aloud events, refusing unknown phases", () => {
    expect(
      parseConversationEvent({ phase: "off", reason: "start_failed", message: "No microphone." }),
    ).toEqual({ phase: "off", reason: "start_failed", message: "No microphone." });
    expect(parseConversationEvent({ phase: "off", reason: null, message: "Ended" })?.reason).toBe(
      null,
    );
    expect(parseConversationEvent({ phase: "napping" })).toBeNull();
    expect(parseTtsEvent({ phase: "speaking", message: "Reading aloud" })).toEqual({
      phase: "speaking",
      message: "Reading aloud",
    });
    expect(parseTtsEvent({ phase: "singing" })).toBeNull();
  });
});

describe("describeNativeProblem", () => {
  const at = new Date(2026, 10, 25, 14, 3).getTime();

  it("offers Copy last transcript when the paste was skipped", () => {
    const problem = describeNativeProblem(
      'Focus changed before insertion. The transcript was not pasted; use "Copy last transcript" to recover it.',
      at,
    );

    expect(problem.action).toEqual({ kind: "copy_transcript" });
    expect(problem.atMs).toBe(at);
  });

  it("points at Windows sound settings for a microphone problem", () => {
    const problem = describeNativeProblem("No default microphone is available.", at);

    expect(problem.action).toMatchObject({ kind: "hint" });
    expect(problem.action?.kind === "hint" && problem.action.text).toContain("Sound");
  });

  it("adds nothing when the message already says what to do", () => {
    expect(
      describeNativeProblem(
        "No speech model is installed (4 locations checked). Run scripts\\Get-AsrModel.ps1, or reinstall LocalFlow.",
        at,
      ).action,
    ).toBeNull();
  });

  it("never shows an empty banner", () => {
    expect(describeNativeProblem("   ", at).message).toContain("LocalFlow");
  });
});

describe("describeReadAloud", () => {
  it("says what read-aloud is doing and enables Stop only while it runs", () => {
    expect(describeReadAloud(null)).toEqual({ notice: null, tone: "status", stopEnabled: false });
    expect(describeReadAloud({ phase: "synthesizing", message: "Preparing speech" })).toMatchObject(
      {
        tone: "status",
        stopEnabled: true,
      },
    );
    expect(describeReadAloud({ phase: "speaking", message: "Reading aloud" }).stopEnabled).toBe(
      true,
    );
    expect(describeReadAloud({ phase: "idle", message: "Finished" })).toMatchObject({
      notice: "Finished reading.",
      stopEnabled: false,
    });
    expect(describeReadAloud({ phase: "idle", message: "Stopped" }).notice).toBe(
      "Stopped reading.",
    );
  });

  it("shows a failure as an alert, never as a stuck 'Preparing'", () => {
    const view = describeReadAloud({
      phase: "error",
      message: "The Fish S2 model is not downloaded.",
    });

    expect(view).toEqual({
      notice: "The Fish S2 model is not downloaded.",
      tone: "alert",
      stopEnabled: false,
    });
  });
});

describe("setup checklist", () => {
  const items = [
    {
      id: "fish-s2",
      label: "Read-aloud voice (Fish S2)",
      ready: false,
      required: false,
      fix: "Run scripts\\Get-FishModel.ps1",
    },
    {
      id: "speech-model",
      label: "Speech model (Parakeet)",
      ready: false,
      required: true,
      fix: "Run scripts\\Get-AsrModel.ps1",
    },
    { id: "ollama", label: "Ollama", ready: true, required: true, fix: null },
  ];

  it("parses the native list and refuses anything else", () => {
    expect(parseReadiness(items)).toHaveLength(3);
    expect(parseReadiness({ items })).toBeNull();
    expect(parseReadiness([{ id: "x" }])).toBeNull();
    expect(parseReadiness([{ id: "x", label: "X", ready: true, fix: "  " }])).toEqual([
      { id: "x", label: "X", ready: true, required: true, fix: null },
    ]);
  });

  it("is shown while something required is missing, with that first", () => {
    const checklist = buildSetupChecklist(parseReadiness(items));

    expect(checklist.visible).toBe(true);
    expect(checklist.missingRequired).toBe(1);
    expect(checklist.items.map((item) => item.id)).toEqual(["speech-model", "ollama", "fish-s2"]);
  });

  it("is hidden when everything required is ready, or the command is missing", () => {
    const ready = items.map((item) => (item.required ? { ...item, ready: true } : item));

    expect(buildSetupChecklist(ready).visible).toBe(false);
    expect(buildSetupChecklist(null).visible).toBe(false);
    expect(buildSetupChecklist([]).visible).toBe(false);
  });
});
