import { describe, expect, it } from "vitest";
import {
  BAND_COUNT,
  COLUMN_COUNT,
  ambientRidge,
  bandFrequency,
  bandIndexForFrequency,
  bandsToColumns,
  catmullRom,
  conditionBands,
  historyAge,
  mirroredColumn,
  rampColor,
  resampleBands,
  ridgeEnvelope,
  smoothBands,
  softCeiling,
  sweepCrest,
  synthesizeBands,
} from "../packages/localflow-sdk/src/react/topology/spectrumMath";

function argmax(values: Float32Array): number {
  let best = 0;
  for (let index = 1; index < values.length; index += 1) {
    if (values[index] > values[best]) {
      best = index;
    }
  }
  return best;
}

function sum(values: Float32Array, from: number, to: number): number {
  let total = 0;
  for (let index = from; index < to; index += 1) {
    total += values[index];
  }
  return total;
}

describe("band frequency mapping", () => {
  it("spans 70 Hz to 8 kHz and inverts exactly", () => {
    expect(bandFrequency(0)).toBeCloseTo(70, 6);
    expect(bandFrequency(BAND_COUNT - 1)).toBeCloseTo(8000, 3);
    for (const index of [0, 7.5, 16, 31]) {
      expect(bandIndexForFrequency(bandFrequency(index))).toBeCloseTo(index, 6);
    }
  });
});

describe("resampleBands", () => {
  it("copies same-length input and clamps out-of-range values", () => {
    const out = new Float32Array(BAND_COUNT);
    const unit = new Array(BAND_COUNT).fill(0).map((_, index) => index / (BAND_COUNT - 1));
    resampleBands(unit, out);
    expect(out[0]).toBe(0);
    expect(out[BAND_COUNT - 1]).toBeCloseTo(1, 6);
    expect(out[16]).toBeCloseTo(16 / 31, 6);

    // A value above 1 is clamped; it never rescales the rest of the frame.
    const hot = new Array(BAND_COUNT).fill(0.5);
    hot[3] = 1.01;
    hot[4] = 255;
    resampleBands(hot, out);
    expect(out[3]).toBe(1);
    expect(out[4]).toBe(1);
    expect(out[5]).toBeCloseTo(0.5, 6);
  });

  it("stretches shorter input, keeping the end points", () => {
    const out = new Float32Array(BAND_COUNT);
    resampleBands([0.2, 0.4, 0.6, 0.8, 1, 0.8, 0.6, 0.4, 0.2, 0, 0, 0, 0, 0, 0, 0.5], out);
    expect(out[0]).toBeCloseTo(0.2, 6);
    expect(out[BAND_COUNT - 1]).toBeCloseTo(0.5, 6);
  });

  it("maps invalid values and empty input to zero", () => {
    const out = new Float32Array(BAND_COUNT).fill(0.7);
    resampleBands([], out);
    expect(Math.max(...out)).toBe(0);

    resampleBands(new Array(BAND_COUNT).fill(Number.NaN), out);
    expect(out.every((value) => value === 0)).toBe(true);

    resampleBands(new Array(BAND_COUNT).fill(-3), out);
    expect(out.every((value) => value === 0)).toBe(true);
  });
});

describe("synthesizeBands", () => {
  it("is deterministic, bounded, and silent at zero level", () => {
    const a = new Float32Array(BAND_COUNT);
    const b = new Float32Array(BAND_COUNT);
    synthesizeBands(a, 0.8, 0.4, 0.6, 1.25);
    synthesizeBands(b, 0.8, 0.4, 0.6, 1.25);
    expect(Array.from(a)).toEqual(Array.from(b));
    expect(a.every((value) => value >= 0 && value <= 1)).toBe(true);
    expect(Math.max(...a)).toBeGreaterThan(0.4);

    synthesizeBands(a, 0, 0.4, 0.6, 1.25);
    expect(Math.max(...a)).toBe(0);
  });

  it("moves the lowest bump up with pitch and shifts energy up with brightness", () => {
    const low = new Float32Array(BAND_COUNT);
    const high = new Float32Array(BAND_COUNT);
    synthesizeBands(low, 1, 0.05, 0, 0);
    synthesizeBands(high, 1, 0.95, 0, 0);
    // 80 Hz sits near band 1 and about 300 Hz near band 9.
    expect(low[1]).toBeGreaterThan(high[1]);
    expect(high[9]).toBeGreaterThan(low[9]);

    const dark = new Float32Array(BAND_COUNT);
    const bright = new Float32Array(BAND_COUNT);
    synthesizeBands(dark, 1, 0.5, 0, 0);
    synthesizeBands(bright, 1, 0.5, 1, 0);
    const share = (bands: Float32Array) => sum(bands, 16, 32) / sum(bands, 0, 32);
    expect(share(bright)).toBeGreaterThan(share(dark));
  });

  it("stays finite for invalid inputs", () => {
    const out = new Float32Array(BAND_COUNT);
    synthesizeBands(out, Number.NaN, Number.POSITIVE_INFINITY, Number.NaN, Number.NaN);
    expect(out.every((value) => Number.isFinite(value))).toBe(true);
  });
});

