import { useCallback, useEffect, useState } from "react";
import type {
  ConversationStatus,
  ConversationStop,
  ScreenReadingStatus,
} from "../domain/conversation";
import { isProblemStop } from "../domain/conversation";
import {
  describeNativeProblem,
  type NativeProblem,
  type TtsEventPayload,
} from "../domain/nativeEvents";
import type { TtsStatus } from "../domain/nativePayloads";
import type { ReadinessItem } from "../domain/readiness";
import {
  errorMessage,
  getConversationStatus,
  getLastTranscript,
  getReadiness,
  getScreenReadingStatus,
  getTtsStatus,
} from "../services/localflowClient";
import { useNativeEvents } from "./useNativeEvents";
import { useOnWindowReturn, usePolling } from "./usePolling";

/** How often conversation and reading status is asked for while the window is visible. */
export const livePollMs = 1000;

export interface LiveStatus {
  /** `undefined` until first asked; `null` outside the desktop app. */
  conversation: ConversationStatus | null | undefined;
  screenReading: ScreenReadingStatus | null | undefined;
  tts: TtsStatus | null;
  lastTranscript: string | null;
  readiness: ReadinessItem[] | null;
  /** Set while the native layer does not answer status requests. */
  liveError: string | null;
  /** The last problem the native layer reported, until dismissed. */
  problem: NativeProblem | null;
  dismissProblem: () => void;
  /** The last read-aloud event, for the Recovery panel. */
  readAloud: TtsEventPayload | null;
  setReadAloud: (event: TtsEventPayload | null) => void;
  /** How the last conversation ended, as its event said; used when the status cannot. */
  conversationStopFromEvent: ConversationStop | null;
  refreshConversation: () => Promise<void>;
  refreshLastTranscript: () => Promise<void>;
  refreshOnReturn: () => void;
}

/**
 * Everything on Home that changes by itself: conversation and reading state (polled about once
 * a second while the window is visible, paused while hidden), the native events, read-aloud,
 * the last transcript and the setup checklist.
 */
export function useLiveStatus(onWindowReturn?: () => void): LiveStatus {
  const [conversation, setConversation] = useState<ConversationStatus | null | undefined>();
  const [screenReading, setScreenReading] = useState<ScreenReadingStatus | null | undefined>();
  const [tts, setTts] = useState<TtsStatus | null>(null);
  const [lastTranscript, setLastTranscript] = useState<string | null>(null);
  const [readiness, setReadiness] = useState<ReadinessItem[] | null>(null);
  const [liveError, setLiveError] = useState<string | null>(null);
  const [problem, setProblem] = useState<NativeProblem | null>(null);
  const [readAloud, setReadAloud] = useState<TtsEventPayload | null>(null);
  const [conversationStopFromEvent, setConversationStopFromEvent] =
    useState<ConversationStop | null>(null);

  const refreshConversation = useCallback(async () => {
    try {
      const [nextConversation, nextReading] = await Promise.all([
        getConversationStatus(),
        getScreenReadingStatus(),
      ]);
      setConversation(nextConversation);
      setScreenReading(nextReading);
      setLiveError(null);
    } catch (reason) {
      setLiveError(errorMessage(reason, "LocalFlow's desktop app is not answering."));
    }
  }, []);

  const refreshLastTranscript = useCallback(async () => {
    try {
      setLastTranscript(await getLastTranscript());
    } catch (reason) {
      setLiveError(errorMessage(reason, "The last transcript could not be read."));
    }
  }, []);

  const refreshSlow = useCallback(() => {
    void refreshLastTranscript();
    void getTtsStatus().then(setTts, () => setTts(null));
    void getReadiness().then(setReadiness);
  }, [refreshLastTranscript]);

  usePolling(() => void refreshConversation(), livePollMs);

  const refreshOnReturn = useCallback(() => {
    refreshSlow();
    onWindowReturn?.();
  }, [onWindowReturn, refreshSlow]);

  // Once on mount, then whenever the window comes back from the tray.
  useEffect(() => {
    refreshSlow();
  }, [refreshSlow]);
  useOnWindowReturn(refreshOnReturn);

  useNativeEvents({
    onDictation: (event) => {
      if (event.phase === "error") {
        setProblem(describeNativeProblem(event.message, Date.now()));
        // A skipped paste keeps the transcript for recovery.
        void refreshLastTranscript();
      } else if (event.phase === "inserted") {
        // A dictation that worked: the earlier problem, and its recovery action, are stale.
        setProblem(null);
        void refreshLastTranscript();
      }
    },
    onConversation: (event) => {
      void refreshConversation();
      if (event.phase === "off" && event.reason) {
        setConversationStopFromEvent({
          reason: event.reason,
          message: event.message,
          atMs: Date.now(),
        });
        // The Conversation panel shows how it ended; the banner need not repeat it.
        if (isProblemStop(event.reason)) {
          setProblem((current) =>
            current && current.message === event.message.trim() ? null : current,
          );
        }
      }
    },
    onTts: (event) => {
      setReadAloud(event);
      setTts((current) =>
        current ? { ...current, speaking: event.phase === "speaking" } : current,
      );
    },
  });

  return {
    conversation,
    screenReading,
    tts,
    lastTranscript,
    readiness,
    liveError,
    problem,
    dismissProblem: () => setProblem(null),
    readAloud,
    setReadAloud,
    conversationStopFromEvent,
    refreshConversation,
    refreshLastTranscript,
    refreshOnReturn,
  };
}
