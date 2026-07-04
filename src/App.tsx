import {
  Activity,
  BookOpenText,
  Braces,
  ClipboardList,
  Download,
  FileClock,
  Gauge,
  History,
  Info,
  Keyboard,
  Mic,
  Play,
  Plus,
  RefreshCw,
  RotateCcw,
  Save,
  Shield,
  SlidersHorizontal,
  Square,
  Trash2,
} from "lucide-react";
import { useEffect, useState } from "react";
import type { ReactNode } from "react";
import "./App.css";
import { OverlayPreview } from "./components/OverlayPreview";
import { defaultStatus, pinnedOllamaModel } from "./domain/defaults";
import { serializeDiagnosticsExport } from "./domain/diagnostics";
import { checkOllamaStatus, hasOllamaModel, type OllamaConnectionStatus } from "./domain/ollama";
import {
  addDictionaryEntry,
  addReplacementRule,
  addSnippet,
  addStyleProfile,
  appCategories,
  cleanupLevels,
  createCustomStyleDraft,
  removeDictionaryEntry,
  removeReplacementRule,
  removeSnippet,
  removeStyleProfile,
  updateDictionaryEntry,
  updateReplacementRule,
  updateSnippet,
  updateStyleProfile,
} from "./domain/settings";
import { canUndoCleanup, restorePreCleanupText } from "./domain/undo";
import type {
  AppStatus,
  AppCategory,
  CleanupLevel,
  DictionaryEntry,
  LocalFlowSettings,
  ReplacementRule,
  Snippet,
  StyleProfile,
} from "./domain/types";
import {
  beginMockSession,
  cancelSession,
  finishMockSession,
  getStatus,
  saveSettings,
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

type UpdateSettings = (updater: (settings: LocalFlowSettings) => LocalFlowSettings) => void;

export function App() {
  const [activeScreen, setActiveScreen] = useState<ScreenId>("home");
  const [status, setStatus] = useState<AppStatus>(defaultStatus);
  const [mockTranscript, setMockTranscript] = useState(
    "meet me Tuesday no Wednesday comma then review the pie torch model",
  );
  const [isBusy, setIsBusy] = useState(false);
  const [isCheckingOllama, setIsCheckingOllama] = useState(false);
  const [ollamaStatus, setOllamaStatus] = useState<OllamaConnectionStatus>();
  const [diagnosticsExport, setDiagnosticsExport] = useState("");

  useEffect(() => {
    void getStatus().then(setStatus);
  }, []);

  const settings = status.settings;
  const activeStyle = settings.styles[0];
  const statusText = status.workflow.error ?? status.workflow.warning ?? status.workflow.phase;
  const ollamaModels = ollamaStatus?.ok ? ollamaStatus.models : [];
  const selectedModelMissingFromList =
    settings.models.ollamaModel.trim().length > 0 &&
    ollamaModels.length > 0 &&
    !hasOllamaModel(ollamaModels, settings.models.ollamaModel);
  const ollamaMessage = formatOllamaStatus(ollamaStatus);

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

  function undoLastCleanup() {
    const lastCompleted = status.workflow.lastCompleted;
    if (!lastCompleted || !canUndoCleanup(lastCompleted)) {
      return;
    }

    const restored = restorePreCleanupText(lastCompleted);
    setStatus((current) => {
      if (!current.workflow.lastCompleted) {
        return current;
      }

      const restoredItem = {
        ...current.workflow.lastCompleted,
        finalText: restored,
      };

      return {
        ...current,
        workflow: {
          ...current.workflow,
          lastCompleted: restoredItem,
        },
        history: current.history.map((item) => (item.id === restoredItem.id ? restoredItem : item)),
      };
    });
  }

  function updateSettings(updater: (settings: LocalFlowSettings) => LocalFlowSettings) {
    setStatus((current) => {
      const nextSettings = updater(current.settings);
      void saveSettings(nextSettings).then((nextStatus) =>
        setStatus((latest) => ({
          ...latest,
          settings: nextStatus.settings,
          diagnostics: nextStatus.diagnostics,
        })),
      );
      return { ...current, settings: nextSettings };
    });
  }

  async function checkLocalOllama() {
    setIsCheckingOllama(true);
    const nextStatus = await checkOllamaStatus();
    setOllamaStatus(nextStatus);
    setIsCheckingOllama(false);
  }

  function prepareDiagnosticsExport() {
    setDiagnosticsExport(serializeDiagnosticsExport(status));
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
                {canUndoCleanup(status.workflow.lastCompleted) ? (
                  <button type="button" onClick={undoLastCleanup} title="Undo AI cleanup">
                    <RotateCcw size={16} aria-hidden="true" />
                    Undo
                  </button>
                ) : (
                  <FileClock size={18} aria-hidden="true" />
                )}
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
                {ollamaModels.length > 0 ? (
                  <select
                    value={settings.models.ollamaModel}
                    onChange={(event) =>
                      updateSettings((current) => ({
                        ...current,
                        models: { ...current.models, ollamaModel: event.currentTarget.value },
                      }))
                    }
                  >
                    {selectedModelMissingFromList ? (
                      <option value={settings.models.ollamaModel}>
                        {settings.models.ollamaModel}
                      </option>
                    ) : null}
                    {ollamaModels.map((model) => (
                      <option value={model.model} key={model.model}>
                        {formatOllamaModelLabel(model.name, model.sizeBytes)}
                      </option>
                    ))}
                  </select>
                ) : (
                  <input
                    value={settings.models.ollamaModel}
                    onChange={(event) =>
                      updateSettings((current) => ({
                        ...current,
                        models: { ...current.models, ollamaModel: event.currentTarget.value },
                      }))
                    }
                    placeholder={pinnedOllamaModel}
                  />
                )}
                <span className="field-hint">
                  Native dictation is pinned to {pinnedOllamaModel}.
                </span>
              </label>
              <div className="button-row button-row--status">
                <button
                  type="button"
                  onClick={checkLocalOllama}
                  disabled={isCheckingOllama}
                  title="Check local Ollama"
                >
                  <RefreshCw size={16} aria-hidden="true" />
                  Check
                </button>
                {ollamaMessage ? (
                  <span
                    className={ollamaStatus?.ok ? "status-pill" : "status-pill status-pill--error"}
                  >
                    {ollamaMessage}
                  </span>
                ) : null}
              </div>
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
          <DictionaryEditor settings={settings} updateSettings={updateSettings} />
        )}

        {activeScreen === "replacements" && (
          <ReplacementEditor settings={settings} updateSettings={updateSettings} />
        )}

        {activeScreen === "snippets" && (
          <SnippetEditor settings={settings} updateSettings={updateSettings} />
        )}

        {activeScreen === "styles" && (
          <StyleEditor settings={settings} updateSettings={updateSettings} />
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
          <section className="panel-grid">
            <RowsPanel
              title="Diagnostics"
              rows={status.diagnostics.map((metric) => [metric.label, metric.value, metric.status])}
            />
            <SettingsPanel title="Export">
              <div className="button-row">
                <button
                  type="button"
                  onClick={prepareDiagnosticsExport}
                  title="Prepare diagnostics export"
                >
                  <Download size={16} aria-hidden="true" />
                  Prepare
                </button>
              </div>
              <textarea aria-label="Diagnostics export" value={diagnosticsExport} readOnly />
            </SettingsPanel>
          </section>
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

function DictionaryEditor({
  settings,
  updateSettings,
}: {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}) {
  const [draft, setDraft] = useState<Omit<DictionaryEntry, "id">>({
    phrase: "",
    pronunciationHint: "",
    category: "custom",
    caseSensitive: false,
  });

  function addEntry() {
    updateSettings((current) => addDictionaryEntry(current, draft));
    setDraft({ phrase: "", pronunciationHint: "", category: "custom", caseSensitive: false });
  }

  return (
    <SettingsPanel title="Dictionary">
      <div className="inline-form inline-form--dictionary">
        <label className="field">
          <span>Phrase</span>
          <input
            value={draft.phrase}
            onChange={(event) =>
              setDraft((current) => ({ ...current, phrase: event.currentTarget.value }))
            }
          />
        </label>
        <label className="field">
          <span>Hint</span>
          <input
            value={draft.pronunciationHint}
            onChange={(event) =>
              setDraft((current) => ({ ...current, pronunciationHint: event.currentTarget.value }))
            }
          />
        </label>
        <label className="field">
          <span>Category</span>
          <select
            value={draft.category}
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                category: event.currentTarget.value as DictionaryEntry["category"],
              }))
            }
          >
            <option value="custom">Custom</option>
            <option value="name">Name</option>
            <option value="acronym">Acronym</option>
            <option value="technical">Technical</option>
          </select>
        </label>
        <Toggle
          label="Case-sensitive"
          checked={draft.caseSensitive}
          onChange={(checked) => setDraft((current) => ({ ...current, caseSensitive: checked }))}
        />
        <button type="button" onClick={addEntry} title="Add dictionary entry">
          <Plus size={16} aria-hidden="true" />
          Add
        </button>
      </div>

      <div className="setting-list">
        {settings.dictionary.map((entry) => (
          <div className="setting-row setting-row--dictionary" key={entry.id}>
            <input
              aria-label="Dictionary phrase"
              value={entry.phrase}
              onChange={(event) =>
                updateSettings((current) =>
                  updateDictionaryEntry(current, entry.id, { phrase: event.currentTarget.value }),
                )
              }
            />
            <input
              aria-label="Pronunciation hint"
              value={entry.pronunciationHint ?? ""}
              onChange={(event) =>
                updateSettings((current) =>
                  updateDictionaryEntry(current, entry.id, {
                    pronunciationHint: event.currentTarget.value,
                  }),
                )
              }
            />
            <select
              aria-label="Dictionary category"
              value={entry.category}
              onChange={(event) =>
                updateSettings((current) =>
                  updateDictionaryEntry(current, entry.id, {
                    category: event.currentTarget.value as DictionaryEntry["category"],
                  }),
                )
              }
            >
              <option value="custom">Custom</option>
              <option value="name">Name</option>
              <option value="acronym">Acronym</option>
              <option value="technical">Technical</option>
            </select>
            <Toggle
              label="Case-sensitive"
              checked={entry.caseSensitive}
              onChange={(checked) =>
                updateSettings((current) =>
                  updateDictionaryEntry(current, entry.id, { caseSensitive: checked }),
                )
              }
            />
            <IconButton
              label="Remove dictionary entry"
              onClick={() => updateSettings((current) => removeDictionaryEntry(current, entry.id))}
            />
          </div>
        ))}
      </div>
    </SettingsPanel>
  );
}

