import {
  Activity,
  BookOpenText,
  Braces,
  ClipboardList,
  FileClock,
  Gauge,
  History,
  Info,
  Keyboard,
  Mic,
  Play,
  RotateCcw,
  Save,
  Shield,
  SlidersHorizontal,
  Square,
} from "lucide-react";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import "./App.css";
import { OverlayPreview } from "./components/OverlayPreview";
import { defaultStatus } from "./domain/defaults";
import type { AppStatus, CleanupLevel, LocalFlowSettings } from "./domain/types";
import {
  beginMockSession,
  cancelSession,
  finishMockSession,
  getStatus,
} from "./services/localflowClient";

type ScreenId =
  | "home"
  | "models"
  | "microphone"
  | "hotkeys"
  | "dictionary"
  | "replacements"
  | "snippets"
  | "styles"
  | "privacy"
  | "history"
  | "diagnostics"
  | "about";

const screens = [
  { id: "home", label: "Home", icon: Activity },
  { id: "models", label: "Models", icon: Braces },
  { id: "microphone", label: "Microphone", icon: Mic },
  { id: "hotkeys", label: "Hotkeys", icon: Keyboard },
  { id: "dictionary", label: "Dictionary", icon: BookOpenText },
  { id: "replacements", label: "Replacements", icon: RotateCcw },
  { id: "snippets", label: "Snippets", icon: ClipboardList },
  { id: "styles", label: "Styles", icon: SlidersHorizontal },
  { id: "privacy", label: "Privacy", icon: Shield },
  { id: "history", label: "History", icon: History },
  { id: "diagnostics", label: "Diagnostics", icon: Gauge },
  { id: "about", label: "About", icon: Info },
] satisfies Array<{ id: ScreenId; label: string; icon: typeof Activity }>;

const cleanupLevels: CleanupLevel[] = ["verbatim", "light", "balanced", "strong"];

