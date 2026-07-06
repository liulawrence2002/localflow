import { useEffect, useRef } from "react";
import type { LocalFlowDictationPhase, LocalFlowVoiceState } from "../types";
import {
  TopologyRenderer,
  type AudioFeatureTargets,
  type TopologyQuality,
} from "./topology/topologyRenderer";
import "./LocalFlowOverlay.css";

export interface LocalFlowOverlayProps {
  state?: LocalFlowVoiceState;
  placement?: "window" | "in-app";
  hiddenWhenIdle?: boolean;
  className?: string;
  ariaLabelPrefix?: string;
  /**
   * Optional high-frequency feature channel. When provided, the renderer polls
   * this ref once per animation frame, so ~18 Hz microphone events never need a
   * React re-render. Falls back to `state.level/pitch/brightness` otherwise.
   */
  featuresRef?: React.RefObject<AudioFeatureTargets | null>;
  /** Rendering quality; "low" halves point density and caps the frame rate. */
  quality?: TopologyQuality;
}

const idleState: LocalFlowVoiceState = {
  sessionId: "localflow-idle",
  phase: "idle",
  message: "Idle",
  level: 0.08,
  pitch: 0.5,
  brightness: 0.35,
};

export function LocalFlowOverlay({
  state = idleState,
  placement = "window",
  hiddenWhenIdle = false,
  className,
  ariaLabelPrefix = "LocalFlow voice",
  featuresRef,
  quality = "balanced",
}: LocalFlowOverlayProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const rendererRef = useRef<TopologyRenderer | null>(null);

  const phase = state.phase;
  const hidden = hiddenWhenIdle && phase === "idle";

  useEffect(() => {
    if (hidden) {
      return undefined;
    }
    const canvas = canvasRef.current;
    if (!canvas) {
      return undefined;
    }

    const renderer = new TopologyRenderer();
    rendererRef.current = renderer;
    renderer.attach(canvas);
    if (typeof window !== "undefined" && typeof window.matchMedia === "function") {
      renderer.setReducedMotion(window.matchMedia("(prefers-reduced-motion: reduce)").matches);
    }
    renderer.start();

    return () => {
      rendererRef.current = null;
      renderer.dispose();
    };
  }, [hidden]);

  useEffect(() => {
    rendererRef.current?.setQuality(quality);
  }, [quality, hidden]);

  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer) {
      return;
    }
    renderer.setPhase(phase);
    if (featuresRef) {
      renderer.setFeatureSource(() => featuresRef.current);
    } else {
      renderer.setFeatureSource(null);
      renderer.setFeatureTargets({
        level: state.level,
        pitch: state.pitch,
        brightness: state.brightness,
      });
    }
  }, [phase, state.level, state.pitch, state.brightness, featuresRef, hidden]);

  if (hidden) {
    return null;
  }

  const status = statusLabel(phase);
  const classes = [
    "voice-overlay",
    `voice-overlay--${phase}`,
    `voice-overlay--${placement}`,
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <main className={classes} aria-label={`${ariaLabelPrefix} ${status.toLowerCase()}`}>
      <div className="voice-overlay__surface">
        <canvas className="voice-overlay__canvas" ref={canvasRef} aria-label={state.message} />
        <span className="voice-overlay__status" aria-hidden="true" />
      </div>
    </main>
  );
}

function statusLabel(phase: LocalFlowDictationPhase): string {
  switch (phase) {
    case "error":
      return "Error";
    case "inserted":
      return "Inserted";
    case "ready":
      return "Ready";
    default:
      return "Active";
  }
}
