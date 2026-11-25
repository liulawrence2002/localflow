import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { enterDesktopApp, handlers, leaveDesktopApp, resetTauri } from "./support/tauriMock";
import { defaultSettings, defaultStatus } from "../src/domain/defaults";
import * as client from "../src/services/localflowClient";

vi.mock("@tauri-apps/api/core", async () => (await import("./support/tauriMock")).coreModule);

const storageKey = "localflow.settings.v1";

describe("localflow client in the desktop app", () => {
  beforeEach(() => {
    resetTauri();
    enterDesktopApp();
    window.localStorage.clear();
  });

  afterEach(() => {
    leaveDesktopApp();
  });

  it("rejects a failed save with the native reason and never writes browser storage", async () => {
    handlers.save_settings = () => {
      throw "Could not persist settings: database is locked";
    };

    await expect(client.saveSettings(defaultSettings)).rejects.toThrow(
      "Could not persist settings: database is locked",
    );
    expect(window.localStorage.getItem(storageKey)).toBeNull();
  });

  it("throws Error objects, not strings", async () => {
    handlers.copy_last_transcript = () => {
      throw "No transcript is available to copy yet.";
    };

    const failure = await client.copyLastTranscript().catch((error: unknown) => error);

    expect(failure).toBeInstanceOf(Error);
    expect(failure).toBeInstanceOf(client.NativeCommandError);
    expect((failure as Error).message).toBe("No transcript is available to copy yet.");
  });

  it("does not show browser mock data when get_status fails", async () => {
    handlers.get_status = () => {
      throw "LocalFlow runtime state lock was poisoned.";
    };

    await expect(client.getStatus()).rejects.toThrow("lock was poisoned");
  });

  it("refuses a reply of the wrong shape", async () => {
    handlers.conversation_status = () => ({ phase: "dancing" });

    await expect(client.getConversationStatus()).rejects.toThrow("unexpected reply");
  });

  it("clamps numbers into the native ranges before saving", async () => {
    let saved: unknown;
    handlers.save_settings = (args) => {
      saved = args.settings;
      return { ...defaultStatus, settings: args.settings };
    };

    await client.saveSettings({
      ...defaultSettings,
      microphone: { ...defaultSettings.microphone, endOfSpeechMs: Number.NaN },
      models: { ...defaultSettings.models, asrThreads: 900 },
    });

    expect(saved).toMatchObject({
      microphone: { endOfSpeechMs: 900 },
      models: { asrThreads: 255 },
    });
  });

  it("reads the readiness checklist, and shows nothing when the command is missing", async () => {
    expect(await client.getReadiness()).toBeNull();

    handlers.readiness_status = () => [
      {
        id: "speech-model",
        label: "Speech model (Parakeet)",
        ready: false,
        required: true,
        fix: "Run scripts\\Get-AsrModel.ps1",
      },
    ];
    expect(await client.getReadiness()).toEqual([
      {
        id: "speech-model",
        label: "Speech model (Parakeet)",
        ready: false,
        required: true,
        fix: "Run scripts\\Get-AsrModel.ps1",
      },
    ]);
  });

  it("keeps the conversation status usable when lastStop is not reported", async () => {
    handlers.conversation_status = () => ({
      phase: "off",
      available: true,
      detail: "Ready",
      hotkey: null,
    });

    const status = await client.getConversationStatus();

    expect(status?.lastStop).toBeUndefined();
    expect(status?.hotkey).toBeNull();
  });
});

describe("localflow client in a browser", () => {
  beforeEach(() => {
    resetTauri();
    leaveDesktopApp();
    window.localStorage.clear();
  });

  it("falls back to browser storage for settings during frontend development", async () => {
    const saved = await client.saveSettings({
      ...defaultSettings,
      models: { ...defaultSettings.models, ollamaModel: "qwen3:4b" },
    });

    expect(saved.settings.models.ollamaModel).toBe("qwen3:4b");
    expect(window.localStorage.getItem(storageKey)).toContain("qwen3:4b");
  });

  it("returns no native-only status outside the desktop app", async () => {
    expect(await client.getConversationStatus()).toBeNull();
    expect(await client.getReadiness()).toBeNull();
    await expect(client.copyLastTranscript()).rejects.toThrow("desktop app");
  });
});
