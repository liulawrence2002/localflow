import { describe, expect, it } from "vitest";
import {
  displacementForMotion,
  visualsForPhase,
} from "../packages/localflow-sdk/src/react/topology/overlayStateVisuals";
import type { LocalFlowDictationPhase } from "../packages/localflow-sdk/src/types";

const allPhases: LocalFlowDictationPhase[] = [
  "idle",
  "ready",
  "listening",
  "processing",
  "refining",
  "inserted",
  "cancelled",
  "error",
];

describe("visualsForPhase", () => {
  it("drives the terrain from the microphone only while listening", () => {
    expect(visualsForPhase("listening", false).micDrive).toBe(1);
    for (const phase of allPhases.filter((phase) => phase !== "listening")) {
      expect(visualsForPhase(phase, false).micDrive).toBe(0);
    }
  });

  it("keeps silence alive with a small ambient floor while listening", () => {
    const listening = visualsForPhase("listening", false);
    expect(listening.energyFloor).toBeGreaterThan(0);
    expect(listening.energyFloor).toBeLessThan(0.3);
  });

  it("uses a traveling sweep instead of mic input for processing and refining", () => {
    for (const phase of ["processing", "refining"] as const) {
      const visuals = visualsForPhase(phase, false);
      expect(visuals.micDrive).toBe(0);
      expect(visuals.sweep).toBeGreaterThan(0);
      expect(visuals.sweepSpeed).toBeGreaterThan(0);
    }
    // Refining is distinguishable from processing by a slower harmonic shift.
    expect(visualsForPhase("refining", false).sweepSpeed).toBeLessThan(
      visualsForPhase("processing", false).sweepSpeed,
    );
  });

  it("collapses toward a calm line on success and cancellation", () => {
    expect(visualsForPhase("inserted", false).collapse).toBeGreaterThan(0.5);
    expect(visualsForPhase("cancelled", false).collapse).toBe(1);
    expect(visualsForPhase("cancelled", false).accentStrength).toBe(0);
  });

  it("applies restrained state tints", () => {
    const [r, g, b] = visualsForPhase("error", false).accent;
    expect(r).toBeGreaterThan(g);
    expect(r).toBeGreaterThan(b);
    expect(visualsForPhase("error", false).accentStrength).toBeLessThan(0.6);

    const inserted = visualsForPhase("inserted", false).accent;
    expect(inserted[1]).toBeGreaterThan(inserted[0]);
  });

  it("keeps feedback under reduced motion while lowering movement", () => {
    for (const phase of allPhases) {
      const normal = visualsForPhase(phase, false);
      const reduced = visualsForPhase(phase, true);
      expect(reduced.travelSpeed).toBeLessThan(normal.travelSpeed + 0.0001);
      expect(reduced.sweep).toBe(0);
      expect(reduced.accentStrength).toBe(normal.accentStrength);
      expect(reduced.energyFloor).toBe(normal.energyFloor);
    }
    // Speech still registers, just more gently.
    expect(visualsForPhase("listening", true).micDrive).toBeGreaterThan(0);
  });

  it("falls back to ambient parameters for unknown phases", () => {
    const unknown = visualsForPhase("bogus" as LocalFlowDictationPhase, false);
    expect(unknown.micDrive).toBe(0);
    expect(unknown.collapse).toBe(0);
  });
});

describe("displacementForMotion", () => {
  it("halves displacement under reduced motion", () => {
    expect(displacementForMotion(true)).toBeLessThan(displacementForMotion(false));
    expect(displacementForMotion(true)).toBeGreaterThan(0);
  });
});
