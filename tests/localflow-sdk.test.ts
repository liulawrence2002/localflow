import { describe, expect, it, vi } from "vitest";
import { createLocalFlowClient, createMockTranscriber } from "@localflow/sdk";
import type { LocalFlowTranscriptEvent, LocalFlowVoiceState } from "@localflow/sdk";

describe("LocalFlow SDK client", () => {
  it("emits listening, transcript, and inserted events through the in-app provider contract", async () => {
    const client = createLocalFlowClient({
      emitMockLevels: false,
      transcriber: createMockTranscriber({
        text: "quick mobile text",
        refinedText: "refined mobile text",
      }),
    });
    const voiceStates: LocalFlowVoiceState[] = [];
    const transcripts: LocalFlowTranscriptEvent[] = [];

    client.on("voice_state", (event) => voiceStates.push(event));
    client.on("transcript", (event) => transcripts.push(event));

    await client.startDictation({ sessionId: "mobile-session" });
    await client.stopDictation();
    await client.cancelDictation();

    expect(voiceStates.map((event) => event.phase)).toEqual([
      "listening",
      "processing",
      "inserted",
      "idle",
    ]);
    expect(transcripts).toEqual([
      {
        sessionId: "mobile-session",
        text: "quick mobile text",
        kind: "quick",
        latencyMs: expect.any(Number),
      },
      {
        sessionId: "mobile-session",
        text: "refined mobile text",
        kind: "refined",
        latencyMs: expect.any(Number),
      },
    ]);
  });

  it("keeps failed transcription recoverable and emits an error without a transcript", async () => {
    const client = createLocalFlowClient({
      emitMockLevels: false,
      transcriber: {
        async transcribe() {
          throw new Error("microphone denied");
        },
      },
    });
    const errors: string[] = [];
    const transcripts: LocalFlowTranscriptEvent[] = [];

    client.on("error", (event) => errors.push(`${event.code}:${event.message}`));
    client.on("transcript", (event) => transcripts.push(event));

    await client.startDictation({ sessionId: "failed-session" });
    await client.stopDictation();
    await client.cancelDictation();

    expect(errors).toEqual(["transcription_failed:microphone denied"]);
    expect(transcripts).toEqual([]);
  });

  it("saves host-owned settings without imposing a desktop schema", async () => {
    const client = createLocalFlowClient<{ language: string }>({ emitMockLevels: false });
    const statusEvents: string[] = [];
    client.on("status", (status) => statusEvents.push(status.settings?.language ?? "missing"));

    const status = await client.saveSettings({ language: "en-US" });

    expect(status.settings).toEqual({ language: "en-US" });
    expect(statusEvents).toEqual(["en-US"]);
  });

  it("cancels stale sessions so superseded transcriptions cannot emit transcripts", async () => {
    let resolveTranscription: (value: { text: string }) => void = vi.fn();
    const client = createLocalFlowClient({
      emitMockLevels: false,
      transcriber: {
        transcribe() {
          return new Promise<{ text: string }>((resolve) => {
            resolveTranscription = resolve;
          });
        },
      },
    });
    const transcripts: LocalFlowTranscriptEvent[] = [];
    client.on("transcript", (event) => transcripts.push(event));

    await client.startDictation({ sessionId: "old-session" });
    const stopping = client.stopDictation();
    await client.startDictation({ sessionId: "new-session" });
    resolveTranscription({ text: "stale text" });
    await stopping;
    await client.cancelDictation();

    expect(transcripts).toEqual([]);
  });
});
