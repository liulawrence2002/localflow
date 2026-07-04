import type { DictionaryEntry } from "./types";

export type WhisperSidecarErrorCode =
  | "missing_executable_path"
  | "missing_model_path"
  | "missing_audio_path"
  | "missing_output_path"
  | "unsupported_audio_format"
  | "invalid_threads"
  | "invalid_timeout"
  | "invalid_json"
  | "invalid_transcript";

export class WhisperSidecarError extends Error {
  readonly code: WhisperSidecarErrorCode;

  constructor(code: WhisperSidecarErrorCode, message: string) {
    super(message);
    this.name = "WhisperSidecarError";
    this.code = code;
  }
}

export interface WhisperCppInvocationConfig {
  executablePath: string;
  modelPath: string;
  audioPath: string;
  outputFileBasePath: string;
  language: string;
  threads: number;
  timeoutMs: number;
  initialPrompt?: string;
  hardwareAcceleration?: "auto" | "cpu";
  includeTokenTimestamps?: boolean;
}

export interface WhisperCppInvocationPlan {
  executablePath: string;
  args: string[];
  timeoutMs: number;
  expectedJsonPath: string;
}

export interface WhisperSegment {
  startMs: number;
  endMs: number;
  text: string;
}

export interface WhisperTranscript {
  text: string;
  segments: WhisperSegment[];
}

const supportedAudioExtensions = new Set(["flac", "mp3", "ogg", "wav"]);

export function planWhisperCppInvocation(
  config: WhisperCppInvocationConfig,
): WhisperCppInvocationPlan {
  validateWhisperConfig(config);

  const args = [
    "--model",
    config.modelPath.trim(),
    "--file",
    config.audioPath.trim(),
    "--threads",
    String(config.threads),
    "--language",
    normalizeLanguage(config.language),
    "--output-json",
    "--output-file",
    config.outputFileBasePath.trim(),
    "--no-prints",
  ];

  const initialPrompt = config.initialPrompt?.trim();
  if (initialPrompt) {
    args.push("--prompt", initialPrompt);
  }

  if (config.includeTokenTimestamps) {
    args.push("--output-json-full");
  }

  if (config.hardwareAcceleration === "cpu") {
    args.push("--no-gpu");
  }

  return {
    executablePath: config.executablePath.trim(),
    args,
    timeoutMs: config.timeoutMs,
    expectedJsonPath: `${config.outputFileBasePath.trim()}.json`,
  };
}

export function buildWhisperInitialPrompt(dictionary: DictionaryEntry[], maxChars = 700): string {
  const terms: string[] = [];
  const seen = new Set<string>();

  for (const entry of dictionary) {
    const phrase = entry.phrase.trim();
    if (!phrase) {
      continue;
    }

    const key = phrase.toLowerCase();
    if (seen.has(key)) {
      continue;
    }
    seen.add(key);

    terms.push(
      entry.pronunciationHint?.trim()
        ? `${phrase} (heard as ${entry.pronunciationHint.trim()})`
        : phrase,
    );
  }

  if (terms.length === 0) {
    return "";
  }

  const prefix = "Vocabulary: ";
  const selected: string[] = [];

  for (const term of terms) {
    const candidate = `${prefix}${[...selected, term].join(", ")}.`;
    if (candidate.length > maxChars) {
      break;
    }
    selected.push(term);
  }

  return selected.length > 0 ? `${prefix}${selected.join(", ")}.` : "";
}

export function parseWhisperJsonTranscript(payload: string): WhisperTranscript {
  let parsed: unknown;
  try {
    parsed = JSON.parse(payload);
  } catch (error) {
    throw new WhisperSidecarError(
      "invalid_json",
      error instanceof Error ? error.message : "Whisper JSON output could not be parsed.",
    );
  }

  if (!isRecord(parsed)) {
    throw new WhisperSidecarError("invalid_transcript", "Whisper JSON output was not an object.");
  }

  const segments = extractSegments(parsed);
  const text =
    typeof parsed.text === "string"
      ? parsed.text.trim()
      : segments
          .map((segment) => segment.text)
          .join(" ")
          .replace(/\s+/g, " ")
          .trim();

  return { text, segments };
}

