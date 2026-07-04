import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef, useState } from "react";

type NativeDictationPhase = "idle" | "listening" | "processing" | "refining" | "inserted" | "error";

interface NativeDictationPayload {
  phase: NativeDictationPhase;
  message: string;
  level?: number | null;
  pitch?: number | null;
  brightness?: number | null;
}

interface RibbonState {
  level: number;
  pitch: number;
  brightness: number;
  targetLevel: number;
  targetPitch: number;
  targetBrightness: number;
  phase: NativeDictationPhase;
}

interface RibbonLayer {
  color: string;
  direction: -1 | 1;
  frequency: number;
  speed: number;
  phase: number;
  strands: number;
  gain: number;
  spread: number;
  alpha: number;
  pitchResponse: number;
}

const ribbonLayers: RibbonLayer[] = [
  {
    color: "255, 77, 35",
    direction: -1,
    frequency: 8.4,
    speed: 1.32,
    phase: 0.1,
    strands: 18,
    gain: 1.05,
    spread: 1.0,
    alpha: 0.12,
    pitchResponse: 1.05,
  },
  {
    color: "255, 154, 34",
    direction: -1,
    frequency: 6.2,
    speed: 1.08,
    phase: 1.35,
    strands: 14,
    gain: 0.78,
    spread: 0.82,
    alpha: 0.11,
    pitchResponse: 0.74,
  },
  {
    color: "37, 154, 255",
    direction: 1,
    frequency: 7.2,
    speed: 1.18,
    phase: 2.35,
    strands: 18,
    gain: 1.08,
    spread: 1.06,
    alpha: 0.13,
    pitchResponse: -1.05,
  },
  {
    color: "79, 77, 255",
    direction: 1,
    frequency: 5.4,
    speed: 0.92,
    phase: 3.15,
    strands: 15,
    gain: 0.82,
    spread: 0.92,
    alpha: 0.12,
    pitchResponse: -0.7,
  },
  {
    color: "255, 78, 210",
    direction: -1,
    frequency: 4.6,
    speed: 0.86,
    phase: 4.2,
    strands: 8,
    gain: 0.34,
    spread: 0.34,
    alpha: 0.22,
    pitchResponse: 0.18,
  },
];

export function VoiceOverlay() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const visualRef = useRef<RibbonState>({
    level: 0.12,
    pitch: 0.5,
    brightness: 0.35,
    targetLevel: 0.12,
    targetPitch: 0.5,
    targetBrightness: 0.35,
    phase: "idle",
  });
  const [payload, setPayload] = useState<NativeDictationPayload>({
    phase: "idle",
    message: "Idle",
    level: 0.08,
    pitch: 0.5,
    brightness: 0.35,
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

  const level = clampUnit(payload.level ?? defaultLevelForPhase(payload.phase));
  const pitch = clampUnit(payload.pitch ?? defaultPitchForPhase(payload.phase));
  const brightness = clampUnit(payload.brightness ?? defaultBrightnessForPhase(payload.phase));

  useEffect(() => {
    const visual = visualRef.current;
    visual.targetLevel = level;
    visual.targetPitch = pitch;
    visual.targetBrightness = brightness;
    visual.phase = payload.phase;
  }, [brightness, level, payload.phase, pitch]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) {
      return undefined;
    }

    const context = canvas.getContext("2d");
    if (!context) {
      return undefined;
    }

    let animationFrame = 0;

    const resizeCanvas = () => {
      const rect = canvas.getBoundingClientRect();
      const scale = window.devicePixelRatio || 1;
      canvas.width = Math.round(rect.width * scale);
      canvas.height = Math.round(rect.height * scale);
      context.setTransform(scale, 0, 0, scale, 0, 0);
    };

    const drawFrame = (timeMs: number) => {
      const rect = canvas.getBoundingClientRect();
      const visual = visualRef.current;
      visual.level = lerp(visual.level, visual.targetLevel, 0.16);
      visual.pitch = lerp(visual.pitch, visual.targetPitch, 0.12);
      visual.brightness = lerp(visual.brightness, visual.targetBrightness, 0.12);

      drawRibbon(context, rect.width, rect.height, visual, timeMs / 1000);
      animationFrame = window.requestAnimationFrame(drawFrame);
    };

    resizeCanvas();
    window.addEventListener("resize", resizeCanvas);
    animationFrame = window.requestAnimationFrame(drawFrame);

    return () => {
      window.cancelAnimationFrame(animationFrame);
      window.removeEventListener("resize", resizeCanvas);
    };
  }, []);

  const status =
    payload.phase === "error" ? "Error" : payload.phase === "inserted" ? "Inserted" : "Active";

  return (
    <main
      className={`voice-overlay voice-overlay--${payload.phase}`}
      aria-label={`LocalFlow voice ${status.toLowerCase()}`}
    >
      <div className="voice-overlay__surface">
        <canvas className="voice-overlay__canvas" ref={canvasRef} aria-label={payload.message} />
        <span className="voice-overlay__status" aria-hidden="true" />
      </div>
    </main>
  );
}

