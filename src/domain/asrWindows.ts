export interface AudioWindow {
  index: number;
  startSample: number;
  endSample: number;
  overlapStartSample: number;
  durationMs: number;
}

export function planRollingWindows({
  totalSamples,
  sampleRate,
  windowMs,
  overlapMs,
}: {
  totalSamples: number;
  sampleRate: number;
  windowMs: number;
  overlapMs: number;
}): AudioWindow[] {
  if (totalSamples <= 0) {
    return [];
  }
  if (sampleRate <= 0 || windowMs <= 0 || overlapMs < 0) {
    throw new Error("Invalid rolling window configuration.");
  }
  if (overlapMs >= windowMs) {
    throw new Error("overlapMs must be shorter than windowMs.");
  }

  const windowSamples = Math.max(1, Math.round((windowMs / 1000) * sampleRate));
  const overlapSamples = Math.round((overlapMs / 1000) * sampleRate);
  const stepSamples = Math.max(1, windowSamples - overlapSamples);
  const windows: AudioWindow[] = [];

  for (let startSample = 0; startSample < totalSamples; startSample += stepSamples) {
    const endSample = Math.min(totalSamples, startSample + windowSamples);
    windows.push({
      index: windows.length,
      startSample,
      endSample,
      overlapStartSample: Math.max(0, startSample - overlapSamples),
      durationMs: ((endSample - startSample) / sampleRate) * 1000,
    });

    if (endSample === totalSamples) {
      break;
    }
  }

  return windows;
}
