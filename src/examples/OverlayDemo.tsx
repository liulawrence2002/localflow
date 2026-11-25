import { LocalFlowOverlay } from "@localflow/sdk/react";
import { useEffect, useRef, useState } from "react";
import type { LocalFlowDictationPhase, LocalFlowVoiceState } from "@localflow/sdk";

/**
 * Development-only preview of the voice overlay, reachable at `?view=overlay-demo`.
 *
 * It needs no microphone and no native app: a small deterministic generator produces
 * speech-like spectrum bands (syllables, moving formants, the occasional consonant burst)
 * like those the native analyser produces, so the visual design can be checked in a
 * browser. Add `&phase=processing` (or any phase) to hold one phase; otherwise the demo
 * cycles through a whole dictation.
 */

const BAND_COUNT = 32;

const CYCLE: Array<{ phase: LocalFlowDictationPhase; ms: number }> = [
  { phase: "ready", ms: 900 },
  { phase: "listening", ms: 6500 },
  { phase: "processing", ms: 2200 },
  { phase: "inserted", ms: 1400 },
];

interface DemoFeatures {
  level: number;
  pitch: number;
  brightness: number;
  bands: number[];
}

/** Speech-like features at time `t` seconds, with spectrum bands in 0..1. */
function speechFeatures(t: number): DemoFeatures {
  // About four syllables a second, with a pause every couple of seconds.
  const syllable = Math.max(0, Math.sin(t * Math.PI * 4.1)) ** 0.6;
  const phrase = Math.sin(t * 0.9) > -0.72 ? 1 : 0;
  const loudness = syllable * phrase * (0.7 + 0.3 * Math.sin(t * 1.7 + 1));

  // Vowels drift between formant positions; consonants are brief high-frequency bursts.
  const vowel = 0.5 + 0.5 * Math.sin(t * 2.3);
  const consonant = Math.max(0, Math.sin(t * Math.PI * 4.1 + 2.6)) ** 6 * phrase;
  const pitch = 0.32 + 0.14 * Math.sin(t * 1.3) + 0.06 * Math.sin(t * 5.1);

  const bumps: Array<[number, number, number]> = [
    [2 + pitch * 4, 1.3, 0.95],
    [8 + vowel * 5, 1.8, 0.85],
    [15 + (1 - vowel) * 5, 2.0, 0.6],
    [23 + vowel * 3, 2.6, 0.32],
  ];

  const bands: number[] = [];
  for (let index = 0; index < BAND_COUNT; index += 1) {
    let value = 0;
    for (const [centre, width, gain] of bumps) {
      const distance = (index - centre) / width;
      value += gain * Math.exp(-0.5 * distance * distance);
    }
    const hiss = consonant * Math.exp(-0.5 * ((index - 27) / 3.5) ** 2);
    const magnitude = Math.min(1, value * loudness + hiss * 0.8);
    bands.push(magnitude);
  }

  return {
    level: Math.min(1, 0.05 + loudness * 0.8 + consonant * 0.3),
    pitch,
    brightness: 0.3 + 0.3 * vowel + 0.4 * consonant,
    bands,
  };
}

function phaseFromQuery(): LocalFlowDictationPhase | null {
  const value = new URLSearchParams(window.location.search).get("phase");
  const phases: LocalFlowDictationPhase[] = [
    "ready",
    "idle",
    "cancelled",
    "listening",
    "processing",
    "refining",
    "inserted",
    "error",
  ];
  return phases.find((phase) => phase === value) ?? null;
}

export function OverlayDemo() {
  const fixedPhase = useRef(phaseFromQuery()).current;
  const [phase, setPhase] = useState<LocalFlowDictationPhase>(fixedPhase ?? "ready");
  const featuresRef = useRef<DemoFeatures | null>(null);

  useEffect(() => {
    const startedAt = performance.now();
    let frame = 0;
    const tick = () => {
      featuresRef.current = speechFeatures((performance.now() - startedAt) / 1000);
      frame = window.requestAnimationFrame(tick);
    };
    frame = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(frame);
  }, []);

  useEffect(() => {
    if (fixedPhase) {
      // A held phase other than listening still needs some captured sound to show.
      if (fixedPhase !== "listening") {
        setPhase("listening");
        const timer = window.setTimeout(() => setPhase(fixedPhase), 1600);
        return () => window.clearTimeout(timer);
      }
      return undefined;
    }

    let step = 0;
    let timer = 0;
    const advance = () => {
      const current = CYCLE[step % CYCLE.length] ?? { phase: "ready", ms: 900 };
      setPhase(current.phase);
      timer = window.setTimeout(() => {
        step = (step + 1) % CYCLE.length;
        advance();
      }, current.ms);
    };
    advance();
    return () => window.clearTimeout(timer);
  }, [fixedPhase]);

  const state: LocalFlowVoiceState = {
    sessionId: "overlay-demo",
    phase,
    message: `Demo: ${phase}`,
  };

  return (
    <div
      style={{
        alignItems: "center",
        background: "linear-gradient(135deg, #1b2a3a, #3a2f4a 55%, #20323a)",
        display: "grid",
        height: "100vh",
        justifyItems: "center",
        width: "100vw",
      }}
    >
      <LocalFlowOverlay state={state} placement="in-app" featuresRef={featuresRef} />
    </div>
  );
}
