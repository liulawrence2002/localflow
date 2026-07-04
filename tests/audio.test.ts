import { describe, expect, it } from "vitest";
import {
  AudioRingBuffer,
  calculateRms,
  downmixToMono,
  EndOfSpeechDetector,
  resampleLinear,
} from "../src/domain/audio";

describe("audio helpers", () => {
  it("keeps a bounded ring buffer", () => {
    const buffer = new AudioRingBuffer(5);

    buffer.push([1, 2, 3]);
    buffer.push([4, 5, 6, 7]);

    expect(Array.from(buffer.toFloat32Array())).toEqual([3, 4, 5, 6, 7]);
    expect(buffer.length).toBe(5);
    expect(buffer.dropped).toBe(2);
  });

  it("detects speech and end-of-speech after trailing silence", () => {
    const detector = new EndOfSpeechDetector({
      sampleRate: 1000,
      speechThresholdRms: 0.1,
      endOfSpeechMs: 300,
    });

    expect(detector.update(new Float32Array(100).fill(0.25)).isSpeech).toBe(true);
    expect(detector.update(new Float32Array(100).fill(0)).endOfSpeech).toBe(false);
    expect(detector.update(new Float32Array(200).fill(0)).endOfSpeech).toBe(true);
  });

  it("calculates RMS and normalizes channel/sample-rate shape", () => {
    expect(calculateRms([1, -1, 1, -1])).toBe(1);
    expect(
      Array.from(
        downmixToMono([
          [1, 1],
          [-1, 1],
        ]),
      ),
    ).toEqual([0, 1]);
    expect(resampleLinear([0, 1, 0, -1], 4, 2)).toHaveLength(2);
  });
});
