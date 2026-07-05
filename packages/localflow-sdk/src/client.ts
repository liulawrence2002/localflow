import type {
  LocalFlowClient,
  LocalFlowDictationPhase,
  LocalFlowErrorEvent,
  LocalFlowEventMap,
  LocalFlowEventName,
  LocalFlowSdkStatus,
  LocalFlowStartOptions,
  LocalFlowTranscriber,
  LocalFlowTranscriptionRequest,
  LocalFlowTranscriptionResult,
  LocalFlowUnsubscribe,
  LocalFlowVoiceState,
} from "./types";

type UntypedHandler = (payload: unknown) => void;

interface CreateLocalFlowClientOptions<TSettings, TContext> {
  settings?: TSettings;
  transcriber?: LocalFlowTranscriber<TSettings, TContext>;
  emitMockLevels?: boolean;
  insertedVisibleMs?: number;
}

interface MockTranscriberOptions {
  text?: string;
  refinedText?: string;
  delayMs?: number;
}

interface ActiveSession<TContext> {
  sessionId: string;
  context?: TContext;
  audio?: Blob | ArrayBuffer | Float32Array;
  startedAt: number;
}

export function createLocalFlowClient<TSettings = unknown, TContext = unknown>(
  options: CreateLocalFlowClientOptions<TSettings, TContext> = {},
): LocalFlowClient<TSettings, TContext> {
  const bus = createEventBus<TSettings>();
  const transcriber = options.transcriber ?? createMockTranscriber<TSettings, TContext>();
  const usesMockTranscriber = options.transcriber == null;
  const emitMockLevels = options.emitMockLevels ?? true;
  const insertedVisibleMs = options.insertedVisibleMs ?? 700;
  let settings = options.settings;
  let phase: LocalFlowDictationPhase = "idle";
  let activeSession: ActiveSession<TContext> | undefined;
  let lastTranscript: string | undefined;
  let levelTimer: ReturnType<typeof setInterval> | undefined;
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  function status(): LocalFlowSdkStatus<TSettings> {
    return {
      phase,
      activeSessionId: activeSession?.sessionId,
      settings,
      lastTranscript,
      diagnostics: [
        {
          label: "SDK runtime",
          value: "In-app provider adapter",
          status: "ok",
        },
        {
          label: "Mobile ASR",
          value: usesMockTranscriber ? "Mock provider active" : "Host provider",
          status: usesMockTranscriber ? "warning" : "ok",
        },
      ],
    };
  }

  function emitStatus() {
    bus.emit("status", status());
  }

  function emitVoice(state: LocalFlowVoiceState) {
    bus.emit("voice_state", state);
    emitStatus();
  }

  function setPhase(nextPhase: LocalFlowDictationPhase, message: string, features = {}) {
    phase = nextPhase;
    emitVoice({
      sessionId: activeSession?.sessionId ?? "localflow-idle",
      phase,
      message,
      ...features,
    });
  }

  function clearLevelTimer() {
    if (levelTimer) {
      clearInterval(levelTimer);
      levelTimer = undefined;
    }
  }

  function clearIdleTimer() {
    if (idleTimer) {
      clearTimeout(idleTimer);
      idleTimer = undefined;
    }
  }

  function startMockMeter(sessionId: string) {
    clearLevelTimer();

    if (!emitMockLevels) {
      return;
    }

    levelTimer = setInterval(() => {
      if (!activeSession || activeSession.sessionId !== sessionId || phase !== "listening") {
        clearLevelTimer();
        return;
      }

      const elapsed = (Date.now() - activeSession.startedAt) / 1000;
      emitVoice({
        sessionId,
        phase: "listening",
        message: "Listening",
        level: clampUnit(0.22 + Math.sin(elapsed * 5.7) * 0.18 + Math.sin(elapsed * 2.1) * 0.08),
        pitch: clampUnit(0.52 + Math.sin(elapsed * 2.6) * 0.22),
        brightness: clampUnit(0.42 + Math.sin(elapsed * 3.4) * 0.2),
      });
    }, 120);
  }

  function scheduleIdle(sessionId: string) {
    clearIdleTimer();
    idleTimer = setTimeout(() => {
      if (!activeSession && phase === "inserted") {
        emitVoice({
          sessionId,
          phase: "idle",
          message: "Idle",
          level: 0.08,
          pitch: 0.5,
          brightness: 0.35,
        });
        phase = "idle";
        emitStatus();
      }
    }, insertedVisibleMs);
  }

  async function completeSession(session: ActiveSession<TContext>) {
    const processingStartedAt = Date.now();
    setPhase("processing", "Processing", { level: 0.34, pitch: 0.58, brightness: 0.56 });

    try {
      const result = await transcriber.transcribe({
        sessionId: session.sessionId,
        audio: session.audio,
        context: session.context,
        settings,
      });

      if (activeSession?.sessionId !== session.sessionId) {
        return;
      }

      lastTranscript = result.refinedText ?? result.text;
      const latencyMs = Date.now() - processingStartedAt;
      bus.emit("transcript", {
        sessionId: session.sessionId,
        text: result.text,
        kind: "quick",
        latencyMs,
      });

      if (result.refinedText && result.refinedText !== result.text) {
        bus.emit("transcript", {
          sessionId: session.sessionId,
          text: result.refinedText,
          kind: "refined",
          latencyMs,
        });
      }

      activeSession = undefined;
      setPhase("inserted", "Inserted", { level: 0.18, pitch: 0.5, brightness: 0.35 });
      scheduleIdle(session.sessionId);
    } catch (error) {
      if (activeSession?.sessionId !== session.sessionId) {
        return;
      }

      const event: LocalFlowErrorEvent = {
        sessionId: session.sessionId,
        code: "transcription_failed",
        message: error instanceof Error ? error.message : "Transcription failed.",
        recoverable: true,
      };
      bus.emit("error", event);
      activeSession = undefined;
      setPhase("error", event.message, { level: 0.24, pitch: 0.74, brightness: 0.78 });
    }
  }

  return {
    async startDictation(startOptions?: LocalFlowStartOptions<TContext>) {
      clearIdleTimer();
      clearLevelTimer();

      if (activeSession) {
        await this.cancelDictation();
      }

      activeSession = {
        sessionId: startOptions?.sessionId ?? createSessionId(),
        context: startOptions?.context,
        audio: startOptions?.audio,
        startedAt: Date.now(),
      };
      setPhase("listening", "Listening", { level: 0.24, pitch: 0.5, brightness: 0.42 });
      startMockMeter(activeSession.sessionId);
    },

    async stopDictation() {
      if (!activeSession || phase !== "listening") {
        return;
      }

      const session = activeSession;
      clearLevelTimer();
      await completeSession(session);
    },

    async cancelDictation() {
      const cancelledSessionId = activeSession?.sessionId ?? "localflow-idle";
      activeSession = undefined;
      clearLevelTimer();
      clearIdleTimer();
      phase = "idle";
      emitVoice({
        sessionId: cancelledSessionId,
        phase: "idle",
        message: "Cancelled",
        level: 0.08,
        pitch: 0.5,
        brightness: 0.35,
      });
    },

    async getStatus() {
      return status();
    },

    async saveSettings(nextSettings: TSettings) {
      settings = nextSettings;
      emitStatus();
      return status();
    },

    on<TEvent extends LocalFlowEventName<TSettings>>(
      eventName: TEvent,
      handler: (payload: LocalFlowEventMap<TSettings>[TEvent]) => void,
    ): LocalFlowUnsubscribe {
      return bus.on(eventName, handler);
    },
  };
}

