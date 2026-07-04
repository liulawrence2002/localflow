import { listen } from "@tauri-apps/api/event";
import type { CSSProperties } from "react";
import { useEffect, useMemo, useState } from "react";

type NativeDictationPhase = "idle" | "listening" | "processing" | "refining" | "inserted" | "error";

interface NativeDictationPayload {
  phase: NativeDictationPhase;
  message: string;
  level?: number | null;
}

const barWeights = [
  0.24, 0.34, 0.48, 0.68, 0.92, 0.58, 0.76, 0.44, 0.84, 0.62, 0.5, 0.72, 0.4, 0.3, 0.22,
];

const waveWeights = [0.18, -0.62, -0.46, 0.28, 0.74, 0.35, -0.22, -0.5, -0.28, 0.36, 0.66, 0.22];

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
        level: event.payload.level == null ? null : clampLevel(event.payload.level),
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

  const level = clampLevel(payload.level ?? defaultLevelForPhase(payload.phase));
  const wavePath = useMemo(() => buildWavePath(level, payload.phase), [level, payload.phase]);
  const quietWavePath = useMemo(() => buildWavePath(Math.max(0.1, level * 0.42), "idle"), [level]);
  const status =
    payload.phase === "error" ? "Error" : payload.phase === "inserted" ? "Inserted" : "Active";
  const bars = useMemo(
    () =>
      barWeights.map((weight, index) => ({
        id: `${weight}-${index}`,
        scale: 0.16 + Math.min(1, level * 1.12 + weight * 0.52),
        lowScale: 0.12 + Math.min(1, level * 0.72 + weight * 0.34),
        highScale: 0.22 + Math.min(1, level * 1.32 + weight * 0.58),
        delay: `${index * 34}ms`,
      })),
    [level],
  );

  return (
    <main
      className={`voice-overlay voice-overlay--${payload.phase}`}
      aria-label={`LocalFlow voice ${status.toLowerCase()}`}
      style={{ "--overlay-level": level.toFixed(3) } as CSSProperties}
    >
      <div className="voice-overlay__surface">
        <div className="voice-overlay__glow" aria-hidden="true" />
        <svg
          className="voice-overlay__wave"
          viewBox="0 0 244 56"
          role="img"
          aria-label={payload.message}
        >
          <path className="voice-overlay__wave-shadow" d={quietWavePath} pathLength="1" />
          <path className="voice-overlay__wave-live" d={wavePath} pathLength="1" />
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

function defaultLevelForPhase(phase: NativeDictationPhase): number {
  switch (phase) {
    case "processing":
    case "refining":
      return 0.32;
    case "inserted":
      return 0.18;
    case "error":
      return 0.2;
    default:
      return 0.12;
  }
}

function buildWavePath(level: number, phase: NativeDictationPhase): string {
  const width = 232;
  const startX = 6;
  const centerY = 28;
  const amplitude = phase === "idle" ? 6 : 7 + level * 13;
  const points = waveWeights.map((weight, index) => ({
    x: startX + (width / (waveWeights.length - 1)) * index,
    y: centerY + weight * amplitude,
  }));

  return smoothPath(points);
}

function smoothPath(points: Array<{ x: number; y: number }>): string {
  if (points.length === 0) {
    return "";
  }

  return points.slice(1).reduce(
    (path, point, index) => {
      const previous = points[index];
      const controlX = (previous.x + point.x) / 2;
      return `${path} C ${controlX.toFixed(1)} ${previous.y.toFixed(1)}, ${controlX.toFixed(1)} ${point.y.toFixed(
        1,
      )}, ${point.x.toFixed(1)} ${point.y.toFixed(1)}`;
    },
    `M ${points[0].x.toFixed(1)} ${points[0].y.toFixed(1)}`,
  );
}
