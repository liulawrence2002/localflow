import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { saveDebounceMs, useAppStatus } from "../src/app/useAppStatus";
import type { AppStatus, LocalFlowSettings } from "../src/domain/types";
import {
  calls,
  desktopStatus,
  enterDesktopApp,
  handlers,
  installDesktopDefaults,
  leaveDesktopApp,
  resetTauri,
} from "./support/tauriMock";

vi.mock("@tauri-apps/api/core", async () => (await import("./support/tauriMock")).coreModule);

function withModel(settings: LocalFlowSettings, ollamaModel: string): LocalFlowSettings {
  return { ...settings, models: { ...settings.models, ollamaModel } };
}

function saves() {
  return calls.filter((call) => call.command === "save_settings");
}

describe("useAppStatus", () => {
  beforeEach(() => {
    resetTauri();
    enterDesktopApp();
    installDesktopDefaults();
  });

  afterEach(() => {
    vi.useRealTimers();
    leaveDesktopApp();
  });

  it("applies edits at once and saves once after typing pauses", async () => {
    const { result } = renderHook(() => useAppStatus());
    await waitFor(() => expect(result.current.settings).not.toBeNull());
    vi.useFakeTimers();

    act(() => {
      result.current.updateSettings((current) => withModel(current, "a"));
      result.current.updateSettings((current) => withModel(current, "ab"));
      result.current.updateSettings((current) => withModel(current, "abc"));
    });

    expect(result.current.settings?.models.ollamaModel).toBe("abc");
    expect(result.current.saveState.kind).toBe("pending");
    expect(saves()).toHaveLength(0);

    await act(async () => {
      vi.advanceTimersByTime(saveDebounceMs);
    });
    expect(saves()).toHaveLength(1);
    expect(saves()[0]?.args.settings).toMatchObject({ models: { ollamaModel: "abc" } });
  });

  it("drops a slow reply to an older save so it cannot undo newer edits", async () => {
    const replies: Array<(status: AppStatus) => void> = [];
    handlers.save_settings = (args) =>
      new Promise<AppStatus>((resolve) => {
        replies.push(() =>
          resolve({ ...desktopStatus(), settings: args.settings as LocalFlowSettings }),
        );
      });
    const { result } = renderHook(() => useAppStatus());
    await waitFor(() => expect(result.current.settings).not.toBeNull());
    vi.useFakeTimers();

    act(() => result.current.updateSettings((current) => withModel(current, "first")));
    await act(async () => {
      vi.advanceTimersByTime(saveDebounceMs);
    });
    act(() => result.current.updateSettings((current) => withModel(current, "second")));
    await act(async () => {
      vi.advanceTimersByTime(saveDebounceMs);
    });
    expect(replies).toHaveLength(2);

    // The newer save answers first, then the older one arrives late.
    await act(async () => {
      replies[1]?.(desktopStatus());
    });
    await act(async () => {
      replies[0]?.(desktopStatus());
    });

    expect(result.current.settings?.models.ollamaModel).toBe("second");
    expect(result.current.saveState.kind).toBe("saved");
  });

  it("does not show Saved when an older save answers while a newer edit waits to be saved", async () => {
    const replies: Array<() => void> = [];
    handlers.save_settings = (args) =>
      new Promise<AppStatus>((resolve) => {
        replies.push(() =>
          resolve({ ...desktopStatus(), settings: args.settings as LocalFlowSettings }),
        );
      });
    const { result } = renderHook(() => useAppStatus());
    await waitFor(() => expect(result.current.settings).not.toBeNull());
    vi.useFakeTimers();

    // Edit A is sent; edit B is made before A's reply, so B's save is still waiting.
    act(() => result.current.updateSettings((current) => withModel(current, "A")));
    await act(async () => {
      vi.advanceTimersByTime(saveDebounceMs);
    });
    expect(replies).toHaveLength(1);
    act(() => result.current.updateSettings((current) => withModel(current, "B")));

    await act(async () => {
      replies[0]?.();
    });
    expect(result.current.saveState.kind).toBe("pending");
    expect(result.current.settings?.models.ollamaModel).toBe("B");

    // B's save goes out and succeeds: now everything is saved.
    await act(async () => {
      vi.advanceTimersByTime(saveDebounceMs);
    });
    expect(replies).toHaveLength(2);
    expect(saves()[1]?.args.settings).toMatchObject({ models: { ollamaModel: "B" } });
    await act(async () => {
      replies[1]?.();
    });
    expect(result.current.saveState.kind).toBe("saved");
    expect(result.current.settings?.models.ollamaModel).toBe("B");
  });

  it("reports a failed save and can try again", async () => {
    let fail = true;
    handlers.save_settings = (args) => {
      if (fail) {
        throw "Could not persist settings: disk full";
      }
      return { ...desktopStatus(), settings: args.settings };
    };
    const { result } = renderHook(() => useAppStatus());
    await waitFor(() => expect(result.current.settings).not.toBeNull());

    act(() => result.current.updateSettings((current) => withModel(current, "qwen3:4b")));
    await waitFor(() => expect(result.current.saveState.kind).toBe("error"), { timeout: 2000 });
    expect(result.current.saveState).toMatchObject({
      message: "Could not persist settings: disk full",
    });
    // The edit is kept, so trying again saves it.
    expect(result.current.settings?.models.ollamaModel).toBe("qwen3:4b");

    fail = false;
    act(() => result.current.retrySave());
    await waitFor(() => expect(result.current.saveState.kind).toBe("saved"));
  });

  it("changes nothing before the settings have loaded", async () => {
    handlers.get_status = () => {
      throw "not ready";
    };
    const { result } = renderHook(() => useAppStatus());
    await waitFor(() => expect(result.current.loadState.kind).toBe("failed"));

    act(() => result.current.updateSettings((current) => withModel(current, "x")));

    expect(result.current.settings).toBeNull();
    expect(result.current.saveState.kind).toBe("idle");
  });
});
