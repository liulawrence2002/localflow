export type LocalFlowDictationPhase =
  "idle" | "listening" | "processing" | "refining" | "inserted" | "error";

export interface LocalFlowVoiceState {
  sessionId: string;
  phase: LocalFlowDictationPhase;
  message: string;
  level?: number | null;
  pitch?: number | null;
  brightness?: number | null;
}

export interface LocalFlowTranscriptEvent {
  sessionId: string;
  text: string;
  kind: "quick" | "refined";
  latencyMs?: number;
}

export interface LocalFlowErrorEvent {
  sessionId: string;
  code: string;
  message: string;
  recoverable: boolean;
}

export interface LocalFlowSdkStatus<TSettings = unknown> {
  phase: LocalFlowDictationPhase;
  activeSessionId?: string;
  settings?: TSettings;
  lastTranscript?: string;
  diagnostics: Array<{
    label: string;
    value: string;
    status: "ok" | "warning" | "blocked" | "error";
  }>;
}

export interface LocalFlowStartOptions<TContext = unknown> {
  sessionId?: string;
  context?: TContext;
  audio?: Blob | ArrayBuffer | Float32Array;
}

export interface LocalFlowTranscriptionRequest<TSettings = unknown, TContext = unknown> {
  sessionId: string;
  audio?: Blob | ArrayBuffer | Float32Array;
  context?: TContext;
  settings?: TSettings;
}

export interface LocalFlowTranscriptionResult {
  text: string;
  refinedText?: string;
  timings?: Record<string, number>;
}

export interface LocalFlowTranscriber<TSettings = unknown, TContext = unknown> {
  transcribe(
    request: LocalFlowTranscriptionRequest<TSettings, TContext>,
  ): Promise<LocalFlowTranscriptionResult>;
}

export interface LocalFlowEventMap<TSettings = unknown> {
  voice_state: LocalFlowVoiceState;
  transcript: LocalFlowTranscriptEvent;
  error: LocalFlowErrorEvent;
  status: LocalFlowSdkStatus<TSettings>;
}

export type LocalFlowEventName<TSettings = unknown> = keyof LocalFlowEventMap<TSettings>;

export type LocalFlowUnsubscribe = () => void;

export interface LocalFlowClient<TSettings = unknown, TContext = unknown> {
  startDictation(options?: LocalFlowStartOptions<TContext>): Promise<void>;
  stopDictation(): Promise<void>;
  cancelDictation(): Promise<void>;
  getStatus(): Promise<LocalFlowSdkStatus<TSettings>>;
  saveSettings(settings: TSettings): Promise<LocalFlowSdkStatus<TSettings>>;
  on<TEvent extends LocalFlowEventName<TSettings>>(
    eventName: TEvent,
    handler: (payload: LocalFlowEventMap<TSettings>[TEvent]) => void,
  ): LocalFlowUnsubscribe;
}
