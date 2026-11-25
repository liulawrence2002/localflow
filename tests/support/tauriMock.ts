/**
 * A stand-in for the Tauri JS API in tests: commands answer from `handlers`, and `emit`
 * delivers an event to whatever the page is listening for. Use it from a test file with
 *
 *   vi.mock("@tauri-apps/api/core", async () => (await import("./support/tauriMock")).coreModule);
 *   vi.mock("@tauri-apps/api/event", async () => (await import("./support/tauriMock")).eventModule);
 */

import { act } from "@testing-library/react";
import { defaultStatus } from "../../src/domain/defaults";
import type { AppStatus } from "../../src/domain/types";

type Handler = (args: Record<string, unknown>) => unknown;
type Listener = (event: { event: string; id: number; payload: unknown }) => void;

export const handlers: Record<string, Handler> = {};
const listeners = new Map<string, Set<Listener>>();
export const calls: Array<{ command: string; args: Record<string, unknown> }> = [];

export const coreModule = {
  invoke: async (command: string, args: Record<string, unknown> = {}) => {
    calls.push({ command, args });
    const handler = handlers[command];
    if (!handler) {
      // What Tauri rejects with for a command the native build does not have.
      throw `Command ${command} not found`;
    }
    return handler(args);
  },
};

export const eventModule = {
  listen: async (name: string, listener: Listener) => {
    const set = listeners.get(name) ?? new Set<Listener>();
    set.add(listener);
    listeners.set(name, set);
    return () => {
      set.delete(listener);
    };
  },
};

/** Deliver an event to the page, inside `act` so React state updates are applied. */
export async function emit(name: string, payload: unknown): Promise<void> {
  await act(async () => {
    for (const listener of listeners.get(name) ?? []) {
      listener({ event: name, id: 1, payload });
    }
  });
}

/** Behave like the desktop app: `isDesktopApp()` looks for this global. */
export function enterDesktopApp(): void {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

export function leaveDesktopApp(): void {
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}

/** Clear handlers, listeners and recorded calls between tests. */
export function resetTauri(): void {
  for (const key of Object.keys(handlers)) {
    delete handlers[key];
  }
  listeners.clear();
  calls.length = 0;
}

/** A desktop status with real-looking diagnostics. */
export function desktopStatus(overrides: Partial<AppStatus> = {}): AppStatus {
  return {
    ...structuredClone(defaultStatus),
    diagnostics: [
      {
        label: "Registered desktop hotkeys",
        value: "Ctrl+Alt+Space, Ctrl+Alt+Shift+Space",
        status: "ok",
      },
    ],
    ...overrides,
  };
}

/** Answer the commands every screen asks for, as a working desktop app would. */
export function installDesktopDefaults(status: AppStatus = desktopStatus()): void {
  handlers.get_status = () => structuredClone(status);
  handlers.save_settings = (args) => ({ ...structuredClone(status), settings: args.settings });
  handlers.get_last_transcript = () => null;
  handlers.tts_status = () => ({
    available: false,
    detail: "The Fish S2 engine is not built. Run scripts\\Build-FishS2.ps1.",
    speaking: false,
  });
  handlers.conversation_status = () => ({
    phase: "off",
    available: true,
    detail: "Local speech recognition, local language model, Phonon voice",
    hotkey: "Ctrl+Alt+T",
  });
  handlers.screen_reading_status = () => ({
    available: true,
    detail: "Reads the focused window word for word.",
    hotkey: "Ctrl+Alt+R",
    reading: false,
  });
}