function ReplacementEditor({
  settings,
  updateSettings,
}: {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}) {
  const [draft, setDraft] = useState<Omit<ReplacementRule, "id">>({
    incorrect: "",
    correct: "",
    enabled: true,
  });

  function addRule() {
    updateSettings((current) => addReplacementRule(current, draft));
    setDraft({ incorrect: "", correct: "", enabled: true });
  }

  return (
    <SettingsPanel title="Replacements">
      <div className="inline-form inline-form--replacement">
        <label className="field">
          <span>Heard</span>
          <input
            value={draft.incorrect}
            onChange={(event) =>
              setDraft((current) => ({ ...current, incorrect: event.currentTarget.value }))
            }
          />
        </label>
        <label className="field">
          <span>Write</span>
          <input
            value={draft.correct}
            onChange={(event) =>
              setDraft((current) => ({ ...current, correct: event.currentTarget.value }))
            }
          />
        </label>
        <Toggle
          label="Enabled"
          checked={draft.enabled}
          onChange={(checked) => setDraft((current) => ({ ...current, enabled: checked }))}
        />
        <button type="button" onClick={addRule} title="Add replacement">
          <Plus size={16} aria-hidden="true" />
          Add
        </button>
      </div>

      <div className="setting-list">
        {settings.replacements.map((rule) => (
          <div className="setting-row setting-row--replacement" key={rule.id}>
            <input
              aria-label="Incorrect phrase"
              value={rule.incorrect}
              onChange={(event) =>
                updateSettings((current) =>
                  updateReplacementRule(current, rule.id, { incorrect: event.currentTarget.value }),
                )
              }
            />
            <input
              aria-label="Correct phrase"
              value={rule.correct}
              onChange={(event) =>
                updateSettings((current) =>
                  updateReplacementRule(current, rule.id, { correct: event.currentTarget.value }),
                )
              }
            />
            <Toggle
              label="Enabled"
              checked={rule.enabled}
              onChange={(checked) =>
                updateSettings((current) =>
                  updateReplacementRule(current, rule.id, { enabled: checked }),
                )
              }
            />
            <IconButton
              label="Remove replacement"
              onClick={() => updateSettings((current) => removeReplacementRule(current, rule.id))}
            />
          </div>
        ))}
      </div>
    </SettingsPanel>
  );
}

