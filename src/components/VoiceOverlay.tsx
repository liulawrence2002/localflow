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

interface WaveState {
  level: number;
  pitch: number;
  brightness: number;
  targetLevel: number;
  targetPitch: number;
  targetBrightness: number;
  phase: NativeDictationPhase;
}

interface WaveLayer {
  color: string;
  verticalBias: number;
  frequency: number;
  speed: number;
  phase: number;
  lineWidth: number;
  gain: number;
  alpha: number;
  pitchResponse: number;
  detail: number;
}

const waveLayers: WaveLayer[] = [
  {
    color: "56, 63, 72",
    verticalBias: 0,
    frequency: 4.2,
    speed: 1.05,
    phase: 0.2,
    lineWidth: 2.7,
    gain: 1.0,
    alpha: 0.72,
    pitchResponse: 0.08,
    detail: 0.22,
  },
  {
    color: "205, 111, 80",
    verticalBias: -1,
    frequency: 5.8,
    speed: 1.16,
    phase: 1.5,
    lineWidth: 1.25,
    gain: 0.72,
    alpha: 0.32,
    pitchResponse: 1.0,
    detail: 0.36,
  },
  {
    color: "70, 128, 178",
    verticalBias: 1,
    frequency: 5.4,
    speed: 0.98,
    phase: 2.65,
    lineWidth: 1.2,
    gain: 0.74,
    alpha: 0.34,
    pitchResponse: -1.0,
    detail: 0.32,
  },
  {
    color: "174, 131, 70",
    verticalBias: -0.35,
    frequency: 7.1,
    speed: 1.34,
    phase: 4.1,
    lineWidth: 0.95,
    gain: 0.38,
    alpha: 0.28,
    pitchResponse: 0.56,
    detail: 0.48,
  },
];
const waveVerticalCenter = 0.5;
const waveVerticalLimit = 0.32;

