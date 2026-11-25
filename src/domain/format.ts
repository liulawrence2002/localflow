/** Formatting for what the settings window shows. Pure functions, unit tested. */

import type { OllamaConnectionStatus } from "./ollama";
import type { DiagnosticMetric } from "./types";

/** `work_messaging` becomes "Work messaging". */
export function formatOptionLabel(value: string): string {
  const words = value.split("_").filter(Boolean).join(" ");
  return words ? `${words.charAt(0).toUpperCase()}${words.slice(1)}` : "";
}

/** One line about the local Ollama check, or "" before it has run. */
export function formatOllamaStatus(status: OllamaConnectionStatus | undefined): string {
  if (!status) {
    return "";
  }
  if (!status.ok) {
    return status.message;
  }
  if (status.models.length === 0) {
    return "Ollama is running, but no models are installed.";
  }
  return `${status.models.length} local model${status.models.length === 1 ? "" : "s"} installed`;
}

export function formatOllamaModelLabel(name: string, sizeBytes: number | undefined): string {
  return sizeBytes ? `${name} (${formatBytes(sizeBytes)})` : name;
}

/** Sizes as Windows Explorer would round them: one decimal below 10 GB. */
export function formatBytes(sizeBytes: number): string {
  if (!Number.isFinite(sizeBytes) || sizeBytes <= 0) {
    return "0 MB";
  }
  const gib = sizeBytes / 1024 ** 3;
  if (gib >= 1) {
    return `${gib.toFixed(gib >= 10 ? 0 : 1)} GB`;
  }
  const mib = sizeBytes / 1024 ** 2;
  return `${Math.max(1, Math.round(mib))} MB`;
}

/** A time of day as HH:MM in local time, for "happened at" lines. */
export function formatClockTime(atMs: number): string {
  if (!Number.isFinite(atMs)) {
    return "";
  }
  const date = new Date(atMs);
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${hours}:${minutes}`;
}

export type StatusTone = "ok" | "warning" | "blocked" | "error" | "neutral";

/** Words for a diagnostics row's status, instead of the raw `ok` / `blocked`. */
export function formatDiagnosticStatus(status: DiagnosticMetric["status"] | string): {
  label: string;
  tone: StatusTone;
} {
  switch (status) {
    case "ok":
      return { label: "OK", tone: "ok" };
    case "warning":
      return { label: "Check", tone: "warning" };
    case "blocked":
      return { label: "Not available", tone: "blocked" };
    case "error":
      return { label: "Problem", tone: "error" };
    default:
      return { label: "Unknown", tone: "neutral" };
  }
}
