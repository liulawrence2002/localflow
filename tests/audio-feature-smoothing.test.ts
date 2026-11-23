import { describe, expect, it } from "vitest";
import {
  SpectrumHistory,
  FeatureSmoother,
  clampUnit,
  smoothingCoefficient,
} from "../packages/localflow-sdk/src/react/topology/audioFeatureSmoothing";

describe("clampUnit", () => {
  it("clamps to the unit range", () => {
    expect(clampUnit(-2)).toBe(0);
    expect(clampUnit(0.4)).toBe(0.4);
    expect(clampUnit(7)).toBe(1);
  });

  it("substitutes the fallback for invalid or missing values", () => {
    expect(clampUnit(Number.NaN, 0.3)).toBe(0.3);
    expect(clampUnit(Number.POSITIVE_INFINITY, 0.3)).toBe(0.3);
    expect(clampUnit(Number.NEGATIVE_INFINITY, 0.3)).toBe(0.3);
    expect(clampUnit(null, 0.3)).toBe(0.3);
    expect(clampUnit(undefined, 0.3)).toBe(0.3);
  });
});

describe("smoothingCoefficient", () => {
  it("is 0 for no elapsed time and approaches 1 for long steps", () => {
    expect(smoothingCoefficient(0, 100)).toBe(0);
    expect(smoothingCoefficient(100_000, 100)).toBeCloseTo(1, 3);
  });

  it("is frame-rate independent: two half steps equal one full step", () => {
    const tau = 120;
    const half = smoothingCoefficient(8, tau);
    const full = smoothingCoefficient(16, tau);
    // Applying the half coefficient twice must equal applying the full one once.
    const composed = 1 - (1 - half) * (1 - half);
    expect(composed).toBeCloseTo(full, 10);
  });

  it("bounds very long gaps inside FeatureSmoother so a missed event cannot snap", () => {
    const gapped = new FeatureSmoother(50, 300, 1);
    gapped.update(0, 10_000);

    const bounded = new FeatureSmoother(50, 300, 1);
    bounded.update(0, 100);

    expect(gapped.value).toBeCloseTo(bounded.value, 10);
    expect(gapped.value).toBeGreaterThan(0);
  });
});

describe("FeatureSmoother", () => {
  it("attacks faster than it releases", () => {
    const smoother = new FeatureSmoother(50, 400, 0);
    smoother.update(1, 50);
    const afterAttack = smoother.value;

    const releasing = new FeatureSmoother(50, 400, 1);
    releasing.update(0, 50);
    const releasedAmount = 1 - releasing.value;

    expect(afterAttack).toBeGreaterThan(releasedAmount);
  });

  it("converges to the target and stays clamped", () => {
    const smoother = new FeatureSmoother(50, 100, 0);
    for (let index = 0; index < 200; index += 1) {
      smoother.update(5, 16);
    }
    expect(smoother.value).toBeCloseTo(1, 3);
  });

  it("holds its value when the target is invalid instead of jumping", () => {
    const smoother = new FeatureSmoother(50, 100, 0.6);
    smoother.update(Number.NaN, 16);
    expect(smoother.value).toBeCloseTo(0.6, 6);
    smoother.update(null, 16);
    expect(smoother.value).toBeCloseTo(0.6, 6);
  });

  it("resets deterministically", () => {
    const smoother = new FeatureSmoother(50, 100, 0.9);
    smoother.reset(0.08);
    expect(smoother.value).toBe(0.08);
  });
});

describe("SpectrumHistory", () => {
  const frame = (value: number) => new Float32Array([value, value * 0.5]);

  it("returns the newest frame at age zero and older frames further back", () => {
    const history = new SpectrumHistory(4, 2);
    history.push(frame(0.2));
    history.push(frame(0.4));
    history.push(frame(0.8));

    expect(history.sample(0, 0)).toBeCloseTo(0.8, 6);
    expect(history.sample(1, 0)).toBeCloseTo(0.4, 6);
    expect(history.sample(2, 0)).toBeCloseTo(0.2, 6);
    expect(history.sample(0, 1)).toBeCloseTo(0.4, 6);
  });

  it("blends linearly between neighbouring frames at fractional ages", () => {
    const history = new SpectrumHistory(4, 2);
    history.push(frame(0.2));
    history.push(frame(0.6));

    expect(history.sample(0.5, 0)).toBeCloseTo(0.4, 6);
    expect(history.sample(0.25, 0)).toBeCloseTo(0.5, 6);
  });

  it("overwrites the oldest frame and clamps reads to the oldest one kept", () => {
    const history = new SpectrumHistory(3, 2);
    for (const value of [0.1, 0.2, 0.3, 0.4]) {
      history.push(frame(value));
    }

    expect(history.sample(0, 0)).toBeCloseTo(0.4, 6);
    expect(history.sample(2, 0)).toBeCloseTo(0.2, 6);
    // Asking for more history than exists returns the oldest frame, not garbage.
    expect(history.sample(99, 0)).toBeCloseTo(0.2, 6);
    expect(history.sample(-5, 0)).toBeCloseTo(0.4, 6);
  });

  it("blends linearly between neighbouring columns", () => {
    const history = new SpectrumHistory(2, 3);
    history.push(new Float32Array([0, 1, 0.5]));

    expect(history.sample(0, 0.5)).toBeCloseTo(0.5, 6);
    expect(history.sample(0, 1.5)).toBeCloseTo(0.75, 6);
    // Out-of-range columns clamp to the ends instead of reading another frame.
    expect(history.sample(0, 9)).toBeCloseTo(0.5, 6);
    expect(history.sample(0, -3)).toBe(0);
  });

  it("sanitises stored values and can be cleared", () => {
    const history = new SpectrumHistory(2, 2);
    history.push(new Float32Array([Number.NaN, 7]));
    expect(history.sample(0, 0)).toBe(0);
    expect(history.sample(0, 1)).toBe(1);

    history.fill(0);
    expect(history.sample(0, 1)).toBe(0);
    expect(history.sample(Number.NaN, 0)).toBe(0);
  });
});
