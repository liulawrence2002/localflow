import { describe, expect, it } from "vitest";
import {
  describeConversation,
  describeScreenReading,
  localModelsToStatus,
  type ConversationPhase,
  type ConversationStatus,
} from "../src/domain/conversation";
import { hasOllamaModel } from "../src/domain/ollama";

function status(phase: ConversationPhase, overrides: Partial<ConversationStatus> = {}) {
  return {
    phase,
    available: true,
    detail: "Ready",
    hotkey: "Ctrl+Alt+T",
    ...overrides,
  } satisfies ConversationStatus;
}

describe("describeConversation", () => {
  it("offers to start when off and says the microphone is closed", () => {
    const view = describeConversation(status("off"));

    expect(view.buttonLabel).toBe("Start conversation (Ctrl+Alt+T)");
    expect(view.buttonDisabled).toBe(false);
    expect(view.microphoneOpen).toBe(false);
    expect(view.headline).toContain("microphone is closed");
  });

  it("cannot be started when something is not installed", () => {
    const view = describeConversation(status("off", { available: false }));

    expect(view.buttonDisabled).toBe(true);
    expect(view.headline).toContain("Not available");
  });

  it("says the microphone is open in every phase where it is", () => {
    const open: ConversationPhase[] = ["listening", "user_speaking", "thinking", "speaking"];
    for (const phase of open) {
      expect(describeConversation(status(phase)).microphoneOpen, phase).toBe(true);
      expect(describeConversation(status(phase)).headline, phase).toContain("microphone is open");
    }
    expect(describeConversation(status("starting")).microphoneOpen).toBe(false);
  });

  it("labels the button with what the hotkey does in each phase", () => {
    expect(describeConversation(status("listening")).buttonLabel).toContain("End conversation");
    expect(describeConversation(status("user_speaking")).buttonLabel).toContain("End conversation");
    expect(describeConversation(status("thinking")).buttonLabel).toContain("Interrupt");
    expect(describeConversation(status("speaking")).buttonLabel).toContain("Interrupt");
    expect(describeConversation(status("starting")).buttonLabel).toContain("Cancel");
  });

  it("leaves the hotkey out when none could be registered", () => {
    expect(describeConversation(status("off", { hotkey: null })).buttonLabel).toBe(
      "Start conversation",
    );
  });

  it("is disabled outside the desktop app", () => {
    const view = describeConversation(null);

    expect(view.buttonDisabled).toBe(true);
    expect(view.microphoneOpen).toBe(false);
  });
});

describe("describeScreenReading", () => {
  const base = { available: true, detail: "", hotkey: "Ctrl+Alt+R", reading: false };

  it("tells the user the hotkey and the spoken alternative", () => {
    const text = describeScreenReading(base);

    expect(text).toContain("Ctrl+Alt+R");
    expect(text).toContain("read this page");
  });

  it("says how to stop while reading", () => {
    expect(describeScreenReading({ ...base, reading: true })).toContain("to stop");
  });

  it("falls back to the spoken command when no hotkey is registered", () => {
    const text = describeScreenReading({ ...base, hotkey: null });

    expect(text).toContain("read this page");
    expect(text).not.toContain("null");
  });

  it("says the desktop app is needed in a browser", () => {
    expect(describeScreenReading(null)).toContain("desktop app");
  });
});

describe("localModelsToStatus", () => {
  it("lets the model picker find a model the native layer listed", () => {
    const result = localModelsToStatus(["llama3.2:3b", "gemma4:12b-it-qat"], "http://x/api");

    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.models.map((model) => model.name)).toEqual([
        "llama3.2:3b",
        "gemma4:12b-it-qat",
      ]);
      expect(hasOllamaModel(result.models, "LLAMA3.2:3B")).toBe(true);
      expect(hasOllamaModel(result.models, "missing:1b")).toBe(false);
    }
  });
});
