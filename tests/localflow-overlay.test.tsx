import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LocalFlowOverlay } from "@localflow/sdk/react";

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
