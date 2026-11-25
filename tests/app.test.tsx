import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../src/app/App";
import {
  calls,
  desktopStatus,
  emit,
  enterDesktopApp,
  handlers,
  installDesktopDefaults,
  leaveDesktopApp,
  resetTauri,
} from "./support/tauriMock";

vi.mock("@tauri-apps/api/core", async () => (await import("./support/tauriMock")).coreModule);
vi.mock("@tauri-apps/api/event", async () => (await import("./support/tauriMock")).eventModule);

async function renderApp() {
  render(<App />);
  // The first status has arrived once the conversation line says the microphone is closed.
  await screen.findByText("Off. The microphone is closed.");
}

function navigateTo(label: string) {
  fireEvent.click(
    within(screen.getByRole("navigation", { name: "Settings sections" })).getByRole("button", {
      name: label,
    }),
  );
}

describe("settings window in the desktop app", () => {
  beforeEach(() => {
    resetTauri();
    enterDesktopApp();
    installDesktopDefaults();
  });

  afterEach(() => {
    leaveDesktopApp();
  });

  it("shows only sections that do something, and marks the current one", async () => {
    await renderApp();
    const nav = screen.getByRole("navigation", { name: "Settings sections" });

    expect(within(nav).queryByRole("button", { name: "Styles" })).not.toBeInTheDocument();
    expect(within(nav).queryByRole("button", { name: "History" })).not.toBeInTheDocument();
    expect(within(nav).getByRole("button", { name: "Home" })).toHaveAttribute(
      "aria-current",
      "page",
    );

    navigateTo("Dictation");
    expect(within(nav).getByRole("button", { name: "Dictation" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    expect(screen.getByRole("heading", { level: 1, name: "Dictation" })).toHaveFocus();
    // The dead controls are gone; the end-of-speech wait is the one setting left.
    expect(screen.queryByText("Voice activity detection")).not.toBeInTheDocument();
    expect(screen.queryByText("Max seconds")).not.toBeInTheDocument();
    expect(screen.getByLabelText("End-of-speech wait (ms)")).toHaveValue(900);
  });

  it("shows a dismissible banner, with the time and a recovery action, on a dictation error", async () => {
    await renderApp();
    handlers.copy_last_transcript = () => null;

    await emit("localflow://native-dictation", {
      phase: "error",
      message:
        'Focus changed before insertion. The transcript was not pasted; use "Copy last transcript" to recover it.',
      level: null,
      pitch: null,
      brightness: null,
    });

    const banner = await screen.findByRole("alert");
    expect(banner).toHaveTextContent(/Something went wrong at \d\d:\d\d/);
    expect(banner).toHaveTextContent("Focus changed before insertion.");

    fireEvent.click(within(banner).getByRole("button", { name: "Copy last transcript" }));
    await waitFor(() =>
      expect(calls.some((call) => call.command === "copy_last_transcript")).toBe(true),
    );
    expect(
      await screen.findByText("Copied. Paste it where you want with Ctrl+V."),
    ).toBeInTheDocument();

    fireEvent.click(within(banner).getByRole("button", { name: "Dismiss this problem" }));
    expect(screen.queryByText(/Something went wrong/)).not.toBeInTheDocument();
  });

  it("ignores audio-feature events", async () => {
    await renderApp();

    await emit("localflow://native-dictation", {
      phase: "error",
      message: "Listening",
      level: 0.4,
      bands: [1, 2, 3],
    });

    expect(screen.queryByText(/Something went wrong/)).not.toBeInTheDocument();
  });

  it("says how the last conversation ended when it failed to start", async () => {
    handlers.conversation_status = () => ({
      phase: "off",
      available: true,
      detail: "Ready",
      hotkey: "Ctrl+Alt+T",
      lastStop: {
        reason: "start_failed",
        message: "No default microphone is available.",
        atMs: new Date(2026, 10, 25, 14, 2).getTime(),
      },
    });

    await renderApp();

    const notice = await screen.findByText(/The last conversation ended with a problem at 14:02/);
    expect(notice.closest('[role="alert"]')).toHaveTextContent(
      "No default microphone is available.",
    );
  });

  it("falls back to the conversation event when the status has no lastStop", async () => {
    await renderApp();

    await emit("localflow://conversation", {
      phase: "off",
      reason: "device_lost",
      message: "Conversation ended: the microphone stopped",
    });

    expect(
      await screen.findByText("Conversation ended: the microphone stopped"),
    ).toBeInTheDocument();
  });

  it("shows the setup checklist while something required is missing", async () => {
    handlers.readiness_status = () => [
      {
        id: "speech-model",
        label: "Speech model (Parakeet)",
        ready: false,
        required: true,
        fix: "Run scripts\\Get-AsrModel.ps1",
      },
      { id: "ollama", label: "Ollama", ready: true, required: true, fix: null },
    ];

    await renderApp();

    const setup = await screen.findByRole("region", { name: "Setup" });
    expect(within(setup).getByText("Speech model (Parakeet)")).toBeInTheDocument();
    expect(within(setup).getByText("Run scripts\\Get-AsrModel.ps1")).toBeInTheDocument();
    expect(within(setup).getByText("Missing")).toBeInTheDocument();
  });

  it("hides the setup checklist when everything required is ready, or the command is missing", async () => {
    await renderApp();
    await waitFor(() =>
      expect(calls.some((call) => call.command === "readiness_status")).toBe(true),
    );
    expect(screen.queryByRole("region", { name: "Setup" })).not.toBeInTheDocument();
  });

  it("reports a failed save instead of showing it as saved", async () => {
    handlers.save_settings = () => {
      throw "Could not persist settings: database is locked";
    };
    await renderApp();
    navigateTo("Dictation");
    const field = screen.getByLabelText("End-of-speech wait (ms)");

    fireEvent.change(field, { target: { value: "1500" } });
    fireEvent.blur(field);

    expect(
      await screen.findByText("Your last change was not saved", {}, { timeout: 2000 }),
    ).toBeInTheDocument();
    expect(screen.getByText("Could not persist settings: database is locked")).toBeInTheDocument();
    expect(screen.getByText("Not saved")).toBeInTheDocument();
    expect(window.localStorage.getItem("localflow.settings.v1")).toBeNull();
  });

  it("saves a validated end-of-speech wait", async () => {
    await renderApp();
    navigateTo("Dictation");
    const field = screen.getByLabelText("End-of-speech wait (ms)");

    fireEvent.change(field, { target: { value: "99999" } });
    fireEvent.blur(field);

    await waitFor(
      () => {
        const save = calls.find((call) => call.command === "save_settings");
        expect(save?.args.settings).toMatchObject({ microphone: { endOfSpeechMs: 3000 } });
      },
      { timeout: 2000 },
    );
    expect(await screen.findByText("All changes saved")).toBeInTheDocument();
  });

  it("does not let settings be edited when they could not be loaded", async () => {
    handlers.get_status = () => {
      throw "LocalFlow runtime state lock was poisoned.";
    };
    render(<App />);
    navigateTo("Models");

    expect(await screen.findByText("LocalFlow's settings could not be loaded")).toBeInTheDocument();
    expect(screen.queryByLabelText("Speech model file")).not.toBeInTheDocument();
  });

  it("follows read-aloud events and enables Stop only while reading runs", async () => {
    handlers.get_last_transcript = () => "hello there";
    handlers.tts_status = () => ({ available: true, detail: "Fish S2 Pro", speaking: false });
    await renderApp();
    const stop = await screen.findByRole("button", { name: "Stop reading" });
    expect(stop).toBeDisabled();

    await emit("localflow://tts", { phase: "synthesizing", message: "Preparing speech" });
    expect(screen.getByText(/Preparing speech with the Fish S2 voice/)).toBeInTheDocument();
    expect(stop).toBeEnabled();

    await emit("localflow://tts", { phase: "speaking", message: "Reading aloud" });
    expect(screen.getByText("Reading the last transcript aloud.")).toBeInTheDocument();

    await emit("localflow://tts", {
      phase: "error",
      message: "Playback failed: no output device.",
    });
    expect(screen.getByText("Playback failed: no output device.")).toBeInTheDocument();
    expect(screen.queryByText(/Preparing speech/)).not.toBeInTheDocument();
    expect(stop).toBeDisabled();
  });

  it("names the Fish S2 voice and what to install when read-aloud is unavailable", async () => {
    handlers.get_last_transcript = () => "hello there";
    await renderApp();

    expect(await screen.findByRole("button", { name: "Read aloud with Fish S2" })).toBeDisabled();
    expect(screen.getByText(/optional Fish S2 Pro voice/)).toHaveTextContent("Build-FishS2.ps1");
  });

  it("refreshes before exporting diagnostics and removes the user folder and microphone", async () => {
    await renderApp();
    handlers.get_status = () =>
      desktopStatus({
        diagnostics: [
          {
            label: "Last recording start",
            value: "Ada's Headset at 2026-11-25T10:00:00Z (48000 Hz, 1 ch)",
            status: "ok",
          },
          {
            label: "Speech model",
            value: "C:\\Users\\Ada\\localflow\\.localflow-runtime\\models\\x.gguf",
            status: "ok",
          },
        ],
      });
    navigateTo("Diagnostics");

    fireEvent.click(screen.getByRole("button", { name: "Prepare export" }));

    const exportBox = screen.getByLabelText("Diagnostics export") as HTMLTextAreaElement;
    await waitFor(() => expect(exportBox.value).toContain("<user folder>"));
    const exported = exportBox.value;
    expect(exported).toContain("<microphone> at 2026-11-25T10:00:00Z");
    expect(exported).not.toContain("Ada");
    // The table on screen shows the refreshed rows with worded statuses.
    expect(screen.getAllByText("OK").length).toBeGreaterThan(0);
  });

  it("shows the real hotkeys and whether each is registered", async () => {
    handlers.conversation_status = () => ({
      phase: "off",
      available: true,
      detail: "Ready",
      hotkey: "Ctrl+Alt+Shift+T",
    });
    await renderApp();
    navigateTo("Hotkeys");

    const table = screen.getByRole("table");
    expect(within(table).getByText("Ctrl+Alt+Space")).toBeInTheDocument();
    expect(within(table).getByText("Escape")).toBeInTheDocument();
    expect(
      within(table).getByText("Another program holds it; Ctrl+Alt+Shift+T works instead"),
    ).toBeInTheDocument();
    expect(screen.queryByText("Push")).not.toBeInTheDocument();
  });

  it("states the privacy facts instead of switches that do nothing", async () => {
    await renderApp();
    navigateTo("Privacy");

    expect(screen.getByText("Dictations are not stored.")).toBeInTheDocument();
    expect(screen.getByText("Audio is deleted right after transcription.")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Nothing leaves this computer, except a coding-agent question you confirm by voice.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  });

  it("explains the next step when no coding projects exist", async () => {
    handlers.list_coding_projects = () => [];
    handlers.detect_coding_agents = () => ({ agents: [], windowsTerminal: null });
    await renderApp();
    navigateTo("Coding agents");

    expect(
      await screen.findByText(/Voice commands work only in a conversation/),
    ).toBeInTheDocument();
  });

  it("reads the version from the build", async () => {
    await renderApp();
    navigateTo("About");

    expect(screen.getByText(__APP_VERSION__)).toBeInTheDocument();
    expect(screen.queryByText("Default style")).not.toBeInTheDocument();
  });
});
