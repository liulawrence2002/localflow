import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Notice } from "../components/ui/Notice";
import { AboutScreen } from "../screens/AboutScreen";
import { CodingAgentsScreen } from "../screens/CodingAgentsScreen";
import { DiagnosticsScreen } from "../screens/DiagnosticsScreen";
import { DictationScreen } from "../screens/DictationScreen";
import { HomeScreen } from "../screens/HomeScreen";
import { HotkeysScreen } from "../screens/HotkeysScreen";
import { DictionaryScreen, ReplacementsScreen, SnippetsScreen } from "../screens/ListScreens";
import { ModelsScreen } from "../screens/ModelsScreen";
import { PrivacyScreen } from "../screens/PrivacyScreen";
import type { UpdateSettings } from "../screens/types";
import type { LocalFlowSettings } from "../domain/types";
import { ErrorBoundary } from "./ErrorBoundary";
import { findScreen, screens, type ScreenId } from "./screens";
import { useAppStatus, type LoadState, type SaveState } from "./useAppStatus";
import { useLiveStatus } from "./useLiveStatus";

/** The settings window: navigation, the current screen, and whether settings are saved. */
export function App() {
  const [activeScreen, setActiveScreen] = useState<ScreenId>("home");
  const app = useAppStatus();
  const { refreshStatus } = app;
  const onWindowReturn = useCallback(() => void refreshStatus(), [refreshStatus]);
  const live = useLiveStatus(onWindowReturn);
  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const navigatedRef = useRef(false);
  const screen = findScreen(activeScreen);

  // After choosing a section, move the focus to its title so a screen reader starts there.
  useEffect(() => {
    if (navigatedRef.current) {
      headingRef.current?.focus();
    }
  }, [activeScreen]);

  function needsSettings(render: (settings: LocalFlowSettings) => ReactNode): ReactNode {
    if (app.settings) {
      return render(app.settings);
    }
    return <SettingsUnavailable loadState={app.loadState} onRetry={() => void refreshStatus()} />;
  }

  const updateSettings: UpdateSettings = app.updateSettings;

  function renderScreen(): ReactNode {
    switch (activeScreen) {
      case "home":
        return <HomeScreen live={live} status={app.status} applyStatus={app.applyStatus} />;
      case "models":
        return needsSettings((settings) => (
          <ModelsScreen settings={settings} updateSettings={updateSettings} />
        ));
      case "dictation":
        return needsSettings((settings) => (
          <DictationScreen settings={settings} updateSettings={updateSettings} />
        ));
      case "hotkeys":
        return (
          <HotkeysScreen
            diagnostics={app.status?.diagnostics ?? null}
            conversationHotkey={live.conversation?.hotkey}
            readingHotkey={live.screenReading?.hotkey}
          />
        );
      case "dictionary":
        return needsSettings((settings) => (
          <DictionaryScreen settings={settings} updateSettings={updateSettings} />
        ));
      case "replacements":
        return needsSettings((settings) => (
          <ReplacementsScreen settings={settings} updateSettings={updateSettings} />
        ));
      case "snippets":
        return needsSettings((settings) => (
          <SnippetsScreen settings={settings} updateSettings={updateSettings} />
        ));
      case "agents":
        return <CodingAgentsScreen />;
      case "privacy":
        return <PrivacyScreen />;
      case "diagnostics":
        return <DiagnosticsScreen status={app.status} refreshStatus={refreshStatus} />;
      case "about":
        return <AboutScreen />;
    }
  }

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark" aria-hidden="true">
            LF
          </div>
          <div>
            <strong>LocalFlow</strong>
            <span>Dictation and voice, on this PC</span>
          </div>
        </div>
        <nav aria-label="Settings sections">
          <ul className="nav-list">
            {screens.map((entry) => {
              const Icon = entry.icon;
              const isActive = entry.id === activeScreen;
              const flagged = entry.id === "home" && live.problem !== null;
              return (
                <li key={entry.id}>
                  <button
                    className={isActive ? "nav-item nav-item--active" : "nav-item"}
                    aria-current={isActive ? "page" : undefined}
                    onClick={() => {
                      navigatedRef.current = true;
                      setActiveScreen(entry.id);
                    }}
                    type="button"
                  >
                    <Icon aria-hidden="true" size={17} />
                    <span>{entry.label}</span>
                    {flagged ? (
                      <>
                        <span className="nav-badge" aria-hidden="true" />
                        <span className="visually-hidden">, a problem was reported</span>
                      </>
                    ) : null}
                  </button>
                </li>
              );
            })}
          </ul>
        </nav>
      </aside>

      <main className="content">
        <header className="topbar">
          <div>
            <h1 ref={headingRef} tabIndex={-1}>
              {screen.label}
            </h1>
            <p>{screen.description}</p>
          </div>
          <SaveIndicator state={app.saveState} />
        </header>

        {app.saveState.kind === "error" ? (
          <Notice
            tone="error"
            title="Your last change was not saved"
            actions={
              <button type="button" onClick={app.retrySave}>
                <RefreshCw size={16} aria-hidden="true" />
                Try again
              </button>
            }
          >
            {app.saveState.message}
          </Notice>
        ) : null}

        <ErrorBoundary key={activeScreen} label={`The ${screen.label} screen`}>
          {renderScreen()}
        </ErrorBoundary>
      </main>
    </div>
  );
}

/** "Saving..." / "Saved" / "Not saved", announced politely as it changes. */
function SaveIndicator({ state }: { state: SaveState }) {
  const text =
    state.kind === "pending" || state.kind === "saving"
      ? "Saving..."
      : state.kind === "saved"
        ? "All changes saved"
        : state.kind === "error"
          ? "Not saved"
          : "";
  return (
    <p className={`save-indicator save-indicator--${state.kind}`} role="status">
      {text}
    </p>
  );
}

function SettingsUnavailable({
  loadState,
  onRetry,
}: {
  loadState: LoadState;
  onRetry: () => void;
}) {
  if (loadState.kind === "failed") {
    return (
      <Notice
        tone="error"
        title="LocalFlow's settings could not be loaded"
        actions={
          <button type="button" onClick={onRetry}>
            <RefreshCw size={16} aria-hidden="true" />
            Try again
          </button>
        }
      >
        {loadState.message} Nothing can be changed until they load, so that your saved settings are
        not replaced.
      </Notice>
    );
  }
  return (
    <p className="empty-state" role="status">
      Loading settings...
    </p>
  );
}
