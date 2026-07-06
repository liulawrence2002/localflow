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
}

interface OverlayFeatures {
  level: number | null;
  pitch: number | null;
  brightness: number | null;
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
  // Audio features arrive ~18x/second; route them through a ref polled by the
  // renderer so React only re-renders on actual phase/message transitions.
  const featuresRef = useRef<OverlayFeatures | null>(null);

  useEffect(() => {
    let mounted = true;
    let cleanup: (() => void) | undefined;

    void listen<NativeDictationPayload>("localflow://native-dictation", (event) => {
      if (!mounted) {
        return;
      }

      const { phase, message, level, pitch, brightness } = event.payload;

      if (level != null || pitch != null || brightness != null) {
        featuresRef.current = {
          level: level == null ? null : clampUnit(level),
          pitch: pitch == null ? null : clampUnit(pitch),
          brightness: brightness == null ? null : clampUnit(brightness),
        };
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

function clampUnit(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.12;
  }

  return Math.max(0.04, Math.min(1, value));
}
