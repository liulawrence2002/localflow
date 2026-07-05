import { createContext, useContext } from "react";
import type {
  LocalFlowClient,
  LocalFlowErrorEvent,
  LocalFlowSdkStatus,
  LocalFlowTranscriptEvent,
  LocalFlowVoiceState,
} from "../types";

export interface LocalFlowContextValue<TSettings = unknown, TContext = unknown> {
  client: LocalFlowClient<TSettings, TContext>;
  status?: LocalFlowSdkStatus<TSettings>;
  voiceState: LocalFlowVoiceState;
  lastTranscript?: LocalFlowTranscriptEvent;
  error?: LocalFlowErrorEvent;
}

export const idleVoiceState: LocalFlowVoiceState = {
  sessionId: "localflow-idle",
  phase: "idle",
  message: "Idle",
  level: 0.08,
  pitch: 0.5,
  brightness: 0.35,
};

export const LocalFlowContext = createContext<LocalFlowContextValue | undefined>(undefined);

export function useLocalFlow<TSettings = unknown, TContext = unknown>() {
  const value = useContext(LocalFlowContext);

  if (!value) {
    throw new Error("useLocalFlow must be used inside LocalFlowProvider.");
  }

  return value as LocalFlowContextValue<TSettings, TContext>;
}
