import { describe, expect, it } from "vitest";
import { defaultStatus } from "../src/domain/defaults";
import { buildDiagnosticsExport, serializeDiagnosticsExport } from "../src/domain/diagnostics";
import type { AppStatus } from "../src/domain/types";

const statusWithSensitiveHistory: AppStatus = {
  ...defaultStatus,
  settings: {
    ...defaultStatus.settings,
    models: {
      ...defaultStatus.settings.models,
      whisperModelPath: "C:\\Users\\liula\\models\\ggml-base.en.bin",
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

    expect(serialized).not.toContain("C:\\Users\\liula");
    expect(serialized).not.toContain("five five five");
    expect(serialized).toContain('"whisperModelConfigured": true');
    expect(serialized).toContain('"ollamaModelConfigured": true');
  });

  it("can include transcript text only when explicitly requested", () => {
    const exported = buildDiagnosticsExport(statusWithSensitiveHistory, {
      includeTranscriptText: true,
    });

    expect(exported.history[0].rawTranscript).toBe("call me at five five five one two one two");
    expect(exported.history[0].finalText).toBe("Call me at 555-1212.");
  });
});
