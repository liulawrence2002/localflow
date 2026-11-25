import { afterEach, describe, expect, it, vi } from "vitest";
import { TopologyRenderer } from "../packages/localflow-sdk/src/react/topology/topologyRenderer";

interface FrameHarness {
  renderer: TopologyRenderer;
  context: ReturnType<typeof createContextStub>;
  runFrame: (timeMs: number) => void;
  pendingFrames: () => number;
  cancelled: number[];
}

function createContextStub() {
  return {
    arc: vi.fn(),
    beginPath: vi.fn(),
    clearRect: vi.fn(),
    fill: vi.fn(),
    lineTo: vi.fn(),
    moveTo: vi.fn(),
    setTransform: vi.fn(),
    stroke: vi.fn(),
    createLinearGradient: vi.fn(() => ({ addColorStop: vi.fn() })),
    fillStyle: "" as string | object,
    globalAlpha: 1,
    globalCompositeOperation: "source-over",
    lineCap: "butt",
    lineJoin: "miter",
    lineWidth: 0,
    strokeStyle: "" as string | object,
  };
}

function createHarness(): FrameHarness {
  const context = createContextStub();
  const canvas = document.createElement("canvas");
  Object.defineProperty(canvas, "clientWidth", { value: 593 });
  Object.defineProperty(canvas, "clientHeight", { value: 91 });
  vi.spyOn(canvas, "getContext").mockReturnValue(context as unknown as CanvasRenderingContext2D);

  const callbacks = new Map<number, (timeMs: number) => void>();
  const cancelled: number[] = [];
  let nextHandle = 1;
  let clock = 0;

  const renderer = new TopologyRenderer({
    requestFrame: (callback) => {
      const handle = nextHandle;
      nextHandle += 1;
      callbacks.set(handle, callback);
      return handle;
    },
    cancelFrame: (handle) => {
      callbacks.delete(handle);
      cancelled.push(handle);
    },
    now: () => clock,
  });
  renderer.attach(canvas);

  return {
    renderer,
    context,
    runFrame: (timeMs: number) => {
      clock = timeMs;
      const entries = [...callbacks.entries()];
      callbacks.clear();
      for (const [, callback] of entries) {
        callback(timeMs);
      }
    },
    pendingFrames: () => callbacks.size,
    cancelled,
  };
}

function setDocumentVisibility(state: DocumentVisibilityState): void {
  Object.defineProperty(document, "visibilityState", {
    configurable: true,
    get: () => state,
  });
  document.dispatchEvent(new Event("visibilitychange"));
}

afterEach(() => {
  // Remove any per-test visibility override so the prototype getter is restored.
  delete (document as { visibilityState?: DocumentVisibilityState }).visibilityState;
  vi.restoreAllMocks();
});

