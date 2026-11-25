import { listen } from "@tauri-apps/api/event";
import { useEffect, useRef } from "react";
import {
  conversationEventName,
  dictationEventName,
  parseConversationEvent,
  parseDictationEvent,
  parseTtsEvent,
  ttsEventName,
  type ConversationEventPayload,
  type DictationPhaseEvent,
  type TtsEventPayload,
} from "../domain/nativeEvents";
import { isDesktopApp } from "../services/localflowClient";

export interface NativeEventHandlers {
  onDictation?: (event: DictationPhaseEvent) => void;
  onConversation?: (event: ConversationEventPayload) => void;
  onTts?: (event: TtsEventPayload) => void;
}

/**
 * Listen to the native layer's events in the settings window. Audio-feature events (about 36
 * a second) are dropped before any React state is touched. The handlers may change between
 * renders; the listeners are registered once.
 */
export function useNativeEvents(handlers: NativeEventHandlers): void {
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  useEffect(() => {
    if (!isDesktopApp()) {
      return undefined;
    }
    let active = true;
    const unlisteners: Array<() => void> = [];
    const keep = (promise: Promise<() => void>) => {
      promise.then(
        (unlisten) => {
          if (active) {
            unlisteners.push(unlisten);
          } else {
            unlisten();
          }
        },
        () => {
          // Listening is not permitted or not available; the window still polls for status.
        },
      );
    };

    keep(
      listen<unknown>(dictationEventName, (event) => {
        const parsed = parseDictationEvent(event.payload);
        if (active && parsed) {
          handlersRef.current.onDictation?.(parsed);
        }
      }),
    );
    keep(
      listen<unknown>(conversationEventName, (event) => {
        const parsed = parseConversationEvent(event.payload);
        if (active && parsed) {
          handlersRef.current.onConversation?.(parsed);
        }
      }),
    );
    keep(
      listen<unknown>(ttsEventName, (event) => {
        const parsed = parseTtsEvent(event.payload);
        if (active && parsed) {
          handlersRef.current.onTts?.(parsed);
        }
      }),
    );

    return () => {
      active = false;
      for (const unlisten of unlisteners.splice(0)) {
        unlisten();
      }
    };
  }, []);
}
