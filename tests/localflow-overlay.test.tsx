import { render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LocalFlowOverlay } from "@localflow/sdk/react";

describe("LocalFlowOverlay", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(createCanvasContextStub());
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(() => 1);
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => undefined);
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
});

function createCanvasContextStub() {
  const gradient = {
    addColorStop: vi.fn(),
  };

  return {
    beginPath: vi.fn(),
    clearRect: vi.fn(),
    createLinearGradient: vi.fn(() => gradient),
    createRadialGradient: vi.fn(() => gradient),
    fillRect: vi.fn(),
    lineTo: vi.fn(),
    moveTo: vi.fn(),
    restore: vi.fn(),
    save: vi.fn(),
    setTransform: vi.fn(),
    stroke: vi.fn(),
    set fillStyle(_value: unknown) {},
    set globalCompositeOperation(_value: unknown) {},
    set lineCap(_value: unknown) {},
    set lineJoin(_value: unknown) {},
    set lineWidth(_value: unknown) {},
    set shadowBlur(_value: unknown) {},
    set shadowColor(_value: unknown) {},
    set strokeStyle(_value: unknown) {},
  } as unknown as CanvasRenderingContext2D;
}