export function VoiceOverlay() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const visualRef = useRef<WaveState>({
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

      drawWaveform(context, rect.width, rect.height, visual, timeMs / 1000);
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

function drawWaveform(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: WaveState,
  time: number,
) {
  context.clearRect(0, 0, width, height);

  drawSoftGuide(context, width, height, visual);
  drawEnergyWash(context, width, height, visual);

  for (const layer of waveLayers) {
    drawWaveLayer(context, width, height, visual, time, layer);
  }

  drawPrimaryLine(context, width, height, visual, time);
}

function drawSoftGuide(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: WaveState,
) {
  const centerY = waveCenterY(height);
  const guide = context.createLinearGradient(26, 0, width - 26, 0);
  guide.addColorStop(0, "rgba(69, 79, 88, 0)");
  guide.addColorStop(0.18, `rgba(69, 79, 88, ${0.06 + visual.level * 0.04})`);
  guide.addColorStop(0.5, `rgba(69, 79, 88, ${0.14 + visual.level * 0.08})`);
  guide.addColorStop(0.82, `rgba(69, 79, 88, ${0.06 + visual.level * 0.04})`);
  guide.addColorStop(1, "rgba(69, 79, 88, 0)");

  context.beginPath();
  context.moveTo(28, centerY);
  context.lineTo(width - 28, centerY);
  context.strokeStyle = guide;
  context.lineWidth = 1;
  context.stroke();
}

function drawEnergyWash(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: WaveState,
) {
  const centerY = waveCenterY(height);
  const washHeight = 18 + visual.level * 18;
  const glow = context.createRadialGradient(
    width * 0.5,
    centerY,
    12,
    width * 0.5,
    centerY,
    width * 0.48,
  );
  glow.addColorStop(0, `rgba(${phaseAccentColor(visual.phase)}, ${0.12 + visual.level * 0.1})`);
  glow.addColorStop(
    0.46,
    `rgba(${phaseAccentColor(visual.phase)}, ${0.055 + visual.level * 0.06})`,
  );
  glow.addColorStop(1, "rgba(255, 255, 255, 0)");

  context.fillStyle = glow;
  context.fillRect(0, centerY - washHeight, width, washHeight * 2);
}

function drawWaveLayer(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: WaveState,
  time: number,
  layer: WaveLayer,
) {
  const centerY = waveCenterY(height);
  const level = levelForDraw(visual);
  const pitchBias = (visual.pitch - 0.5) * layer.pitchResponse;
  const pitchLift = layer.verticalBias * pitchBias * height * 0.13;
  const amplitude = Math.min((5.5 + level * 25) * layer.gain, height * waveVerticalLimit);
  const frequency = layer.frequency + visual.brightness * 1.25 + Math.abs(pitchBias) * 1.2;
  const speed = layer.speed + level * 0.36;
  const color = phaseWaveColor(layer.color, visual.phase);

  context.save();
  context.globalCompositeOperation = "source-over";
  context.beginPath();

  for (let x = 22; x <= width - 22; x += 2) {
    const progress = x / width;
    const y =
      centerY +
      pitchLift +
      waveEnvelope(progress) *
        amplitude *
        (Math.sin(progress * Math.PI * frequency + time * speed + layer.phase) * 0.72 +
          Math.sin(
            progress * Math.PI * (frequency * 1.82 + visual.brightness * 1.8) -
              time * speed * 0.84 +
              layer.phase,
          ) *
            layer.detail);
    const boundedY = squashToWaveBand(y, centerY, height);

    if (x === 22) {
      context.moveTo(x, boundedY);
    } else {
      context.lineTo(x, boundedY);
    }
  }

  context.strokeStyle = `rgba(${color}, ${layer.alpha})`;
  context.lineWidth = layer.lineWidth;
  context.lineCap = "round";
  context.lineJoin = "round";
  context.stroke();
  context.restore();
}

function drawPrimaryLine(
  context: CanvasRenderingContext2D,
  width: number,
  height: number,
  visual: WaveState,
  time: number,
) {
  const centerY = waveCenterY(height);
  const level = levelForDraw(visual);
  const pitchTilt = (visual.pitch - 0.5) * height * -0.06;
  const amplitude = Math.min(6 + level * 24, height * 0.26);
  const lineGradient = context.createLinearGradient(0, 0, width, 0);
  lineGradient.addColorStop(0, "rgba(66, 74, 82, 0)");
  lineGradient.addColorStop(0.16, `rgba(66, 74, 82, ${0.38 + level * 0.16})`);
  lineGradient.addColorStop(0.5, `rgba(${phaseAccentColor(visual.phase)}, ${0.52 + level * 0.2})`);
  lineGradient.addColorStop(0.84, `rgba(66, 74, 82, ${0.38 + level * 0.16})`);
  lineGradient.addColorStop(1, "rgba(66, 74, 82, 0)");

  context.beginPath();
  for (let x = 20; x <= width - 20; x += 2) {
    const progress = x / width;
    const y =
      centerY +
      pitchTilt +
      waveEnvelope(progress) *
        amplitude *
        (Math.sin(progress * Math.PI * (4.6 + visual.brightness * 1.6) + time * 1.28) * 0.78 +
          Math.sin(progress * Math.PI * (9.4 + visual.brightness * 2.2) - time * 1.06) *
            0.18 *
            visual.brightness);

    if (x === 20) {
      context.moveTo(x, y);
    } else {
      context.lineTo(x, y);
    }
  }

  context.strokeStyle = lineGradient;
  context.lineWidth = 2.8;
  context.lineCap = "round";
  context.lineJoin = "round";
  context.shadowColor = `rgba(${phaseAccentColor(visual.phase)}, ${0.16 + level * 0.12})`;
  context.shadowBlur = 10;
  context.stroke();
  context.shadowBlur = 0;
}

function waveCenterY(height: number): number {
  return height * waveVerticalCenter;
}

function squashToWaveBand(value: number, centerY: number, height: number): number {
  const limit = Math.max(1, height * waveVerticalLimit);
  return centerY + Math.tanh((value - centerY) / limit) * limit;
}

function waveEnvelope(progress: number): number {
  const base = Math.sin(Math.PI * progress);
  return Math.pow(Math.max(0, base), 0.72) * (0.92 + 0.08 * Math.sin(progress * Math.PI * 6));
}

function levelForDraw(visual: WaveState): number {
  if (visual.phase === "inserted") {
    return 0.14;
  }

  if (visual.phase === "error") {
    return Math.max(visual.level, 0.24);
  }

  if (visual.phase === "processing" || visual.phase === "refining") {
    return Math.max(visual.level, 0.3);
  }

  return visual.level;
}

function phaseAccentColor(phase: NativeDictationPhase): string {
  if (phase === "inserted") {
    return "42, 164, 103";
  }

  if (phase === "error") {
    return "210, 74, 54";
  }

  return "63, 132, 183";
}

function phaseWaveColor(color: string, phase: NativeDictationPhase): string {
  if (phase === "inserted") {
    return "42, 164, 103";
  }

  if (phase === "error") {
    return "210, 74, 54";
  }

  return color;
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
