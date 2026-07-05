import { listen } from "@tauri-apps/api/event";
import { LocalFlowOverlay } from "@localflow/sdk/react";
import { useEffect, useState } from "react";
import type { LocalFlowDictationPhase, LocalFlowVoiceState } from "@localflow/sdk";

interface NativeDictationPayload {
  phase: LocalFlowDictationPhase;
  message: string;
  level?: number | null;
  pitch?: number | null;
  brightness?: number | null;
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

  useEffect(() => {
    let mounted = true;
    let cleanup: (() => void) | undefined;

    void listen<NativeDictationPayload>("localflow://native-dictation", (event) => {
      if (!mounted) {
        return;
      }

      setState({
        sessionId: "tauri-native",
        phase: event.payload.phase,
        message: event.payload.message,
        level: event.payload.level == null ? null : clampUnit(event.payload.level),
        pitch: event.payload.pitch == null ? null : clampUnit(event.payload.pitch),
        brightness: event.payload.brightness == null ? null : clampUnit(event.payload.brightness),
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

  return <LocalFlowOverlay state={state} placement="window" />;
}

function clampUnit(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.12;
  }

  return Math.max(0.04, Math.min(1, value));
}
