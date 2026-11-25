import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LocalFlowOverlay } from "@localflow/sdk/react";
import { TopologyRenderer } from "../packages/localflow-sdk/src/react/topology/topologyRenderer";

describe("LocalFlowOverlay", () => {
  let rafSpy: ReturnType<typeof vi.spyOn>;
  let cafSpy: ReturnType<typeof vi.spyOn>;

  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(createCanvasContextStub());
    rafSpy = vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => 1);
    cafSpy = vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("renders the in-app waveform surface from SDK voice state", () => {
    render(
      <LocalFlowOverlay
        placement="in-app"
        state={{
          sessionId: "mobile",
          phase: "listening",
          message: "Listening",
          level: 0.5,
          pitch: 0.7,
          brightness: 0.6,
        }}
      />,
    );

    expect(screen.getByLabelText("LocalFlow voice active")).toHaveClass("voice-overlay--in-app");
    expect(screen.getByLabelText("Listening")).toBeInTheDocument();
  });

  it("can stay hidden while idle for host-app overlays", () => {
    render(
      <LocalFlowOverlay
        hiddenWhenIdle
        state={{
          sessionId: "mobile",
          phase: "idle",
          message: "Idle",
        }}
      />,
    );

    expect(screen.queryByLabelText("LocalFlow voice active")).not.toBeInTheDocument();
  });

  it("renders the ready pulse state", () => {
    render(
      <LocalFlowOverlay
        state={{
          sessionId: "desktop-ready",
          phase: "ready",
          message: "LocalFlow ready",
        }}
      />,
    );

    expect(screen.getByLabelText("LocalFlow voice ready")).toHaveClass("voice-overlay--ready");
    expect(screen.getByLabelText("LocalFlow ready")).toBeInTheDocument();
  });

  it("runs a single animation loop across rapid state changes", () => {
    const { rerender } = render(
      <LocalFlowOverlay
        state={{ sessionId: "s", phase: "listening", message: "Listening", level: 0.4 }}
      />,
    );

    for (const phase of ["processing", "refining", "listening", "processing"] as const) {
      rerender(<LocalFlowOverlay state={{ sessionId: "s", phase, message: phase }} />);
    }

    // The mocked rAF never fires its callback, so any additional loop would
    // show up as a second requestAnimationFrame call.
    expect(rafSpy).toHaveBeenCalledTimes(1);
  });

  it("follows a change to the reduced-motion setting while it is shown", () => {
    const listeners = new Set<() => void>();
    const query = {
      matches: false,
      media: "(prefers-reduced-motion: reduce)",
      addEventListener: (_type: string, listener: () => void) => listeners.add(listener),
      removeEventListener: (_type: string, listener: () => void) => listeners.delete(listener),
    };
    const originalMatchMedia = window.matchMedia;
    window.matchMedia = (() => query) as unknown as typeof window.matchMedia;
    // Only the values are recorded: the renderer itself is far too large to print.
    const seen: boolean[] = [];
    const original = TopologyRenderer.prototype.setReducedMotion;
    TopologyRenderer.prototype.setReducedMotion = function (reducedMotion: boolean) {
      seen.push(reducedMotion);
    };

    try {
      const { unmount } = render(
        <LocalFlowOverlay state={{ sessionId: "s", phase: "listening", message: "Listening" }} />,
      );
      expect(seen).toEqual([false]);

      query.matches = true;
      for (const listener of [...listeners]) {
        listener();
      }
      expect(seen).toEqual([false, true]);

      unmount();
      expect(listeners.size).toBe(0);
    } finally {
      TopologyRenderer.prototype.setReducedMotion = original;
      window.matchMedia = originalMatchMedia;
    }
  });

  it("cancels the animation frame and stays quiet after unmount", () => {
    const { unmount } = render(
      <LocalFlowOverlay
        state={{ sessionId: "s", phase: "listening", message: "Listening", level: 0.4 }}
      />,
    );

    expect(rafSpy).toHaveBeenCalledTimes(1);
    unmount();
    expect(cafSpy).toHaveBeenCalledWith(1);
  });
});

function createCanvasContextStub() {
  return {
    arc: vi.fn(),
    beginPath: vi.fn(),
    clearRect: vi.fn(),
    fill: vi.fn(),
    lineTo: vi.fn(),
    moveTo: vi.fn(),
    setTransform: vi.fn(),
    stroke: vi.fn(),
    set fillStyle(_value: unknown) {},
    set lineWidth(_value: unknown) {},
    set strokeStyle(_value: unknown) {},
  } as unknown as CanvasRenderingContext2D;
}