export function createMockTranscriber<TSettings = unknown, TContext = unknown>(
  options: MockTranscriberOptions = {},
): LocalFlowTranscriber<TSettings, TContext> {
  const text = options.text ?? "This is a LocalFlow mobile SDK dictation.";
  const refinedText = options.refinedText;
  const delayMs = options.delayMs ?? 0;

  return {
    async transcribe(request: LocalFlowTranscriptionRequest<TSettings, TContext>) {
      if (delayMs > 0) {
        await wait(delayMs);
      }

      return {
        text,
        refinedText,
        timings: {
          mockProviderMs: delayMs,
          audioProvided: request.audio ? 1 : 0,
        },
      };
    },
  };
}

function createEventBus<TSettings>() {
  const listeners = new Map<keyof LocalFlowEventMap<TSettings>, Set<UntypedHandler>>();

  return {
    on<TEvent extends LocalFlowEventName<TSettings>>(
      eventName: TEvent,
      handler: (payload: LocalFlowEventMap<TSettings>[TEvent]) => void,
    ): LocalFlowUnsubscribe {
      const handlers = listeners.get(eventName) ?? new Set<UntypedHandler>();
      handlers.add(handler as UntypedHandler);
      listeners.set(eventName, handlers);

      return () => {
        handlers.delete(handler as UntypedHandler);
      };
    },

    emit<TEvent extends LocalFlowEventName<TSettings>>(
      eventName: TEvent,
      payload: LocalFlowEventMap<TSettings>[TEvent],
    ) {
      listeners.get(eventName)?.forEach((handler) => handler(payload));
    },
  };
}

function createSessionId(): string {
  return `localflow-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function clampUnit(value: number): number {
  if (!Number.isFinite(value)) {
    return 0.12;
  }

  return Math.max(0.04, Math.min(1, value));
}

function wait(delayMs: number): Promise<void> {
  return new Promise((resolve) => {
    setTimeout(resolve, delayMs);
  });
}

export type { CreateLocalFlowClientOptions, MockTranscriberOptions, LocalFlowTranscriptionResult };
