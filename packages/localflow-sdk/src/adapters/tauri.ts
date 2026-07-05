import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type {
  LocalFlowClient,
  LocalFlowDictationPhase,
  LocalFlowEventMap,
  LocalFlowEventName,
  LocalFlowSdkStatus,
  LocalFlowStartOptions,
  LocalFlowUnsubscribe,
  LocalFlowVoiceState,
} from "../types";

interface TauriLocalFlowClientOptions {
  eventName?: string;
  commands?: Partial<{
    startDictation: string;
    stopDictation: string;
    cancelDictation: string;
    getStatus: string;
    saveSettings: string;
  }>;
}

interface TauriStatusLike<TSettings> {
  workflow?: {
    phase?: string;
    activeSession?: {
      id?: string;
    };
  };
  settings?: TSettings;
  diagnostics?: LocalFlowSdkStatus<TSettings>["diagnostics"];
}

type UntypedHandler = (payload: unknown) => void;

const defaultCommands = {
  startDictation: "mobile_start_dictation",
  stopDictation: "mobile_stop_dictation",
  cancelDictation: "cancel_session",
  getStatus: "get_status",
  saveSettings: "save_settings",
};

export function createTauriLocalFlowClient<TSettings = unknown, TContext = unknown>(
  options: TauriLocalFlowClientOptions = {},
): LocalFlowClient<TSettings, TContext> {
  const eventName = options.eventName ?? "localflow://native-dictation";
  const commands = { ...defaultCommands, ...options.commands };
  const listeners = new Map<keyof LocalFlowEventMap<TSettings>, Set<UntypedHandler>>();
  let nativeUnlisten: LocalFlowUnsubscribe | undefined;

  async function ensureNativeListener() {
    if (nativeUnlisten) {
      return;
    }

    nativeUnlisten = await listen<Partial<LocalFlowVoiceState>>(eventName, (event) => {
      emit("voice_state", normalizeVoiceState(event.payload));
    });
  }

  function on<TEvent extends LocalFlowEventName<TSettings>>(
    event: TEvent,
    handler: (payload: LocalFlowEventMap<TSettings>[TEvent]) => void,
  ): LocalFlowUnsubscribe {
    const handlers = listeners.get(event) ?? new Set<UntypedHandler>();
    handlers.add(handler as UntypedHandler);
    listeners.set(event, handlers);

    if (event === "voice_state") {
      void ensureNativeListener();
    }

    return () => {
      handlers.delete(handler as UntypedHandler);
    };
  }

  function emit<TEvent extends LocalFlowEventName<TSettings>>(
    event: TEvent,
    payload: LocalFlowEventMap<TSettings>[TEvent],
  ) {
    listeners.get(event)?.forEach((handler) => handler(payload));
  }

  return {
    async startDictation(startOptions?: LocalFlowStartOptions<TContext>) {
      await invoke(commands.startDictation, { options: startOptions ?? {} });
    },

    async stopDictation() {
      await invoke(commands.stopDictation);
    },

    async cancelDictation() {
      await invoke(commands.cancelDictation);
    },

    async getStatus() {
      const status = await invoke<TauriStatusLike<TSettings>>(commands.getStatus);
      const sdkStatus = mapTauriStatus(status);
      emit("status", sdkStatus);
      return sdkStatus;
    },

    async saveSettings(settings: TSettings) {
      const status = await invoke<TauriStatusLike<TSettings>>(commands.saveSettings, { settings });
      const sdkStatus = mapTauriStatus(status);
      emit("status", sdkStatus);
      return sdkStatus;
    },

    on,
  };
}

function normalizeVoiceState(payload: Partial<LocalFlowVoiceState>): LocalFlowVoiceState {
  return {
    sessionId: payload.sessionId ?? "tauri-native",
    phase: payload.phase ?? "idle",
    message: payload.message ?? formatPhase(payload.phase ?? "idle"),
    level: payload.level,
    pitch: payload.pitch,
    brightness: payload.brightness,
  };
}

function mapTauriStatus<TSettings>(
  status: TauriStatusLike<TSettings>,
): LocalFlowSdkStatus<TSettings> {
  return {
    phase: mapWorkflowPhase(status.workflow?.phase),
    activeSessionId: status.workflow?.activeSession?.id,
    settings: status.settings,
    diagnostics: status.diagnostics ?? [],
  };
}

function mapWorkflowPhase(phase: string | undefined): LocalFlowDictationPhase {
  switch (phase) {
    case "Listening":
      return "listening";
    case "Transcribing":
    case "Inserting":
      return "processing";
    case "Refining":
      return "refining";
    case "Complete":
      return "inserted";
    case "Error":
      return "error";
    default:
      return "idle";
  }
}

function formatPhase(phase: LocalFlowDictationPhase): string {
  return phase.charAt(0).toUpperCase() + phase.slice(1);
}

export type { TauriLocalFlowClientOptions };
