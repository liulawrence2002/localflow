import { describe, expect, it } from "vitest";
import {
  depthAlpha,
  dotRadius,
  edgeEnvelope,
  heightScale,
  projectPoint,
  rowBaseline,
  softClampY,
  surfaceHeight,
  type SmoothedFeatures,
  type SurfaceMotion,
} from "../packages/localflow-sdk/src/react/topology/topologyMath";

const calmFeatures: SmoothedFeatures = { level: 0.1, pitch: 0.5, brightness: 0.35 };
const loudFeatures: SmoothedFeatures = { level: 0.9, pitch: 0.5, brightness: 0.35 };
const baseMotion: SurfaceMotion = {
  displacement: 1,
  travel: 1,
  sweep: 0,
  sweepPosition: 0,
  collapse: 0,
};
const viewport = { width: 593, height: 91 };

describe("edgeEnvelope", () => {
  it("is zero at both edges and near one in the middle", () => {
    expect(edgeEnvelope(0)).toBe(0);
    expect(edgeEnvelope(1)).toBe(0);
    expect(edgeEnvelope(0.5)).toBeCloseTo(1, 5);
  });

  it("handles out-of-range and invalid input without NaN", () => {
    expect(edgeEnvelope(-1)).toBe(0);
    expect(edgeEnvelope(2)).toBe(0);
    expect(edgeEnvelope(Number.NaN)).toBe(0);
  });
});

describe("surfaceHeight", () => {
  it("is deterministic for identical inputs", () => {
    const a = surfaceHeight(0.37, 0.6, 12.5, loudFeatures, baseMotion);
    const b = surfaceHeight(0.37, 0.6, 12.5, loudFeatures, baseMotion);
    expect(a).toBe(b);
  });

  it("stays bounded for every combination of extremes", () => {
    const extremes = [0, 0.25, 0.5, 0.75, 1];
    for (const level of extremes) {
      for (const pitch of extremes) {
        for (const brightness of extremes) {
          for (let x = 0; x <= 1.0001; x += 0.05) {
            const h = surfaceHeight(x, 0.8, 33.3, { level, pitch, brightness }, baseMotion);
            expect(Number.isFinite(h)).toBe(true);
            expect(Math.abs(h)).toBeLessThan(2.5);
          }
        }
      }
    }
  });

  it("grows the center ridge with speech level", () => {
    const quiet = Math.abs(surfaceHeight(0.5, 1, 4, calmFeatures, baseMotion));
    let loudPeak = 0;
    for (let x = 0.4; x <= 0.6; x += 0.01) {
      loudPeak = Math.max(loudPeak, Math.abs(surfaceHeight(x, 1, 4, loudFeatures, baseMotion)));
    }
    expect(loudPeak).toBeGreaterThan(quiet);
  });

  it("changes ridge spacing (not loudness) when pitch changes", () => {
    // Different pitch must give a different shape...
    const low = surfaceHeight(0.31, 0.5, 4, { ...calmFeatures, pitch: 0.1 }, baseMotion);
    const high = surfaceHeight(0.31, 0.5, 4, { ...calmFeatures, pitch: 0.9 }, baseMotion);
    expect(low).not.toBeCloseTo(high, 5);

    // ...but roughly the same overall energy: pitch is never a loudness axis.
    const energy = (pitch: number) => {
      let sum = 0;
      for (let x = 0.02; x < 1; x += 0.02) {
        sum += Math.abs(surfaceHeight(x, 0.5, 4, { ...calmFeatures, pitch }, baseMotion));
      }
      return sum;
    };
    const ratio = energy(0.9) / energy(0.1);
    expect(ratio).toBeGreaterThan(0.5);
    expect(ratio).toBeLessThan(2);
  });

  it("flattens completely at full collapse", () => {
    const collapsed = surfaceHeight(0.5, 0.7, 9, loudFeatures, { ...baseMotion, collapse: 1 });
    expect(collapsed).toBe(0);
  });

  it("raises a crest at the sweep position when sweeping", () => {
    const motion: SurfaceMotion = { ...baseMotion, sweep: 1, sweepPosition: 0.3 };
    const still: SmoothedFeatures = { level: 0, pitch: 0.5, brightness: 0.35 };
    const atCrest = surfaceHeight(0.3, 0.5, 0, still, motion);
    const away = surfaceHeight(0.3, 0.5, 0, still, { ...motion, sweepPosition: 0.9 });
    expect(atCrest).toBeGreaterThan(away);
  });
});

describe("projection", () => {
  it("places rear rows higher (smaller y) than foreground rows", () => {
    expect(rowBaseline(0, viewport)).toBeLessThan(rowBaseline(1, viewport));
  });

  it("gives rear rows a narrower horizontal span", () => {
    const rearLeft = projectPoint(0, 0, 0, viewport).x;
    const frontLeft = projectPoint(0, 1, 0, viewport).x;
    const rearRight = projectPoint(1, 0, 0, viewport).x;
    const frontRight = projectPoint(1, 1, 0, viewport).x;
    expect(rearRight - rearLeft).toBeLessThan(frontRight - frontLeft);
  });

  it("moves foreground rows more per unit of height", () => {
    expect(heightScale(1, viewport)).toBeGreaterThan(heightScale(0, viewport));
  });

  it("keeps every point inside the pill even for extreme heights", () => {
    for (const depth of [0, 0.5, 1]) {
      for (const height of [-6, -1, 0, 1, 6]) {
        const point = projectPoint(0.5, depth, height, viewport);
        expect(point.y).toBeGreaterThan(0);
        expect(point.y).toBeLessThan(viewport.height);
      }
    }
  });

  it("soft-clamps smoothly rather than hard-clipping", () => {
    const nearEdge = softClampY(viewport.height * 0.9, viewport);
    const pastEdge = softClampY(viewport.height * 3, viewport);
    expect(pastEdge).toBeGreaterThan(nearEdge);
    expect(pastEdge).toBeLessThan(viewport.height);
  });
});

describe("depth styling", () => {
  it("makes foreground rows brighter and larger", () => {
    expect(depthAlpha(1, 0.2)).toBeGreaterThan(depthAlpha(0, 0.2));
    expect(dotRadius(1, 0.2)).toBeGreaterThan(dotRadius(0, 0.2));
  });

  it("brightens and enlarges with activity without exceeding bounds", () => {
    expect(depthAlpha(1, 1)).toBeGreaterThan(depthAlpha(1, 0));
    expect(depthAlpha(1, 5)).toBeLessThanOrEqual(1);
    expect(dotRadius(1, 1)).toBeGreaterThan(dotRadius(1, 0));
  });
});
