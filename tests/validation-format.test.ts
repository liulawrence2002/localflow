import { describe, expect, it } from "vitest";
import {
  formatBytes,
  formatClockTime,
  formatDiagnosticStatus,
  formatOllamaModelLabel,
  formatOllamaStatus,
  formatOptionLabel,
} from "../src/domain/format";
import { boundedIntegerMessage, parseBoundedInteger, requiredText } from "../src/domain/validation";

describe("parseBoundedInteger", () => {
  it("accepts whole numbers in range unchanged", () => {
    expect(parseBoundedInteger("900", 300, 3000)).toEqual({
      ok: true,
      value: 900,
      adjusted: false,
    });
    expect(parseBoundedInteger(" 1200 ", 300, 3000)).toEqual({
      ok: true,
      value: 1200,
      adjusted: false,
    });
  });

  it("moves out-of-range and fractional numbers into range and says so", () => {
    expect(parseBoundedInteger("5000", 300, 3000)).toEqual({
      ok: true,
      value: 3000,
      adjusted: true,
    });
    expect(parseBoundedInteger("-4", 300, 3000)).toEqual({ ok: true, value: 300, adjusted: true });
    expect(parseBoundedInteger("950.6", 300, 3000)).toEqual({
      ok: true,
      value: 951,
      adjusted: true,
    });
  });

  it("refuses empty and non-numeric text instead of saving NaN", () => {
    for (const text of ["", "  ", "abc", "1e3", "12ms", "NaN", "Infinity", "0x10"]) {
      expect(parseBoundedInteger(text, 300, 3000), text).toEqual({ ok: false });
    }
  });

  it("explains a refused or adjusted value", () => {
    expect(boundedIntegerMessage({ ok: false }, 300, 3000, "ms")).toBe(
      "Enter a whole number from 300 to 3000 ms.",
    );
    expect(boundedIntegerMessage({ ok: true, value: 3000, adjusted: true }, 300, 3000, "ms")).toBe(
      "Set to 3000 ms: the allowed range is 300 to 3000 ms.",
    );
    expect(boundedIntegerMessage({ ok: true, value: 900, adjusted: false }, 300, 3000, "ms")).toBe(
      null,
    );
  });

  it("treats text of only spaces as missing", () => {
    expect(requiredText("  pie torch ")).toBe("pie torch");
    expect(requiredText("   ")).toBeNull();
  });
});

describe("formatters", () => {
  it("turns option values into sentence-case labels", () => {
    expect(formatOptionLabel("work_messaging")).toBe("Work messaging");
    expect(formatOptionLabel("verbatim")).toBe("Verbatim");
    expect(formatOptionLabel("")).toBe("");
  });

  it("sizes model files in MB below a gigabyte and GB above", () => {
    expect(formatBytes(2_019_000_000)).toBe("1.9 GB");
    expect(formatBytes(12 * 1024 ** 3)).toBe("12 GB");
    expect(formatBytes(32 * 1024 ** 2)).toBe("32 MB");
    expect(formatBytes(0)).toBe("0 MB");
    expect(formatOllamaModelLabel("llama3.2:3b", 2_019_000_000)).toBe("llama3.2:3b (1.9 GB)");
    expect(formatOllamaModelLabel("llama3.2:3b", undefined)).toBe("llama3.2:3b");
  });

  it("describes the Ollama check in a sentence", () => {
    expect(formatOllamaStatus(undefined)).toBe("");
    expect(
      formatOllamaStatus({ ok: false, baseUrl: "x", code: "unavailable", message: "Not running." }),
    ).toBe("Not running.");
    expect(formatOllamaStatus({ ok: true, baseUrl: "x", models: [] })).toContain("no models");
    expect(
      formatOllamaStatus({ ok: true, baseUrl: "x", models: [{ name: "a", model: "a" }] }),
    ).toBe("1 local model installed");
  });

  it("shows a time of day as HH:MM", () => {
    expect(formatClockTime(new Date(2026, 10, 25, 9, 5).getTime())).toBe("09:05");
    expect(formatClockTime(new Date(2026, 10, 25, 23, 59).getTime())).toBe("23:59");
    expect(formatClockTime(Number.NaN)).toBe("");
  });

  it("puts words on diagnostics statuses", () => {
    expect(formatDiagnosticStatus("ok")).toEqual({ label: "OK", tone: "ok" });
    expect(formatDiagnosticStatus("warning")).toEqual({ label: "Check", tone: "warning" });
    expect(formatDiagnosticStatus("blocked")).toEqual({ label: "Not available", tone: "blocked" });
    expect(formatDiagnosticStatus("error")).toEqual({ label: "Problem", tone: "error" });
    expect(formatDiagnosticStatus("strange")).toEqual({ label: "Unknown", tone: "neutral" });
  });
});
