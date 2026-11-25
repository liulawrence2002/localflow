import { describe, expect, it } from "vitest";
import { defaultStatus } from "../src/domain/defaults";
import {
  buildDiagnosticsExport,
  findMicrophoneNames,
  microphonePlaceholder,
  redactLocalDetails,
  serializeDiagnosticsExport,
  userFolderPlaceholder,
} from "../src/domain/diagnostics";
import type { AppStatus } from "../src/domain/types";

const statusWithSensitiveHistory: AppStatus = {
  ...defaultStatus,
  settings: {
    ...defaultStatus.settings,
    models: {
      ...defaultStatus.settings.models,
      whisperModelPath: "C:\\Users\\you\\models\\ggml-base.en.bin",
      ollamaModel: "llama3.1:8b-instruct",
    },
  },
  history: [
    {
      id: "history-1",
      completedAt: "2026-07-04T12:00:00.000Z",
      targetApplication: "Notepad",
      rawTranscript: "call me at five five five one two one two",
      deterministicText: "call me at 555-1212",
      refinedText: "Call me at 555-1212.",
      finalText: "Call me at 555-1212.",
      cleanupLevel: "balanced",
    },
  ],
};

describe("diagnostics export", () => {
  it("redacts dictated content by default", () => {
    const exported = buildDiagnosticsExport(statusWithSensitiveHistory, {
      generatedAt: "2026-07-04T13:00:00.000Z",
    });

    expect(exported.history[0]).toMatchObject({
      rawTranscript: "[redacted:41 chars]",
      deterministicText: "[redacted:19 chars]",
      refinedText: "[redacted:20 chars]",
      finalText: "[redacted:20 chars]",
    });
  });

  it("does not include local model paths in serialized diagnostics", () => {
    const serialized = serializeDiagnosticsExport(statusWithSensitiveHistory, {
      generatedAt: "2026-07-04T13:00:00.000Z",
    });

    expect(serialized).not.toContain("C:\\\\Users\\\\you");
    expect(serialized).not.toContain("ggml-base.en.bin");
    expect(serialized).not.toContain("five five five");
    expect(serialized).toContain('"whisperModelConfigured": true');
    expect(serialized).toContain('"ollamaModelConfigured": true');
  });

  it("can include transcript text only when explicitly requested", () => {
    const exported = buildDiagnosticsExport(statusWithSensitiveHistory, {
      includeTranscriptText: true,
    });

    expect(exported.history[0]?.rawTranscript).toBe("call me at five five five one two one two");
    expect(exported.history[0]?.finalText).toBe("Call me at 555-1212.");
  });

  it("replaces the user folder and microphone names in diagnostics rows", () => {
    const status: AppStatus = {
      ...statusWithSensitiveHistory,
      diagnostics: [
        {
          label: "Last recording start",
          value: "Microphone (Ada's AirPods) at 2026-11-25T10:00:00Z (48000 Hz, 2 ch)",
          status: "ok",
        },
        {
          label: "Last recording error",
          value:
            'Captured audio from "Microphone (Ada\'s AirPods)" was silent or near-silent. at 2026-11-25T10:01:00Z',
          status: "ok",
        },
        {
          label: "Speech model",
          value: "Missing: C:\\Users\\Ada Lovelace\\localflow\\.localflow-runtime\\models\\x.gguf",
          status: "error",
        },
        { label: "Fallback", value: "Checked d:/users/ada/models/y.bin", status: "warning" },
      ],
    };

    const exported = buildDiagnosticsExport(status, { generatedAt: "2026-11-25T11:00:00Z" });
    const serialized = JSON.stringify(exported);

    expect(serialized).not.toContain("AirPods");
    expect(serialized).not.toContain("Ada");
    expect(serialized).not.toContain("ada/");
    expect(exported.diagnostics[0]?.value).toBe(
      `${microphonePlaceholder} at 2026-11-25T10:00:00Z (48000 Hz, 2 ch)`,
    );
    expect(exported.diagnostics[1]?.value).toContain(`from "${microphonePlaceholder}"`);
    expect(exported.diagnostics[2]?.value).toBe(
      `Missing: ${userFolderPlaceholder}\\localflow\\.localflow-runtime\\models\\x.gguf`,
    );
    expect(exported.diagnostics[3]?.value).toBe(`Checked ${userFolderPlaceholder}/models/y.bin`);
  });

  it("finds microphone names from settings and older recording rows", () => {
    const status: AppStatus = {
      ...defaultStatus,
      settings: {
        ...defaultStatus.settings,
        microphone: { ...defaultStatus.settings.microphone, selectedDeviceName: "Studio Mic" },
      },
      diagnostics: [
        {
          label: "Last recording start",
          value: "USB Desk Mic at t (16000 Hz, 1 ch)",
          status: "ok",
        },
      ],
    };

    expect(findMicrophoneNames(status)).toEqual(["USB Desk Mic", "Studio Mic"]);
    expect(redactLocalDetails("Studio Mic stopped", findMicrophoneNames(status))).toBe(
      `${microphonePlaceholder} stopped`,
    );
  });

  it("leaves the default microphone name and plain text alone", () => {
    expect(findMicrophoneNames(defaultStatus)).toEqual([]);
    expect(redactLocalDetails("Registered desktop hotkeys: Ctrl+Alt+Space")).toBe(
      "Registered desktop hotkeys: Ctrl+Alt+Space",
    );
  });
});