function SnippetEditor({
  settings,
  updateSettings,
}: {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}) {
  const [draft, setDraft] = useState<Omit<Snippet, "id">>({
    trigger: "",
    expansion: "",
    enabled: true,
    allowCleanup: false,
  });

  function addExactSnippet() {
    updateSettings((current) => addSnippet(current, draft));
    setDraft({ trigger: "", expansion: "", enabled: true, allowCleanup: false });
  }

  return (
    <SettingsPanel title="Snippets">
      <div className="inline-form inline-form--snippet">
        <label className="field">
          <span>Trigger</span>
          <input
            value={draft.trigger}
            onChange={(event) =>
              setDraft((current) => ({ ...current, trigger: event.currentTarget.value }))
            }
          />
        </label>
        <label className="field">
          <span>Expansion</span>
          <textarea
            value={draft.expansion}
            onChange={(event) =>
              setDraft((current) => ({ ...current, expansion: event.currentTarget.value }))
            }
            rows={3}
          />
        </label>
        <Toggle
          label="Enabled"
          checked={draft.enabled}
          onChange={(checked) => setDraft((current) => ({ ...current, enabled: checked }))}
        />
        <Toggle
          label="Cleanup"
          checked={draft.allowCleanup}
          onChange={(checked) => setDraft((current) => ({ ...current, allowCleanup: checked }))}
        />
        <button type="button" onClick={addExactSnippet} title="Add snippet">
          <Plus size={16} aria-hidden="true" />
          Add
        </button>
      </div>

      <div className="setting-list">
        {settings.snippets.map((snippet) => (
          <div className="setting-row setting-row--snippet" key={snippet.id}>
            <input
              aria-label="Snippet trigger"
              value={snippet.trigger}
              onChange={(event) =>
                updateSettings((current) =>
                  updateSnippet(current, snippet.id, { trigger: event.currentTarget.value }),
                )
              }
            />
            <textarea
              aria-label="Snippet expansion"
              value={snippet.expansion}
              onChange={(event) =>
                updateSettings((current) =>
                  updateSnippet(current, snippet.id, { expansion: event.currentTarget.value }),
                )
              }
              rows={3}
            />
            <Toggle
              label="Enabled"
              checked={snippet.enabled}
              onChange={(checked) =>
                updateSettings((current) =>
                  updateSnippet(current, snippet.id, { enabled: checked }),
                )
              }
            />
            <Toggle
              label="Cleanup"
              checked={snippet.allowCleanup}
              onChange={(checked) =>
                updateSettings((current) =>
                  updateSnippet(current, snippet.id, { allowCleanup: checked }),
                )
              }
            />
            <IconButton
              label="Remove snippet"
              onClick={() => updateSettings((current) => removeSnippet(current, snippet.id))}
            />
          </div>
        ))}
      </div>
    </SettingsPanel>
  );
}