function drawRibbon(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: RibbonState,
  time: number,
) {
  context.clearRect(0, 0, width, height);

  drawBackground(context, width, height, visual);
  drawCenterGlow(context, width, height, visual);

  for (const layer of ribbonLayers) {
    drawRibbonLayer(context, width, height, visual, time, layer);
  }

  drawCenterLine(context, width, height, visual, time);
}

function drawBackground(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: RibbonState,
) {
  const gradient = context.createLinearGradient(0, 0, width, height);
  gradient.addColorStop(0, phaseBackgroundTop(visual.phase));
  gradient.addColorStop(0.5, "#07090c");
  gradient.addColorStop(1, phaseBackgroundBottom(visual.phase));
  context.fillStyle = gradient;
  context.fillRect(0, 0, width, height);
}

function drawCenterGlow(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: RibbonState,
) {
  const centerY = height * 0.52;
  const glow = context.createLinearGradient(0, centerY - 18, 0, centerY + 18);
  glow.addColorStop(0, `rgba(255, 99, 53, ${0.08 + visual.pitch * 0.08})`);
  glow.addColorStop(0.5, `rgba(243, 73, 220, ${0.16 + visual.level * 0.14})`);
  glow.addColorStop(1, `rgba(30, 139, 255, ${0.1 + (1 - visual.pitch) * 0.1})`);
  context.fillStyle = glow;
  context.fillRect(0, centerY - 20, width, 40);
}

function drawRibbonLayer(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: RibbonState,
  time: number,
  layer: RibbonLayer,
) {
  const centerY = height * 0.52;
  const level = visual.phase === "inserted" ? 0.18 : visual.level;
  const phaseGain = visual.phase === "error" ? 0.72 : 1;
  const pitchBias = (visual.pitch - 0.5) * layer.pitchResponse;
  const pitchGain = 0.72 + Math.abs(pitchBias) * 0.95;
  const amplitude = (12 + level * 46) * layer.gain * pitchGain * phaseGain;
  const frequency = layer.frequency + visual.brightness * 2.4 + visual.pitch * layer.pitchResponse;
  const speed = layer.speed + level * 0.72;
  const strandCount = layer.strands;

  context.save();
  context.globalCompositeOperation = "screen";

  for (let strand = 0; strand < strandCount; strand += 1) {
    const strandPosition = strandCount === 1 ? 0 : strand / (strandCount - 1);
    const offset = (strandPosition - 0.5) * amplitude * layer.spread;
    const alpha = layer.alpha * (0.38 + strandPosition * 0.86);

    context.beginPath();
    for (let x = 0; x <= width; x += 3) {
      const progress = x / width;
      const envelope =
        0.34 +
        0.66 *
          Math.pow(
            Math.sin(Math.PI * progress) *
              (0.76 + 0.24 * Math.sin(progress * Math.PI * 5.0 + layer.phase)),
            1.35,
          );
      const mainWave = Math.sin(progress * Math.PI * frequency + time * speed + layer.phase);
      const detailWave = Math.sin(
        progress * Math.PI * (frequency * 2.18 + visual.brightness * 4.2) -
          time * (speed * 1.22) +
          strand * 0.21,
      );
      const spikeWave = Math.pow(
        Math.max(
          0,
          Math.sin(progress * Math.PI * (frequency * 0.62 + 1.2) + time * 0.7 + layer.phase),
        ),
        4,
      );
      const pitchShape =
        layer.direction === -1
          ? spikeWave * visual.pitch * 0.95
          : spikeWave * (1 - visual.pitch) * 0.9;
      const displacement =
        layer.direction *
        envelope *
        amplitude *
        (mainWave * 0.52 + detailWave * 0.28 + pitchShape + pitchBias * 0.32);
      const y = centerY + displacement + offset;

      if (x === 0) {
        context.moveTo(x, y);
      } else {
        context.lineTo(x, y);
      }
    }

    context.strokeStyle = `rgba(${phaseColor(layer.color, visual.phase)}, ${alpha})`;
    context.lineWidth = 0.7 + strandPosition * 0.48;
    context.stroke();
  }

  context.restore();
}

