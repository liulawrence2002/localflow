import { listen } from "@tauri-apps/api/event";
import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";

type NativeDictationPhase = "idle" | "listening" | "processing" | "refining" | "inserted" | "error";

interface NativeDictationPayload {
  phase: NativeDictationPhase;
  message: string;
  level?: number | null;
}

const barWeights = [0.22, 0.38, 0.56, 0.82, 0.48, 0.68, 0.34, 0.74, 0.5, 0.3, 0.42];

export function VoiceOverlay() {
  const [payload, setPayload] = useState<NativeDictationPayload>({
    phase: "idle",
    message: "Idle",
    level: 0.08,
  });

  useEffect(() => {
    let mounted = true;
    let cleanup: (() => void) | undefined;

    void listen<NativeDictationPayload>("localflow://native-dictation", (event) => {
      if (!mounted) {
        return;
      }

      setPayload({
        phase: event.payload.phase,
        message: event.payload.message,
        level: clampLevel(event.payload.level ?? 0.12),
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

  const level = clampLevel(payload.level ?? 0.12);
  const status =
    payload.phase === "error" ? "Error" : payload.phase === "inserted" ? "Inserted" : "Active";
  const bars = useMemo(
    () =>
      barWeights.map((weight, index) => ({
        id: `${weight}-${index}`,
        scale: 0.22 + Math.min(1, level * 1.35 + weight * 0.55),
        lowScale: 0.18 + Math.min(1, level * 0.98 + weight * 0.36),
        highScale: 0.3 + Math.min(1, level * 1.48 + weight * 0.62),
        delay: `${index * 42}ms`,
      })),
    [level],
  );

  return (
    <main
      className={`voice-overlay voice-overlay--${payload.phase}`}
      aria-label={`LocalFlow voice ${status.toLowerCase()}`}
    >
      <div className="voice-overlay__surface">
        <svg
          className="voice-overlay__line"
          viewBox="0 0 210 42"
          role="img"
          aria-label={payload.message}
        >
          <path
            d="M6 22 C 30 5, 47 38, 68 20 S 101 16, 122 22 S 158 31, 181 18 S 197 20, 204 18"
            pathLength="1"
          />
        </svg>
        <div className="voice-overlay__bars" aria-hidden="true">
          {bars.map((bar) => (
            <span
              key={bar.id}
              style={
                {
                  "--bar-scale": bar.scale.toFixed(3),
                  "--bar-scale-low": bar.lowScale.toFixed(3),
                  "--bar-scale-high": bar.highScale.toFixed(3),
                  "--bar-delay": bar.delay,
                } as CSSProperties
              }
            />
          ))}
        </div>
      </div>
    </main>
  );
}

function clampLevel(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.12;
  }

  return Math.max(0.04, Math.min(1, value));
}