describe("conditionBands", () => {
  it("removes the floor and fades everything out near silence", () => {
    const loud = new Float32Array(BAND_COUNT).fill(0.5);
    conditionBands(loud, 0.8);
    expect(loud[10]).toBeCloseTo((0.5 - 0.04) / 0.96, 5);

    const hiss = new Float32Array(BAND_COUNT).fill(0.03);
    conditionBands(hiss, 0.8);
    expect(Math.max(...hiss)).toBe(0);

    const quiet = new Float32Array(BAND_COUNT).fill(0.5);
    conditionBands(quiet, 0.05);
    expect(Math.max(...quiet)).toBe(0);
  });

  it("softens a single-band spike without moving it", () => {
    const bands = new Float32Array(BAND_COUNT);
    bands[10] = 1;
    conditionBands(bands, 1);
    expect(argmax(bands)).toBe(10);
    expect(bands[10]).toBeCloseTo(0.5, 5);
    expect(bands[9]).toBeCloseTo(0.25, 5);
    expect(bands[11]).toBeCloseTo(0.25, 5);
    expect(bands[13]).toBe(0);
  });
});

describe("smoothBands", () => {
  it("is frame-rate independent", () => {
    const target = new Float32Array(BAND_COUNT).fill(1);
    const once = new Float32Array(BAND_COUNT);
    const twice = new Float32Array(BAND_COUNT);
    smoothBands(once, target, 16);
    smoothBands(twice, target, 8);
    smoothBands(twice, target, 8);
    for (let index = 0; index < BAND_COUNT; index += 1) {
      expect(twice[index]).toBeCloseTo(once[index], 5);
    }
  });

  it("rises faster than it falls, and high bands fall faster than low ones", () => {
    const rising = new Float32Array(BAND_COUNT);
    smoothBands(rising, new Float32Array(BAND_COUNT).fill(1), 30);
    const falling = new Float32Array(BAND_COUNT).fill(1);
    smoothBands(falling, new Float32Array(BAND_COUNT), 30);

    expect(rising[0]).toBeGreaterThan(1 - falling[0]);
    expect(falling[BAND_COUNT - 1]).toBeLessThan(falling[0]);
  });
});

describe("bandsToColumns", () => {
  it("keeps a flat spectrum flat and passes through the band values", () => {
    const flat = new Float32Array(BAND_COUNT).fill(0.4);
    const columns = new Float32Array(COLUMN_COUNT);
    bandsToColumns(flat, columns);
    expect(columns.every((value) => Math.abs(value - 0.4) < 1e-6)).toBe(true);

    const ramp = new Float32Array(BAND_COUNT).map((_, index) => index / (BAND_COUNT - 1));
    bandsToColumns(ramp, columns);
    expect(columns[0]).toBeCloseTo(0, 6);
    expect(columns[COLUMN_COUNT - 1]).toBeCloseTo(1, 6);
    for (let column = 1; column < COLUMN_COUNT; column += 1) {
      expect(columns[column]).toBeGreaterThanOrEqual(columns[column - 1] - 1e-6);
    }
  });

  it("never overshoots outside 0..1 around a sharp peak", () => {
    const spike = new Float32Array(BAND_COUNT);
    spike[12] = 1;
    const columns = new Float32Array(COLUMN_COUNT);
    bandsToColumns(spike, columns);
    expect(columns.every((value) => value >= 0 && value <= 1)).toBe(true);
    expect(Math.max(...columns)).toBeGreaterThan(0.9);
  });

  it("interpolates midway between two bands", () => {
    expect(catmullRom(0, 0, 1, 1, 0.5)).toBeCloseTo(0.5, 6);
    expect(catmullRom(0.3, 0.3, 0.3, 0.3, 0.37)).toBeCloseTo(0.3, 6);
  });
});

