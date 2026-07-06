import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type {
  LocalFlowClient,
  LocalFlowErrorEvent,
  LocalFlowSdkStatus,
  LocalFlowTranscriptEvent,
  LocalFlowVoiceState,
} from "../types";
import { idleVoiceState, LocalFlowContext } from "./LocalFlowContext";
import type { LocalFlowContextValue } from "./LocalFlowContext";

interface LocalFlowProviderProps<TSettings = unknown, TContext = unknown> {
  client: LocalFlowClient<TSettings, TContext>;
  children: ReactNode;
}

export function LocalFlowProvider<TSettings = unknown, TContext = unknown>({
  client,
  children,
}: LocalFlowProviderProps<TSettings, TContext>) {
  const [status, setStatus] = useState<LocalFlowSdkStatus<TSettings>>();
  const [voiceState, setVoiceState] = useState<LocalFlowVoiceState>(idleVoiceState);
  const [lastTranscript, setLastTranscript] = useState<LocalFlowTranscriptEvent>();
  const [error, setError] = useState<LocalFlowErrorEvent>();

  useEffect(() => {
    let mounted = true;

    void client.getStatus().then((nextStatus) => {
      if (mounted) {
        setStatus(nextStatus);
      }
    });

    const unsubscribeVoice = client.on("voice_state", (nextVoiceState) => {
      setVoiceState(nextVoiceState);
      if (nextVoiceState.phase !== "error") {
        setError(undefined);
      }
    });
    const unsubscribeStatus = client.on("status", setStatus);
    const unsubscribeTranscript = client.on("transcript", setLastTranscript);
    const unsubscribeError = client.on("error", setError);

    return () => {
      mounted = false;
      unsubscribeVoice();
      unsubscribeStatus();
      unsubscribeTranscript();
      unsubscribeError();
    };
  }, [client]);

  const value = useMemo<LocalFlowContextValue<TSettings, TContext>>(
    () => ({
      client,
      status,
      voiceState,
      lastTranscript,
      error,
    }),
    [client, error, lastTranscript, status, voiceState],
  );

  return <LocalFlowContext.Provider value={value}>{children}</LocalFlowContext.Provider>;
}

export type { LocalFlowProviderProps };
