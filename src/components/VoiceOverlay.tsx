import { listen } from "@tauri-apps/api/event";
import { LocalFlowOverlay } from "@localflow/sdk/react";
import { useEffect, useRef, useState } from "react";
import type { LocalFlowDictationPhase, LocalFlowVoiceState } from "@localflow/sdk";

interface NativeDictationPayload {
  phase: LocalFlowDictationPhase;
  message: string;
  level?: number | null;
  pitch?: number | null;
  brightness?: number | null;
  /** Spectrum bands as bytes (0..255), low to high frequency. */
  bands?: number[] | null;
}

interface OverlayFeatures {
  level: number | null;
  pitch: number | null;
  brightness: number | null;
  bands: number[] | null;
}

const idleState: LocalFlowVoiceState = {
  sessionId: "tauri-native",
  phase: "idle",
  message: "Idle",
  level: 0.08,
  pitch: 0.5,
  brightness: 0.35,
};

export function VoiceOverlay() {
  const [state, setState] = useState<LocalFlowVoiceState>(idleState);
  // Audio features arrive about 36 times a second; route them through a ref polled by the
  // renderer so React only re-renders on actual phase/message transitions.
  const featuresRef = useRef<OverlayFeatures | null>(null);

  useEffect(() => {
    let mounted = true;
    let cleanup: (() => void) | undefined;

    void listen<NativeDictationPayload>("localflow://native-dictation", (event) => {
      if (!mounted) {
        return;
      }

      const { phase, message, level, pitch, brightness, bands } = event.payload;

      if (level != null || pitch != null || brightness != null || bands != null) {
        // An audio-feature event. It only feeds the picture and never changes the phase:
        // the meter thread can emit one last frame after "processing" has been announced,
        // and that must not flip the overlay back to "listening".
        featuresRef.current = {
          level: level == null ? null : clampUnit(level),
          pitch: pitch == null ? null : clampUnit(pitch),
          brightness: brightness == null ? null : clampUnit(brightness),
          bands: Array.isArray(bands) ? bands.map(byteToUnit) : null,
        };
        return;
      }

      if (phase === "listening" || phase === "ready") {
        // A new session must not start from the previous session's last frame.
        featuresRef.current = null;
      }

      setState((previous) => {
        if (previous.phase === phase && previous.message === message) {
          return previous;
        }
        return { sessionId: "tauri-native", phase, message };
      });
    }).then((unlisten) => {
      if (mounted) {
        cleanup = unlisten;
      } else {
        unlisten();
      }
    });

    return () => {
      mounted = false;
      cleanup?.();
    };
  }, []);

  return <LocalFlowOverlay state={state} placement="window" featuresRef={featuresRef} />;
}

/** The native layer sends spectrum bands as bytes (0..255); the renderer takes 0..1. */
function byteToUnit(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(1, value / 255)) : 0;
}

function clampUnit(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.12;
  }

  return Math.max(0.04, Math.min(1, value));
}
