import { describe, expect, it } from "vitest";
import {
  buildWhisperInitialPrompt,
  parseWhisperJsonTranscript,
  planWhisperCppInvocation,
} from "../src/domain/whisperSidecar";
import type { DictionaryEntry } from "../src/domain/types";

const baseConfig = {
  executablePath: "C:\\whisper.cpp\\build\\bin\\Release\\whisper-cli.exe",
  modelPath: "C:\\models\\ggml-base.en.bin",
  audioPath: "C:\\Users\\liula\\AppData\\Local\\LocalFlow\\cache\\session.wav",
  outputFileBasePath: "C:\\Users\\liula\\AppData\\Local\\LocalFlow\\cache\\session",
  language: "auto",
  threads: 4,
  timeoutMs: 30_000,
};

describe("whisper.cpp sidecar contract", () => {
  it("plans a JSON-producing whisper-cli invocation", () => {
    const plan = planWhisperCppInvocation({
      ...baseConfig,
      initialPrompt: "Vocabulary: PyTorch.",
      hardwareAcceleration: "cpu",
      includeTokenTimestamps: true,
    });

    expect(plan.executablePath).toBe(baseConfig.executablePath);
    expect(plan.expectedJsonPath).toBe(`${baseConfig.outputFileBasePath}.json`);
    expect(plan.args).toEqual([
      "--model",
      baseConfig.modelPath,
      "--file",
      baseConfig.audioPath,
      "--threads",
      "4",
      "--language",
      "auto",
      "--output-json",
      "--output-file",
      baseConfig.outputFileBasePath,
      "--no-prints",
      "--prompt",
      "Vocabulary: PyTorch.",
      "--output-json-full",
      "--no-gpu",
    ]);
  });

  it("defaults a blank language to auto-detect", () => {
    const plan = planWhisperCppInvocation({ ...baseConfig, language: " " });

    expect(plan.args).toContain("auto");
  });

  it("rejects missing model paths and unsupported audio formats clearly", () => {
    expect(() => planWhisperCppInvocation({ ...baseConfig, modelPath: "" })).toThrow(/model path/);
    expect(() =>
      planWhisperCppInvocation({ ...baseConfig, audioPath: "C:\\cache\\session.aac" }),
    ).toThrow(/wav, flac, mp3, or ogg/);
  });

  it("rejects invalid thread and timeout settings", () => {
    expect(() => planWhisperCppInvocation({ ...baseConfig, threads: 0 })).toThrow(/thread count/);
    expect(() => planWhisperCppInvocation({ ...baseConfig, timeoutMs: 0 })).toThrow(/timeout/);
  });

  it("builds a compact dictionary prompt without duplicate phrases", () => {
    const dictionary: DictionaryEntry[] = [
      {
        id: "dict-pytorch",
        phrase: "PyTorch",
        pronunciationHint: "pie torch",
        category: "technical",
        caseSensitive: false,
      },
      {
        id: "dict-pytorch-duplicate",
        phrase: "pytorch",
        category: "technical",
        caseSensitive: false,
      },
      {
        id: "dict-localflow",
        phrase: "LocalFlow",
        category: "custom",
        caseSensitive: false,
      },
    ];

    expect(buildWhisperInitialPrompt(dictionary)).toBe(
      "Vocabulary: PyTorch (heard as pie torch), LocalFlow.",
    );
  });

  it("parses whisper.cpp JSON transcript segments with millisecond offsets", () => {
    const transcript = parseWhisperJsonTranscript(
      JSON.stringify({
        transcription: [
          {
            timestamps: { from: "00:00:00,000", to: "00:00:01,500" },
            offsets: { from: 0, to: 1500 },
            text: " Hello world",
          },
          {
            offsets: { from: 1500, to: 2500 },
            text: " from LocalFlow ",
          },
        ],
      }),
    );

    expect(transcript).toEqual({
      text: "Hello world from LocalFlow",
      segments: [
        { startMs: 0, endMs: 1500, text: "Hello world" },
        { startMs: 1500, endMs: 2500, text: "from LocalFlow" },
      ],
    });
  });

  it("parses alternate JSON segment start/end seconds", () => {
    const transcript = parseWhisperJsonTranscript(
      JSON.stringify({
        segments: [{ start: 1.25, end: 2.75, text: " timed segment " }],
      }),
    );

    expect(transcript.segments[0]).toEqual({
      startMs: 1250,
      endMs: 2750,
      text: "timed segment",
    });
  });

  it("rejects invalid JSON output", () => {
    expect(() => parseWhisperJsonTranscript("not json")).toThrow(/Unexpected token/);
  });
});