function validateWhisperConfig(config: WhisperCppInvocationConfig): void {
  if (!config.executablePath.trim()) {
    throw new WhisperSidecarError(
      "missing_executable_path",
      "Configure the whisper.cpp executable path before transcription.",
    );
  }

  if (!config.modelPath.trim()) {
    throw new WhisperSidecarError(
      "missing_model_path",
      "Configure a local whisper.cpp model path before transcription.",
    );
  }

  if (!config.audioPath.trim()) {
    throw new WhisperSidecarError(
      "missing_audio_path",
      "Provide a local audio file path for whisper.cpp transcription.",
    );
  }

  if (!config.outputFileBasePath.trim()) {
    throw new WhisperSidecarError(
      "missing_output_path",
      "Provide a cache output path for whisper.cpp JSON output.",
    );
  }

  if (!supportedAudioExtensions.has(fileExtension(config.audioPath))) {
    throw new WhisperSidecarError(
      "unsupported_audio_format",
      "whisper.cpp sidecar input must be wav, flac, mp3, or ogg.",
    );
  }

  if (!Number.isInteger(config.threads) || config.threads < 1 || config.threads > 64) {
    throw new WhisperSidecarError(
      "invalid_threads",
      "whisper.cpp thread count must be an integer from 1 to 64.",
    );
  }

  if (!Number.isFinite(config.timeoutMs) || config.timeoutMs <= 0) {
    throw new WhisperSidecarError(
      "invalid_timeout",
      "whisper.cpp transcription timeout must be greater than zero.",
    );
  }
}

function extractSegments(payload: Record<string, unknown>): WhisperSegment[] {
  const rawSegments = Array.isArray(payload.transcription)
    ? payload.transcription
    : Array.isArray(payload.segments)
      ? payload.segments
      : [];

  return rawSegments.map((segment) => parseSegment(segment));
}

function parseSegment(payload: unknown): WhisperSegment {
  if (!isRecord(payload) || typeof payload.text !== "string") {
    throw new WhisperSidecarError(
      "invalid_transcript",
      "Whisper transcript segment was missing text.",
    );
  }

  const offsets = isRecord(payload.offsets) ? payload.offsets : undefined;
  const timestamps = isRecord(payload.timestamps) ? payload.timestamps : undefined;

  return {
    startMs:
      numberMilliseconds(offsets?.from) ??
      stringTimestampMs(timestamps?.from) ??
      numberSecondsMs(payload.start) ??
      0,
    endMs:
      numberMilliseconds(offsets?.to) ??
      stringTimestampMs(timestamps?.to) ??
      numberSecondsMs(payload.end) ??
      0,
    text: payload.text.trim(),
  };
}

function normalizeLanguage(language: string): string {
  const trimmed = language.trim();
  return trimmed ? trimmed : "auto";
}

function fileExtension(path: string): string {
  const normalized = path.trim().toLowerCase();
  const match = /\.([^.\\/]+)$/.exec(normalized);
  return match?.[1] ?? "";
}

function numberMilliseconds(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }

  return Math.round(value);
}

function numberSecondsMs(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    return undefined;
  }

  return Math.round(value * 1000);
}

function stringTimestampMs(value: unknown): number | undefined {
  if (typeof value !== "string") {
    return undefined;
  }

  const match = /^(\d{2}):(\d{2}):(\d{2})[,.](\d{3})$/.exec(value.trim());
  if (!match) {
    return undefined;
  }

  const [, hours, minutes, seconds, millis] = match;
  return (
    Number(hours) * 60 * 60 * 1000 +
    Number(minutes) * 60 * 1000 +
    Number(seconds) * 1000 +
    Number(millis)
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