describe("historyAge", () => {
  it("shows the same history under a ridge just before and just after a push", () => {
    // Just before a push the ridge two frames back reads age 2 - 1 = 1 (in old numbering).
    // The push makes every stored frame one older, so that same frame is now age 2, which
    // is exactly what the ridge reads with the fraction back at 0.
    const beforePush = historyAge(1, 2, 1);
    const afterPush = historyAge(1, 2, 0);
    expect(afterPush - beforePush).toBeCloseTo(1, 6);
  });

  it("reads newer history as time passes and never goes negative", () => {
    expect(historyAge(3, 2, 0.5)).toBeLessThan(historyAge(3, 2, 0));
    expect(historyAge(0, 2, 0.9)).toBe(0);
    expect(historyAge(2, 2, 7)).toBe(3);
  });
});

describe("layout helpers", () => {
  it("mirrors columns around the centre", () => {
    expect(mirroredColumn(0.5, COLUMN_COUNT)).toBe(0);
    expect(mirroredColumn(0, COLUMN_COUNT)).toBe(COLUMN_COUNT - 1);
    expect(mirroredColumn(1, COLUMN_COUNT)).toBe(COLUMN_COUNT - 1);
    expect(mirroredColumn(0.25, COLUMN_COUNT)).toBeCloseTo(mirroredColumn(0.75, COLUMN_COUNT), 6);
  });

  it("brings ridges to zero at both ends and leaves the middle untouched", () => {
    expect(ridgeEnvelope(0)).toBe(0);
    expect(ridgeEnvelope(1)).toBe(0);
    expect(ridgeEnvelope(Number.NaN)).toBe(0);
    expect(ridgeEnvelope(0.5)).toBe(1);
    expect(ridgeEnvelope(0.3)).toBeGreaterThan(0.95);
    expect(ridgeEnvelope(0.02)).toBeLessThan(0.2);
  });

  it("keeps ambient motion and the sweep crest within 0..1", () => {
    for (let step = 0; step < 200; step += 1) {
      const ambient = ambientRidge(step / 200, step % 9, step * 0.13);
      expect(ambient).toBeGreaterThanOrEqual(0);
      expect(ambient).toBeLessThanOrEqual(1);
    }
    expect(sweepCrest(0.4, 0.4, 0)).toBeCloseTo(1, 6);
    expect(sweepCrest(0.9, 0.1, 0)).toBeLessThan(0.01);
    // Rear rows see the crest slightly later.
    expect(sweepCrest(0.46, 0.4, 2)).toBeCloseTo(1, 6);
  });
});

describe("softCeiling", () => {
  it("leaves coordinates at or below the knee unchanged", () => {
    expect(softCeiling(60, 3, 18)).toBe(60);
    expect(softCeiling(18, 3, 18)).toBe(18);
  });

  it("is continuous, monotonic, and never crosses the top", () => {
    expect(softCeiling(17.999, 3, 18)).toBeCloseTo(18, 2);
    let previous = softCeiling(18, 3, 18);
    for (let y = 17; y > -2000; y -= 7) {
      const value = softCeiling(y, 3, 18);
      expect(value).toBeLessThanOrEqual(previous);
      expect(value).toBeGreaterThanOrEqual(3);
      previous = value;
    }
    expect(softCeiling(Number.NEGATIVE_INFINITY, 3, 18)).toBe(3);
  });
});

describe("rampColor", () => {
  const accent = [96, 165, 210] as const;

  it("runs from a darkened accent to near-white", () => {
    const out = new Uint8ClampedArray(3);
    rampColor(0, accent, 1, out);
    expect(Array.from(out)).toEqual([58, 99, 126]);
    rampColor(0.35, accent, 1, out);
    expect(Array.from(out)).toEqual([96, 165, 210]);
    rampColor(1, accent, 1, out);
    expect(Array.from(out)).toEqual([240, 247, 252]);
  });

  it("gets brighter with t and is neutral when the accent strength is zero", () => {
    const out = new Uint8ClampedArray(3);
    let previous = -1;
    for (let step = 0; step <= 10; step += 1) {
      rampColor(step / 10, accent, 0.5, out);
      const brightness = out[0] + out[1] + out[2];
      expect(brightness).toBeGreaterThanOrEqual(previous);
      previous = brightness;
    }

    const neutral = new Uint8ClampedArray(3);
    const other = new Uint8ClampedArray(3);
    rampColor(0.5, accent, 0, neutral);
    rampColor(0.5, [214, 92, 74], 0, other);
    expect(Array.from(neutral)).toEqual(Array.from(other));
  });
});
