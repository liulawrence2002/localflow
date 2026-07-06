import { describe, expect, it } from "vitest";
import {
  CircularHistory,
  FeatureSmoother,
  clampUnit,
  historyStepsForDepth,
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

describe("CircularHistory", () => {
  it("returns the newest entry at offset 0 and older entries behind it", () => {
    const history = new CircularHistory(4, 0);
    history.push(0.1);
    history.push(0.2);
    history.push(0.3);
    expect(history.at(0)).toBeCloseTo(0.3);
    expect(history.at(1)).toBeCloseTo(0.2);
    expect(history.at(2)).toBeCloseTo(0.1);
  });

  it("overwrites the oldest entry once full and never grows", () => {
    const history = new CircularHistory(3, 0);
    history.push(0.1);
    history.push(0.2);
    history.push(0.3);
    history.push(0.4);
    expect(history.at(0)).toBeCloseTo(0.4);
    expect(history.at(2)).toBeCloseTo(0.2);
    // Reading past the capacity clamps to the oldest retained entry.
    expect(history.at(10)).toBeCloseTo(0.2);
  });

  it("ignores invalid pushes rather than storing NaN", () => {
    const history = new CircularHistory(2, 0.5);
    history.push(Number.NaN);
    expect(Number.isFinite(history.at(0))).toBe(true);
  });
});

describe("historyStepsForDepth", () => {
  it("gives the foreground the newest sample and rear rows older ones", () => {
    expect(historyStepsForDepth(1, 32)).toBe(0);
    expect(historyStepsForDepth(0, 32)).toBeGreaterThan(historyStepsForDepth(0.5, 32));
    expect(historyStepsForDepth(0, 32)).toBeLessThan(32);
  });
});