function StyleEditor({
  settings,
  updateSettings,
}: {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}) {
  return (
    <section className="panel-grid">
      <section className="panel panel--wide">
        <div className="panel-heading">
          <h2>Profiles</h2>
          <button
            type="button"
            onClick={() =>
              updateSettings((current) => addStyleProfile(current, createCustomStyleDraft()))
            }
            title="Add style profile"
          >
            <Plus size={16} aria-hidden="true" />
            Add
          </button>
        </div>
      </section>

      {settings.styles.map((style) => (
        <StyleProfilePanel
          key={style.id}
          style={style}
          canRemove={settings.styles.length > 1}
          updateSettings={updateSettings}
        />
      ))}
    </section>
  );
}

function StyleProfilePanel({
  style,
  canRemove,
  updateSettings,
}: {
  style: StyleProfile;
  canRemove: boolean;
  updateSettings: UpdateSettings;
}) {
  function patchStyle(patch: Partial<Omit<StyleProfile, "id">>) {
    updateSettings((current) => updateStyleProfile(current, style.id, patch));
  }

  return (
    <section className="panel">
      <div className="panel-heading">
        <h2>{style.name}</h2>
        {canRemove && (
          <IconButton
            label="Remove style profile"
            onClick={() => updateSettings((current) => removeStyleProfile(current, style.id))}
          />
        )}
      </div>
      <label className="field">
        <span>Name</span>
        <input
          value={style.name}
          onChange={(event) => patchStyle({ name: event.currentTarget.value })}
        />
      </label>
      <label className="field">
        <span>Category</span>
        <select
          value={style.category}
          onChange={(event) => patchStyle({ category: event.currentTarget.value as AppCategory })}
        >
          {appCategories.map((category) => (
            <option key={category} value={category}>
              {formatOptionLabel(category)}
            </option>
          ))}
        </select>
      </label>
      <label className="field">
        <span>Cleanup</span>
        <select
          value={style.cleanupLevel}
          onChange={(event) =>
            patchStyle({ cleanupLevel: event.currentTarget.value as CleanupLevel })
          }
        >
          {cleanupLevels.map((level) => (
            <option key={level} value={level}>
              {formatOptionLabel(level)}
            </option>
          ))}
        </select>
      </label>
      <RangeField
        label="Conciseness"
        value={style.conciseness}
        onChange={(value) => patchStyle({ conciseness: value })}
      />
      <RangeField
        label="Formality"
        value={style.formality}
        onChange={(value) => patchStyle({ formality: value })}
      />
      <label className="field">
        <span>Emoji</span>
        <select
          value={style.emoji}
          onChange={(event) =>
            patchStyle({ emoji: event.currentTarget.value as StyleProfile["emoji"] })
          }
        >
          <option value="never">Never</option>
          <option value="preserve">Preserve</option>
          <option value="sparingly">Sparingly</option>
        </select>
      </label>
      <label className="field">
        <span>Paragraphs</span>
        <select
          value={style.paragraphLength}
          onChange={(event) =>
            patchStyle({
              paragraphLength: event.currentTarget.value as StyleProfile["paragraphLength"],
            })
          }
        >
          <option value="short">Short</option>
          <option value="medium">Medium</option>
          <option value="long">Long</option>
        </select>
      </label>
      <label className="field">
        <span>Bullets</span>
        <select
          value={style.bulletPreference}
          onChange={(event) =>
            patchStyle({
              bulletPreference: event.currentTarget.value as StyleProfile["bulletPreference"],
            })
          }
        >
          <option value="preserve">Preserve</option>
          <option value="prefer">Prefer</option>
          <option value="avoid">Avoid</option>
        </select>
      </label>
      <div className="toggle-grid">
        <Toggle
          label="Contractions"
          checked={style.contractions}
          onChange={(checked) => patchStyle({ contractions: checked })}
        />
        <Toggle
          label="Remove fillers"
          checked={style.aggressiveFillerRemoval}
          onChange={(checked) => patchStyle({ aggressiveFillerRemoval: checked })}
        />
        <Toggle
          label="Fragments"
          checked={style.allowSentenceFragments}
          onChange={(checked) => patchStyle({ allowSentenceFragments: checked })}
        />
      </div>
      <div className="two-column-fields">
        <label className="field">
          <span>Greeting</span>
          <select
            value={style.greetingBehavior}
            onChange={(event) =>
              patchStyle({
                greetingBehavior: event.currentTarget.value as StyleProfile["greetingBehavior"],
              })
            }
          >
            <option value="preserve">Preserve</option>
            <option value="add_when_missing">Add</option>
            <option value="avoid">Avoid</option>
          </select>
        </label>
        <label className="field">
          <span>Sign-off</span>
          <select
            value={style.signOffBehavior}
            onChange={(event) =>
              patchStyle({
                signOffBehavior: event.currentTarget.value as StyleProfile["signOffBehavior"],
              })
            }
          >
            <option value="preserve">Preserve</option>
            <option value="add_when_missing">Add</option>
            <option value="avoid">Avoid</option>
          </select>
        </label>
      </div>
    </section>
  );
}

