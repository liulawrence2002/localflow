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
    fillStyle: "",
    lineWidth: 0,
    strokeStyle: "",
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

  it("draws depth rows as dot fills every frame", () => {
    const harness = createHarness();
    harness.renderer.start();
    harness.runFrame(16);

    expect(harness.context.clearRect).toHaveBeenCalled();
    // Balanced quality: 11 depth-row fills per frame.
    expect(harness.context.fill.mock.calls.length).toBeGreaterThanOrEqual(11);
    expect(harness.context.arc.mock.calls.length).toBeGreaterThan(100);
    harness.renderer.dispose();
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

    harness.renderer.setPhase("inserted");
    harness.renderer.setPhase("listening");
    expect(harness.renderer.currentLevel).toBeLessThan(0.15);
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
    harness.renderer.dispose();
  });
});