function drawCenterLine(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: RibbonState,
  time: number,
) {
  const centerY = height * 0.52;
  const lineGradient = context.createLinearGradient(0, 0, width, 0);
  lineGradient.addColorStop(0, "rgba(24, 140, 255, 0.1)");
  lineGradient.addColorStop(0.45, `rgba(255, 101, 221, ${0.32 + visual.level * 0.28})`);
  lineGradient.addColorStop(1, "rgba(255, 135, 36, 0.12)");

  context.beginPath();
  for (let x = 0; x <= width; x += 4) {
    const progress = x / width;
    const y =
      centerY +
      Math.sin(progress * Math.PI * (5.4 + visual.brightness * 2.1) + time * 1.5) *
        (1.5 + visual.level * 4.2);

    if (x === 0) {
      context.moveTo(x, y);
    } else {
      context.lineTo(x, y);
    }
  }

  context.strokeStyle = lineGradient;
  context.lineWidth = 2.2;
  context.shadowColor = "rgba(255, 76, 225, 0.42)";
  context.shadowBlur = 8;
  context.stroke();
  context.shadowBlur = 0;
}

function phaseColor(color: string, phase: NativeDictationPhase): string {
  if (phase === "inserted") {
    return "51, 224, 139";
  }

  if (phase === "error") {
    return "255, 78, 54";
  }

  return color;
}

function phaseBackgroundTop(phase: NativeDictationPhase): string {
  if (phase === "inserted") {
    return "#03130d";
  }

  if (phase === "error") {
    return "#180604";
  }

  return "#050608";
}

function phaseBackgroundBottom(phase: NativeDictationPhase): string {
  if (phase === "inserted") {
    return "#061c13";
  }

  if (phase === "error") {
    return "#210805";
  }

  return "#0b0f18";
}

function clampUnit(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.12;
  }

  return Math.max(0.04, Math.min(1, value));
}

function defaultLevelForPhase(phase: NativeDictationPhase): number {
  switch (phase) {
    case "processing":
    case "refining":
      return 0.34;
    case "inserted":
      return 0.18;
    case "error":
      return 0.24;
    default:
      return 0.12;
  }
}

function defaultPitchForPhase(phase: NativeDictationPhase): number {
  switch (phase) {
    case "processing":
    case "refining":
      return 0.58;
    case "error":
      return 0.74;
    default:
      return 0.5;
  }
}

function defaultBrightnessForPhase(phase: NativeDictationPhase): number {
  switch (phase) {
    case "processing":
    case "refining":
      return 0.56;
    case "error":
      return 0.78;
    default:
      return 0.35;
  }
}

function lerp(current: number, target: number, amount: number): number {
  return current + (target - current) * amount;
}