function RangeField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <label className="field">
      <span>{label}</span>
      <div className="range-field">
        <input
          type="range"
          min="1"
          max="10"
          value={value}
          onChange={(event) => onChange(Number(event.currentTarget.value))}
        />
        <output>{value}</output>
      </div>
    </label>
  );
}

function IconButton({ label, onClick }: { label: string; onClick: () => void }) {
  return (
    <button
      className="icon-button"
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
    >
      <Trash2 size={16} aria-hidden="true" />
    </button>
  );
}

function formatOptionLabel(value: string): string {
  return value
    .split("_")
    .map((part) => `${part.charAt(0).toUpperCase()}${part.slice(1)}`)
    .join(" ");
}

function formatOllamaStatus(status: OllamaConnectionStatus | undefined): string {
  if (!status) {
    return "";
  }

  if (!status.ok) {
    return status.message;
  }

  if (status.models.length === 0) {
    return "No local models found.";
  }

  return `${status.models.length} local model${status.models.length === 1 ? "" : "s"}`;
}

function formatOllamaModelLabel(name: string, sizeBytes: number | undefined): string {
  return sizeBytes ? `${name} (${formatBytes(sizeBytes)})` : name;
}

function formatBytes(sizeBytes: number): string {
  const gib = sizeBytes / 1024 ** 3;
  return `${gib.toFixed(gib >= 10 ? 0 : 1)} GiB`;
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
