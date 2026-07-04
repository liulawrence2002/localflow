export interface VadDecision {
  rms: number;
  isSpeech: boolean;
  endOfSpeech: boolean;
  trailingSilenceMs: number;
}

export interface EndOfSpeechConfig {
  sampleRate: number;
  speechThresholdRms: number;
  endOfSpeechMs: number;
}

export class AudioRingBuffer {
  private samples: number[] = [];
  private droppedSamples = 0;

  constructor(private readonly maxSamples: number) {
    if (!Number.isInteger(maxSamples) || maxSamples <= 0) {
      throw new Error("AudioRingBuffer maxSamples must be a positive integer.");
    }
  }

  push(chunk: ArrayLike<number>): void {
    const next = [...this.samples, ...Array.from(chunk)];
    const overflow = Math.max(0, next.length - this.maxSamples);

    if (overflow > 0) {
      this.droppedSamples += overflow;
      this.samples = next.slice(overflow);
      return;
    }

    this.samples = next;
  }

  clear(): void {
    this.samples = [];
    this.droppedSamples = 0;
  }

  toFloat32Array(): Float32Array {
    return Float32Array.from(this.samples);
  }

  get length(): number {
    return this.samples.length;
  }

  get dropped(): number {
    return this.droppedSamples;
  }
}

export class EndOfSpeechDetector {
  private trailingSilenceMs = 0;
  private observedSpeech = false;

  constructor(private readonly config: EndOfSpeechConfig) {
    if (config.sampleRate <= 0) {
      throw new Error("sampleRate must be positive.");
    }
    if (config.endOfSpeechMs < 0) {
      throw new Error("endOfSpeechMs cannot be negative.");
    }
  }

  update(chunk: ArrayLike<number>): VadDecision {
    const rms = calculateRms(chunk);
    const isSpeech = rms >= this.config.speechThresholdRms;
    const chunkMs = (chunk.length / this.config.sampleRate) * 1000;

    if (isSpeech) {
      this.observedSpeech = true;
      this.trailingSilenceMs = 0;
    } else if (this.observedSpeech) {
      this.trailingSilenceMs += chunkMs;
    }

    return {
      rms,
      isSpeech,
      endOfSpeech: this.observedSpeech && this.trailingSilenceMs >= this.config.endOfSpeechMs,
      trailingSilenceMs: this.trailingSilenceMs,
    };
  }

  reset(): void {
    this.trailingSilenceMs = 0;
    this.observedSpeech = false;
  }
}

export function calculateRms(samples: ArrayLike<number>): number {
  if (samples.length === 0) {
    return 0;
  }

  let sumSquares = 0;
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index] ?? 0;
    sumSquares += sample * sample;
  }

  return Math.sqrt(sumSquares / samples.length);
}

export function downmixToMono(channels: ArrayLike<ArrayLike<number>>): Float32Array {
  if (channels.length === 0) {
    return new Float32Array();
  }

  const frameCount = Math.min(...Array.from(channels, (channel) => channel.length));
  const mono = new Float32Array(frameCount);

  for (let frame = 0; frame < frameCount; frame += 1) {
    let sum = 0;
    for (let channel = 0; channel < channels.length; channel += 1) {
      sum += channels[channel]?.[frame] ?? 0;
    }
    mono[frame] = sum / channels.length;
  }

  return mono;
}

export function resampleLinear(
  samples: ArrayLike<number>,
  sourceSampleRate: number,
  targetSampleRate: number,
): Float32Array {
  if (sourceSampleRate <= 0 || targetSampleRate <= 0) {
    throw new Error("Sample rates must be positive.");
  }

  if (samples.length === 0 || sourceSampleRate === targetSampleRate) {
    return Float32Array.from(Array.from(samples));
  }

  const targetLength = Math.max(
    1,
    Math.round((samples.length * targetSampleRate) / sourceSampleRate),
  );
  const output = new Float32Array(targetLength);
  const ratio = sourceSampleRate / targetSampleRate;

  for (let index = 0; index < targetLength; index += 1) {
    const sourcePosition = index * ratio;
    const leftIndex = Math.floor(sourcePosition);
    const rightIndex = Math.min(samples.length - 1, leftIndex + 1);
    const mix = sourcePosition - leftIndex;
    const left = samples[leftIndex] ?? 0;
    const right = samples[rightIndex] ?? left;
    output[index] = left + (right - left) * mix;
  }

  return output;
}
