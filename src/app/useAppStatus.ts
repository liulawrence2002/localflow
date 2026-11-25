import { useCallback, useEffect, useRef, useState } from "react";
import type { AppStatus, LocalFlowSettings } from "../domain/types";
import { errorMessage, getStatus, saveSettings } from "../services/localflowClient";

/** How long typing pauses before settings are saved. */
export const saveDebounceMs = 300;

export type SaveState =
  | { kind: "idle" }
  | { kind: "pending" }
  | { kind: "saving" }
  | { kind: "saved"; atMs: number }
  | { kind: "error"; message: string };

export type LoadState =
  { kind: "loading" } | { kind: "ready" } | { kind: "failed"; message: string };

export interface AppStatusApi {
  /** The last status from the native layer; `null` until it has loaded. */
  status: AppStatus | null;
  /** The settings as edited here: ahead of the native copy while a save is pending. */
  settings: LocalFlowSettings | null;
  loadState: LoadState;
  saveState: SaveState;
  /** Change settings now and save them shortly after; nothing happens before they loaded. */
  updateSettings: (updater: (settings: LocalFlowSettings) => LocalFlowSettings) => void;
  /** Save the current settings again after a failure. */
  retrySave: () => void;
  /** Ask the native layer for its status again (diagnostics, history). */
  refreshStatus: () => Promise<AppStatus | null>;
  /** Replace the status (workflow, history, diagnostics) with one a command returned. */
  applyStatus: (status: AppStatus) => void;
}

/**
 * The app status and settings. Edits apply here at once (optimistic) and are saved after a
 * short pause, outside any state updater. Each save has a sequence number, so a slow reply to
 * an older save can never overwrite newer edits, and a failed save is reported, never shown as
 * saved.
 */
export function useAppStatus(): AppStatusApi {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [settings, setSettings] = useState<LocalFlowSettings | null>(null);
  const [loadState, setLoadState] = useState<LoadState>({ kind: "loading" });
  const [saveState, setSaveState] = useState<SaveState>({ kind: "idle" });

  const settingsRef = useRef<LocalFlowSettings | null>(null);
  /** Counts local edits; a reply may only replace the settings if none happened since. */
  const editVersionRef = useRef(0);
  /** Counts saves sent; only the newest save's reply is used. */
  const saveSequenceRef = useRef(0);
  const timerRef = useRef<number | null>(null);
  const mountedRef = useRef(true);

  const adoptSettings = useCallback((next: LocalFlowSettings) => {
    settingsRef.current = next;
    setSettings(next);
  }, []);

  const flush = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const pending = settingsRef.current;
    if (!pending) {
      return;
    }
    const sequence = ++saveSequenceRef.current;
    const editVersion = editVersionRef.current;
    setSaveState({ kind: "saving" });
    saveSettings(pending).then(
      (next) => {
        if (!mountedRef.current || sequence !== saveSequenceRef.current) {
          return;
        }
        setStatus((current) => ({ ...(current ?? next), ...next }));
        // An edit made while this save was in flight is not in it, and its own save may still
        // be waiting for the pause: then nothing is "saved" yet and the state stays pending.
        const nothingNewer = editVersion === editVersionRef.current && timerRef.current === null;
        if (!nothingNewer) {
          return;
        }
        adoptSettings(next.settings);
        setSaveState({ kind: "saved", atMs: Date.now() });
      },
      (reason: unknown) => {
        if (!mountedRef.current || sequence !== saveSequenceRef.current) {
          return;
        }
        setSaveState({
          kind: "error",
          message: errorMessage(reason, "LocalFlow could not save the settings."),
        });
      },
    );
  }, [adoptSettings]);

  const updateSettings = useCallback(
    (updater: (current: LocalFlowSettings) => LocalFlowSettings) => {
      const current = settingsRef.current;
      if (!current) {
        return;
      }
      const next = updater(current);
      if (next === current) {
        return;
      }
      editVersionRef.current += 1;
      adoptSettings(next);
      setSaveState({ kind: "pending" });
      if (timerRef.current !== null) {
        window.clearTimeout(timerRef.current);
      }
      timerRef.current = window.setTimeout(flush, saveDebounceMs);
    },
    [adoptSettings, flush],
  );

  const refreshStatus = useCallback(async (): Promise<AppStatus | null> => {
    try {
      const next = await getStatus();
      if (!mountedRef.current) {
        return next;
      }
      setStatus(next);
      // Settings from the native layer replace the local copy only while nothing is waiting
      // to be saved; otherwise the pending edits would be lost.
      if (timerRef.current === null && settingsRef.current === null) {
        adoptSettings(next.settings);
      }
      setLoadState({ kind: "ready" });
      return next;
    } catch (reason) {
      if (mountedRef.current && settingsRef.current === null) {
        setLoadState({
          kind: "failed",
          message: errorMessage(reason, "LocalFlow's settings could not be loaded."),
        });
      }
      return null;
    }
  }, [adoptSettings]);

  useEffect(() => {
    mountedRef.current = true;
    void refreshStatus();
    // A pending save is sent at once when the window is hidden or closed.
    const onHidden = () => {
      if (document.visibilityState === "hidden" && timerRef.current !== null) {
        flush();
      }
    };
    const onPageHide = () => {
      if (timerRef.current !== null) {
        flush();
      }
    };
    document.addEventListener("visibilitychange", onHidden);
    window.addEventListener("pagehide", onPageHide);
    return () => {
      document.removeEventListener("visibilitychange", onHidden);
      window.removeEventListener("pagehide", onPageHide);
      if (timerRef.current !== null) {
        flush();
      }
      mountedRef.current = false;
    };
  }, [flush, refreshStatus]);

  // Settings are not taken from it: they come from loading and from save replies only, so an
  // unrelated command can never undo an edit.
  const applyStatus = useCallback((next: AppStatus) => setStatus(next), []);

  return {
    status,
    settings,
    loadState,
    saveState,
    updateSettings,
    retrySave: flush,
    refreshStatus,
    applyStatus,
  };
}
