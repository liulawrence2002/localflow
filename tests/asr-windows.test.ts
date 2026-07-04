import { describe, expect, it } from "vitest";
import { planRollingWindows } from "../src/domain/asrWindows";

describe("rolling ASR windows", () => {
  it("plans overlapped audio windows for incremental transcription", () => {
    const windows = planRollingWindows({
      totalSamples: 48_000,
      sampleRate: 16_000,
      windowMs: 1000,
      overlapMs: 250,
    });

    expect(windows).toEqual([
      { index: 0, startSample: 0, endSample: 16_000, overlapStartSample: 0, durationMs: 1000 },
      {
        index: 1,
        startSample: 12_000,
        endSample: 28_000,
        overlapStartSample: 8_000,
        durationMs: 1000,
      },
      {
        index: 2,
        startSample: 24_000,
        endSample: 40_000,
        overlapStartSample: 20_000,
        durationMs: 1000,
      },
      {
        index: 3,
        startSample: 36_000,
        endSample: 48_000,
        overlapStartSample: 32_000,
        durationMs: 750,
      },
    ]);
  });
});