describe("TopologyRenderer", () => {
  it("schedules exactly one animation frame at a time", () => {
    const harness = createHarness();
    harness.renderer.start();
    expect(harness.pendingFrames()).toBe(1);

    // Rapid state changes must not create additional loops.
    harness.renderer.setPhase("listening");
    harness.renderer.setPhase("processing");
    harness.renderer.setPhase("listening");
    expect(harness.pendingFrames()).toBe(1);

    harness.runFrame(16);
    expect(harness.pendingFrames()).toBe(1);
    harness.renderer.dispose();
  });

  it("draws every ridge as an occluding fill and a stroked line", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.runFrame(16);

    expect(harness.context.clearRect).toHaveBeenCalled();
    // Balanced quality: nine ridges, each erasing what it covers and then stroked.
    expect(harness.context.fill.mock.calls.length).toBeGreaterThanOrEqual(9);
    expect(harness.context.stroke.mock.calls.length).toBeGreaterThanOrEqual(9);
    expect(harness.context.lineTo.mock.calls.length).toBeGreaterThan(800);
    harness.renderer.dispose();
  });

  it("keeps every drawn coordinate finite and below the top edge", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureTargets({
      level: 1,
      pitch: 0.5,
      brightness: 0.5,
      bands: new Array(32).fill(1),
    });
    for (let frame = 1; frame <= 40; frame += 1) {
      harness.runFrame(frame * 16);
    }

    let allFinite = true;
    let highest = Number.POSITIVE_INFINITY;
    for (const [x, y] of harness.context.lineTo.mock.calls as Array<[number, number]>) {
      allFinite = allFinite && Number.isFinite(x) && Number.isFinite(y);
      highest = Math.min(highest, y);
    }
    expect(allFinite).toBe(true);
    // Even a full-scale spectrum stays below the top edge of the 91 px pill.
    expect(highest).toBeGreaterThanOrEqual(3);
    harness.renderer.dispose();
  });

  it("follows real spectrum bands", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    const bands = new Array(32).fill(0);
    bands[4] = 1;
    harness.renderer.setFeatureTargets({ level: 0.8, pitch: 0.5, brightness: 0.5, bands });
    for (let frame = 1; frame <= 30; frame += 1) {
      harness.runFrame(frame * 16);
    }

    const current = harness.renderer.currentBands;
    expect(current[4]).toBeGreaterThan(0.3);
    expect(current[25]).toBeLessThan(0.02);
    expect(current[4]).toBeGreaterThan((current[10] ?? Number.NaN) * 5);
    harness.renderer.dispose();
  });

  it("starts every listening phase clean, even straight after processing", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureTargets({
      level: 1,
      pitch: 0.5,
      brightness: 0.5,
      bands: new Array(32).fill(1),
    });
    for (let frame = 1; frame <= 30; frame += 1) {
      harness.runFrame(frame * 16);
    }
    expect(Math.max(...harness.renderer.currentBands)).toBeGreaterThan(0.5);

    // The hotkey is pressed again while the previous dictation is still being transcribed.
    harness.renderer.setPhase("processing");
    harness.renderer.setPhase("listening");
    expect(Math.max(...harness.renderer.currentBands)).toBe(0);
    harness.runFrame(31 * 16);
    expect(Math.max(...harness.renderer.currentBands)).toBeLessThan(0.05);
    harness.renderer.dispose();
  });

  it("keeps the front ridge continuous when processing begins", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureTargets({
      level: 1,
      pitch: 0.5,
      brightness: 0.5,
      bands: new Array(32).fill(1),
    });
    for (let frame = 1; frame <= 40; frame += 1) {
      harness.runFrame(frame * 16);
    }
    const frontRidge = () => {
      const calls = harness.context.lineTo.mock.calls as Array<[number, number]>;
      // The front ridge is drawn last; take the highest point of the final 132 lineTo calls.
      return Math.min(...calls.slice(-132).map(([, y]) => y));
    };
    const before = frontRidge();

    harness.renderer.setPhase("processing");
    harness.context.lineTo.mockClear();
    harness.runFrame(41 * 16);
    const after = frontRidge();

    // One 16 ms frame later the ridge (about 50 px tall here) has only eased down a little:
    // it decays from the live spectrum instead of dropping to nothing or to a stale frame.
    expect(after).toBeGreaterThanOrEqual(before);
    expect(after - before).toBeLessThan(8);
    harness.renderer.dispose();
  });

  it("synthesises a spectrum when no bands are provided", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureTargets({ level: 0.9, pitch: 0.5, brightness: 0.5 });
    for (let frame = 1; frame <= 30; frame += 1) {
      harness.runFrame(frame * 16);
    }

    expect(Math.max(...harness.renderer.currentBands)).toBeGreaterThan(0.3);
    harness.renderer.dispose();
  });

  it("ignores the microphone while processing", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("processing");
    harness.renderer.setFeatureTargets({
      level: 1,
      pitch: 0.5,
      brightness: 0.5,
      bands: new Array(32).fill(1),
    });
    for (let frame = 1; frame <= 30; frame += 1) {
      harness.runFrame(frame * 16);
    }

    expect(Math.max(...harness.renderer.currentBands)).toBe(0);
    harness.renderer.dispose();
  });

  it("draws fewer points at low quality and fewer ridges with reduced motion", () => {
    const balanced = createHarness();
    balanced.renderer.start();
    balanced.runFrame(16);
    const balancedPoints = balanced.context.lineTo.mock.calls.length;
    balanced.renderer.dispose();

    const low = createHarness();
    low.renderer.setQuality("low");
    low.renderer.start();
    low.runFrame(16);
    expect(low.context.lineTo.mock.calls.length).toBeLessThan(balancedPoints / 2);
    low.renderer.dispose();

    const reduced = createHarness();
    reduced.renderer.setReducedMotion(true);
    reduced.renderer.start();
    reduced.runFrame(16);
    expect(reduced.context.lineTo.mock.calls.length).toBeLessThan(balancedPoints / 2);
    reduced.renderer.dispose();
  });

  it("cancels the pending frame on stop and dispose", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.stop();
    expect(harness.pendingFrames()).toBe(0);
    expect(harness.cancelled.length).toBe(1);

    harness.renderer.start();
    harness.renderer.dispose();
    expect(harness.pendingFrames()).toBe(0);
  });

  it("does not restart after dispose", () => {
    const harness = createHarness();
    harness.renderer.dispose();
    harness.renderer.start();
    expect(harness.pendingFrames()).toBe(0);
  });

  it("pauses while the document is hidden and resumes when visible", () => {
    const harness = createHarness();
    harness.renderer.start();

    setDocumentVisibility("hidden");
    expect(harness.renderer.isRunning).toBe(false);
    expect(harness.pendingFrames()).toBe(0);

    setDocumentVisibility("visible");
    expect(harness.renderer.isRunning).toBe(true);
    expect(harness.pendingFrames()).toBe(1);
    harness.renderer.dispose();
  });

  it("removes environment listeners on dispose", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.dispose();

    setDocumentVisibility("visible");
    expect(harness.renderer.isRunning).toBe(false);
    expect(harness.pendingFrames()).toBe(0);
  });

  it("resets smoothed dynamics when a new session starts", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureTargets({ level: 1, pitch: 0.5, brightness: 0.5 });
    for (let frame = 1; frame <= 30; frame += 1) {
      harness.runFrame(frame * 16);
    }
    expect(harness.renderer.currentLevel).toBeGreaterThan(0.5);

    expect(Math.max(...harness.renderer.currentBands)).toBeGreaterThan(0.2);

    harness.renderer.setPhase("inserted");
    harness.renderer.setPhase("listening");
    expect(harness.renderer.currentLevel).toBeLessThan(0.15);
    // The previous session's spectrum is gone too, and stale targets cannot bring it back.
    expect(Math.max(...harness.renderer.currentBands)).toBe(0);
    harness.runFrame(31 * 16);
    expect(Math.max(...harness.renderer.currentBands)).toBeLessThan(0.05);
    harness.renderer.dispose();
  });

  it("polls a feature source without requiring target pushes", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureSource(() => ({ level: 1, pitch: 0.5, brightness: 0.5 }));
    for (let frame = 1; frame <= 30; frame += 1) {
      harness.runFrame(frame * 16);
    }
    expect(harness.renderer.currentLevel).toBeGreaterThan(0.5);
    harness.renderer.dispose();
  });

  it("survives invalid feature values without NaN poisoning", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.renderer.setPhase("listening");
    harness.renderer.setFeatureTargets({
      level: Number.NaN,
      pitch: Number.POSITIVE_INFINITY,
      brightness: null,
    });
    harness.runFrame(16);
    harness.runFrame(32);
    expect(Number.isFinite(harness.renderer.currentLevel)).toBe(true);

    harness.renderer.setFeatureTargets({
      level: 0.8,
      pitch: 0.5,
      brightness: 0.5,
      bands: [Number.NaN, Number.POSITIVE_INFINITY, -4, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5],
    });
    harness.runFrame(48);
    harness.runFrame(64);
    expect(harness.renderer.currentBands.every((band) => Number.isFinite(band))).toBe(true);
    expect(harness.renderer.currentBands.every((band) => band >= 0 && band <= 1)).toBe(true);
    harness.renderer.dispose();
  });
});