export function App() {
  const [activeScreen, setActiveScreen] = useState<ScreenId>("home");
  const [status, setStatus] = useState<AppStatus>(defaultStatus);
  const [mockTranscript, setMockTranscript] = useState(
    "meet me Tuesday no Wednesday comma then review the pie torch model",
  );
  const [isBusy, setIsBusy] = useState(false);

  useEffect(() => {
    void getStatus().then(setStatus);
  }, []);

  const settings = status.settings;
  const activeStyle = settings.styles[0];
  const statusText = status.workflow.error ?? status.workflow.warning ?? status.workflow.phase;

  async function startMockDictation() {
    setIsBusy(true);
    const workflow = await beginMockSession();
    setStatus((current) => ({ ...current, workflow }));
    setIsBusy(false);
  }

  async function completeMockDictation() {
    setIsBusy(true);
    const nextStatus = await finishMockSession(mockTranscript);
    setStatus(nextStatus);
    setIsBusy(false);
  }

  async function stopSession() {
    setIsBusy(true);
    const workflow = await cancelSession();
    setStatus((current) => ({ ...current, workflow }));
    setIsBusy(false);
  }

  function updateSettings(updater: (settings: LocalFlowSettings) => LocalFlowSettings) {
    setStatus((current) => ({ ...current, settings: updater(current.settings) }));
  }

  return (
    <main className="app-shell">
      <aside className="sidebar" aria-label="LocalFlow sections">
        <div className="brand">
          <div className="brand-mark" aria-hidden="true">
            LF
          </div>
          <div>
            <strong>LocalFlow</strong>
            <span>Local dictation</span>
          </div>
        </div>
        <nav className="nav-list">
          {screens.map((screen) => {
            const Icon = screen.icon;
            return (
              <button
                key={screen.id}
                className={screen.id === activeScreen ? "nav-item nav-item--active" : "nav-item"}
                onClick={() => setActiveScreen(screen.id)}
                title={screen.label}
                type="button"
              >
                <Icon aria-hidden="true" size={17} />
                <span>{screen.label}</span>
              </button>
            );
          })}
        </nav>
      </aside>

      <section className="content">
        <header className="topbar">
          <div>
            <h1>{screens.find((screen) => screen.id === activeScreen)?.label}</h1>
            <p>{settings.hotkeys.defaultHotkey}</p>
          </div>
          <OverlayPreview phase={status.workflow.phase} message={statusText} />
        </header>

        {activeScreen === "home" && (
          <section className="panel-grid panel-grid--home">
            <div className="panel">
              <div className="panel-heading">
                <h2>Session</h2>
                <span className={`status-pill status-pill--${status.workflow.phase.toLowerCase()}`}>
                  {status.workflow.phase}
                </span>
              </div>
              <label className="field">
                <span>Mock transcript</span>
                <textarea
                  value={mockTranscript}
                  onChange={(event) => setMockTranscript(event.currentTarget.value)}
                  rows={5}
                />
              </label>
              <div className="button-row">
                <button type="button" onClick={startMockDictation} disabled={isBusy} title="Start">
                  <Play size={16} aria-hidden="true" />
                  Start
                </button>
                <button
                  type="button"
                  onClick={completeMockDictation}
                  disabled={isBusy}
                  title="Finish"
                >
                  <Save size={16} aria-hidden="true" />
                  Finish
                </button>
                <button type="button" onClick={stopSession} disabled={isBusy} title="Cancel">
                  <Square size={16} aria-hidden="true" />
                  Cancel
                </button>
              </div>
            </div>

            <div className="panel">
              <div className="panel-heading">
                <h2>Last Output</h2>
                <FileClock size={18} aria-hidden="true" />
              </div>
              <output className="transcript-output">
                {status.workflow.lastCompleted?.finalText ?? "No completed dictation yet."}
              </output>
            </div>
          </section>
        )}

        {activeScreen === "models" && (
          <section className="panel-grid">
            <SettingsPanel title="ASR">
              <label className="field">
                <span>Whisper model path</span>
                <input
                  value={settings.models.whisperModelPath}
                  onChange={(event) =>
                    updateSettings((current) => ({
                      ...current,
                      models: { ...current.models, whisperModelPath: event.currentTarget.value },
                    }))
                  }
                  placeholder="C:\\models\\ggml-base.en.bin"
                />
              </label>
              <label className="field field--inline">
                <span>Threads</span>
                <input
                  type="number"
                  min="1"
                  max="16"
                  value={settings.models.asrThreads}
                  onChange={(event) =>
                    updateSettings((current) => ({
                      ...current,
                      models: { ...current.models, asrThreads: Number(event.currentTarget.value) },
                    }))
                  }
                />
              </label>
            </SettingsPanel>
            <SettingsPanel title="Refinement">
              <label className="field">
                <span>Ollama model</span>
                <input
                  value={settings.models.ollamaModel}
                  onChange={(event) =>
                    updateSettings((current) => ({
                      ...current,
                      models: { ...current.models, ollamaModel: event.currentTarget.value },
                    }))
                  }
                  placeholder="llama3.1:8b-instruct"
                />
              </label>
              <Toggle
                label="Low-resource mode"
                checked={settings.models.lowResourceMode}
                onChange={(checked) =>
                  updateSettings((current) => ({
                    ...current,
                    models: { ...current.models, lowResourceMode: checked },
                  }))
                }
              />
            </SettingsPanel>
          </section>
        )}

        {activeScreen === "microphone" && (
          <section className="panel-grid">
            <SettingsPanel title="Input">
              <label className="field">
                <span>Device</span>
                <input value={settings.microphone.selectedDeviceName} readOnly />
              </label>
              <div className="meter" aria-label="Input level">
                <span style={{ width: "38%" }} />
              </div>
              <Toggle
                label="Voice activity detection"
                checked={settings.microphone.vadEnabled}
                onChange={(checked) =>
                  updateSettings((current) => ({
                    ...current,
                    microphone: { ...current.microphone, vadEnabled: checked },
                  }))
                }
              />
            </SettingsPanel>
            <SettingsPanel title="Limits">
              <label className="field field--inline">
                <span>End-of-speech ms</span>
                <input
                  type="number"
                  value={settings.microphone.endOfSpeechMs}
                  onChange={(event) =>
                    updateSettings((current) => ({
                      ...current,
                      microphone: {
                        ...current.microphone,
                        endOfSpeechMs: Number(event.currentTarget.value),
                      },
                    }))
                  }
                />
              </label>
              <label className="field field--inline">
                <span>Max seconds</span>
                <input
                  type="number"
                  value={settings.microphone.maxRecordingSeconds}
                  onChange={(event) =>
                    updateSettings((current) => ({
                      ...current,
                      microphone: {
                        ...current.microphone,
                        maxRecordingSeconds: Number(event.currentTarget.value),
                      },
                    }))
                  }
                />
              </label>
            </SettingsPanel>
          </section>
        )}

        {activeScreen === "hotkeys" && (
          <section className="panel-grid">
            <SettingsPanel title="Activation">
              <label className="field">
                <span>Default hotkey</span>
                <input value={settings.hotkeys.defaultHotkey} readOnly />
              </label>
              <div className="segmented" role="group" aria-label="Activation mode">
                {(["push_to_talk", "toggle"] as const).map((mode) => (
                  <button
                    key={mode}
                    className={settings.hotkeys.activationMode === mode ? "selected" : ""}
                    onClick={() =>
                      updateSettings((current) => ({
                        ...current,
                        hotkeys: { ...current.hotkeys, activationMode: mode },
                      }))
                    }
                    type="button"
                  >
                    {mode === "push_to_talk" ? "Push" : "Toggle"}
                  </button>
                ))}
              </div>
            </SettingsPanel>
            <SettingsPanel title="Command Mode">
              <label className="field">
                <span>Command hotkey</span>
                <input value={settings.hotkeys.commandHotkey} readOnly />
              </label>
            </SettingsPanel>
          </section>
        )}

        {activeScreen === "dictionary" && (
          <RowsPanel
            title="Dictionary"
            rows={settings.dictionary.map((entry) => [
              entry.phrase,
              entry.category,
              entry.pronunciationHint ?? "",
            ])}
          />
        )}

        {activeScreen === "replacements" && (
          <RowsPanel
            title="Replacements"
            rows={settings.replacements.map((rule) => [
              rule.incorrect,
              rule.correct,
              rule.enabled ? "On" : "Off",
            ])}
          />
        )}

        {activeScreen === "snippets" && (
          <RowsPanel
            title="Snippets"
            rows={settings.snippets.map((snippet) => [
              snippet.trigger,
              snippet.expansion,
              snippet.allowCleanup ? "Cleanup" : "Exact",
            ])}
          />
        )}

        {activeScreen === "styles" && (
          <section className="panel-grid">
            {settings.styles.map((style) => (
              <SettingsPanel key={style.id} title={style.name}>
                <label className="field">
                  <span>Cleanup</span>
                  <select
                    value={style.cleanupLevel}
                    onChange={(event) =>
                      updateSettings((current) => ({
                        ...current,
                        styles: current.styles.map((item) =>
                          item.id === style.id
                            ? { ...item, cleanupLevel: event.currentTarget.value as CleanupLevel }
                            : item,
                        ),
                      }))
                    }
                  >
                    {cleanupLevels.map((level) => (
                      <option key={level} value={level}>
                        {level}
                      </option>
                    ))}
                  </select>
                </label>
                <label className="field">
                  <span>Conciseness</span>
                  <input type="range" min="1" max="10" value={style.conciseness} readOnly />
                </label>
                <label className="field">
                  <span>Formality</span>
                  <input type="range" min="1" max="10" value={style.formality} readOnly />
                </label>
              </SettingsPanel>
            ))}
          </section>
        )}

        {activeScreen === "privacy" && (
          <section className="panel-grid">
            <SettingsPanel title="Context">
              <Toggle
                label="Active-app detection"
                checked={settings.privacy.activeAppDetection}
                onChange={(checked) =>
                  updateSettings((current) => ({
                    ...current,
                    privacy: { ...current.privacy, activeAppDetection: checked },
                  }))
                }
              />
              <Toggle
                label="Accessibility text context"
                checked={settings.privacy.accessibilityContext}
                onChange={(checked) =>
                  updateSettings((current) => ({
                    ...current,
                    privacy: { ...current.privacy, accessibilityContext: checked },
                  }))
                }
              />
              <Toggle
                label="Selected-text transforms"
                checked={settings.privacy.selectedTextTransforms}
                onChange={(checked) =>
                  updateSettings((current) => ({
                    ...current,
                    privacy: { ...current.privacy, selectedTextTransforms: checked },
                  }))
                }
              />
            </SettingsPanel>
            <SettingsPanel title="Retention">
              <label className="field">
                <span>History</span>
                <select value={settings.privacy.historyRetention} disabled>
                  <option value="off">Off</option>
                  <option value="transcript_only">Transcript only</option>
                  <option value="original_and_cleaned">Original and cleaned</option>
                </select>
              </label>
              <Toggle
                label="Delete audio after processing"
                checked={settings.privacy.deleteAudioAfterProcessing}
                onChange={(checked) =>
                  updateSettings((current) => ({
                    ...current,
                    privacy: { ...current.privacy, deleteAudioAfterProcessing: checked },
                  }))
                }
              />
            </SettingsPanel>
          </section>
        )}

        {activeScreen === "history" && (
          <RowsPanel
            title="History"
            rows={status.history.map((item) => [
              item.completedAt,
              item.targetApplication,
              item.finalText,
            ])}
            empty="No stored dictations."
          />
        )}

        {activeScreen === "diagnostics" && (
          <RowsPanel
            title="Diagnostics"
            rows={status.diagnostics.map((metric) => [metric.label, metric.value, metric.status])}
          />
        )}

        {activeScreen === "about" && (
          <section className="panel-grid">
            <SettingsPanel title="Build">
              <dl className="details">
                <div>
                  <dt>Name</dt>
                  <dd>LocalFlow</dd>
                </div>
                <div>
                  <dt>Version</dt>
                  <dd>0.1.0</dd>
                </div>
                <div>
                  <dt>Default style</dt>
                  <dd>{activeStyle.name}</dd>
                </div>
              </dl>
            </SettingsPanel>
          </section>
        )}
      </section>
    </main>
  );
}

function SettingsPanel({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>{title}</h2>
      </div>
      {children}
    </section>
  );
}

function Toggle({
  label,
  checked,
  onChange,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <label className="toggle">
      <input
        type="checkbox"
        checked={checked}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

function RowsPanel({
  title,
  rows,
  empty = "No rows.",
}: {
  title: string;
  rows: string[][];
  empty?: string;
}) {
  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>{title}</h2>
      </div>
      {rows.length === 0 ? (
        <p className="empty-state">{empty}</p>
      ) : (
        <div className="row-table">
          {rows.map((row) => (
            <div className="row-table__row" key={row.join("|")}>
              {row.map((cell) => (
                <span key={cell}>{cell}</span>
              ))}
            </div>
          ))}
        </div>
      )}
    </section>
  );
}

export default App;
