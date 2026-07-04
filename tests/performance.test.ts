import { describe, expect, it } from "vitest";
import {
  PerformanceRecorder,
  formatMeasurement,
  performanceDiagnostics,
} from "../src/domain/performance";

describe("performance instrumentation", () => {
  it("derives known latency measurements from timestamp marks", () => {
    const times = [1000, 1042, 1100, 1400, 2100];
    const recorder = new PerformanceRecorder(() => times.shift() ?? 0);

    recorder.mark("hotkey_pressed");
    recorder.mark("recording_started");
    recorder.mark("first_asr_partial");
    recorder.mark("hotkey_released");
    recorder.mark("final_transcript_ready");
    recorder.deriveKnownDurations();

    const snapshot = recorder.snapshot();

    expect(snapshot.measurements).toEqual([
      { key: "hotkey_to_recording_start", value: 42, unit: "ms" },
      { key: "asr_partial_latency", value: 58, unit: "ms" },
      { key: "release_to_final_transcript", value: 700, unit: "ms" },
    ]);
  });

  it("records LLM, insertion, model-load, and memory metrics explicitly", () => {
    const recorder = new PerformanceRecorder();

    recorder.recordDuration("llm_refinement_latency", 321.2);
    recorder.recordDuration("text_insertion_latency", 18.6);
    recorder.recordDuration("model_load_time", 2400);
    recorder.recordPeakMemory(1.5 * 1024 ** 3);

    expect(recorder.snapshot().diagnostics).toEqual(
      expect.arrayContaining([
        { label: "LLM refinement latency", value: "321 ms", status: "ok" },
        { label: "Text insertion latency", value: "19 ms", status: "ok" },
        { label: "Model load time", value: "2400 ms", status: "ok" },
        { label: "Peak memory", value: "1.5 GiB", status: "ok" },
      ]),
    );
  });

  it("reports unmeasured metrics as warnings instead of inventing values", () => {
    const diagnostics = performanceDiagnostics([]);

    expect(diagnostics).toContainEqual({
      label: "ASR partial latency",
      value: "Not measured",
      status: "warning",
    });
    expect(diagnostics.every((metric) => metric.value === "Not measured")).toBe(true);
  });

  it("rejects invalid measurement values", () => {
    const recorder = new PerformanceRecorder();

    expect(() => recorder.recordDuration("model_load_time", -1)).toThrow(/duration/);
    expect(() => recorder.recordPeakMemory(Number.NaN)).toThrow(/Peak memory/);
  });

  it("requires both marks for manual duration measurement", () => {
    const recorder = new PerformanceRecorder(() => 25);

    recorder.mark("model_load_started");

    expect(() =>
      recorder.measureDuration("model_load_time", "model_load_started", "model_load_finished"),
    ).toThrow(/without both marks/);
  });

  it("formats byte and millisecond measurements", () => {
    expect(formatMeasurement({ key: "peak_memory", value: 512, unit: "bytes" })).toBe("512 B");
    expect(formatMeasurement({ key: "peak_memory", value: 64 * 1024 ** 2, unit: "bytes" })).toBe(
      "64 MiB",
    );
    expect(formatMeasurement({ key: "asr_partial_latency", value: 12.5, unit: "ms" })).toBe(
      "13 ms",
    );
  });
});
