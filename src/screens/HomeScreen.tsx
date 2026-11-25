import { ClipboardList, Eye, EyeOff, Mic, RefreshCw, Square, Volume2 } from "lucide-react";
import { useState } from "react";
import type { LiveStatus } from "../app/useLiveStatus";
import { StatusPill } from "../components/ui/Buttons";
import { LiveMessage, Notice } from "../components/ui/Notice";
import { Panel, PanelGrid } from "../components/ui/Panel";
import {
  describeConversation,
  describeLastStop,
  describeScreenReading,
} from "../domain/conversation";
import { formatClockTime } from "../domain/format";
import { describeHotkeys, dictationHotkey } from "../domain/hotkeys";
import { describeReadAloud, type NativeProblem } from "../domain/nativeEvents";
import { buildSetupChecklist } from "../domain/readiness";
import { describeLastTranscript } from "../domain/recovery";
import type { AppStatus } from "../domain/types";
import {
  copyLastTranscript,
  errorMessage,
  isDesktopApp,
  speakLastTranscript,
  stopSpeaking,
  toggleConversation,
} from "../services/localflowClient";
import { DevSimulationPanels } from "./DevSimulationPanels";

export interface HomeScreenProps {
  live: LiveStatus;
  status: AppStatus | null;
  applyStatus: (status: AppStatus) => void;
}

type Message = { text: string; tone: "status" | "alert" } | null;

export function HomeScreen({ live, status, applyStatus }: HomeScreenProps) {
  const [recoveryMessage, setRecoveryMessage] = useState<Message>(null);
  const [revealTranscript, setRevealTranscript] = useState(false);
  const [conversationError, setConversationError] = useState<string | null>(null);
  const [isToggling, setIsToggling] = useState(false);
  const desktop = isDesktopApp();

  async function copyTranscript() {
    try {
      await copyLastTranscript();
      setRecoveryMessage({ text: "Copied. Paste it where you want with Ctrl+V.", tone: "status" });
    } catch (reason) {
      setRecoveryMessage({
        text: errorMessage(reason, "The transcript could not be copied."),
        tone: "alert",
      });
    }
  }

  async function readAloud() {
    setRecoveryMessage(null);
    live.setReadAloud({ phase: "synthesizing", message: "Preparing speech" });
    try {
      await speakLastTranscript();
    } catch (reason) {
      live.setReadAloud({
        phase: "error",
        message: errorMessage(reason, "The transcript could not be read aloud."),
      });
    }
  }

  async function stopReading() {
    try {
      await stopSpeaking();
      live.setReadAloud({ phase: "idle", message: "Stopped" });
    } catch (reason) {
      live.setReadAloud({
        phase: "error",
        message: errorMessage(reason, "Reading could not be stopped."),
      });
    }
  }

  async function pressConversationButton() {
    setIsToggling(true);
    setConversationError(null);
    try {
      await toggleConversation();
      await live.refreshConversation();
    } catch (reason) {
      setConversationError(errorMessage(reason, "The conversation could not be started."));
    } finally {
      setIsToggling(false);
    }
  }

  const checklist = buildSetupChecklist(live.readiness);
  const conversation = live.conversation;
  const conversationView =
    desktop && conversation === undefined
      ? {
          headline: "Checking conversation mode...",
          buttonLabel: "Start conversation",
          buttonDisabled: true,
          microphoneOpen: false,
        }
      : describeConversation(conversation ?? null);
  const lastStop =
    conversation && conversation.phase === "off"
      ? describeLastStop(
          conversation.lastStop !== undefined
            ? conversation.lastStop
            : live.conversationStopFromEvent,
        )
      : null;
  const readingText =
    desktop && live.screenReading === undefined
      ? "Checking whether windows can be read aloud..."
      : describeScreenReading(live.screenReading ?? null);
  const recovery = describeLastTranscript(live.lastTranscript, revealTranscript);
  const readAloudView = describeReadAloud(live.readAloud);
  const ttsAvailable = live.tts?.available === true;
  const stopEnabled = readAloudView.stopEnabled || live.tts?.speaking === true;
  const dictationRow = describeHotkeys({
    diagnostics: status?.diagnostics ?? null,
    conversationHotkey: conversation?.hotkey,
    readingHotkey: live.screenReading?.hotkey,
  })[0];

  return (
    <>
      {live.problem ? (
        <ProblemBanner
          problem={live.problem}
          onDismiss={live.dismissProblem}
          onCopy={() => void copyTranscript()}
        />
      ) : null}
      {live.liveError ? (
        <Notice tone="warning" title="LocalFlow's desktop app is not answering">
          {live.liveError}
        </Notice>
      ) : null}

      <PanelGrid home>
        {checklist.visible ? (
          <Panel
            title="Setup"
            wide
            description={`${checklist.missingRequired} required part${
              checklist.missingRequired === 1 ? " is" : "s are"
            } missing. Do what is listed; the list is checked again when you come back to this window.`}
          >
            <ul className="checklist">
              {checklist.items.map((item) => (
                <li className="checklist__item" key={item.id}>
                  <StatusPill tone={item.ready ? "ok" : item.required ? "error" : "warning"}>
                    {item.ready ? "Ready" : item.required ? "Missing" : "Optional"}
                  </StatusPill>
                  <div className="checklist__text">
                    <strong>{item.label}</strong>
                    {!item.ready && item.fix ? <code className="command">{item.fix}</code> : null}
                  </div>
                </li>
              ))}
            </ul>
          </Panel>
        ) : null}

        <Panel title="Conversation">
          <p className="field-hint">
            Talk with the local language model hands-free. Speech recognition, the model and the
            voice run on this computer, and nothing that is said is stored. The one exception is a
            coding-agent question you confirm by voice: that question goes to Claude Code.
          </p>
          <p className="state-line" role="status" aria-live="polite">
            {conversationView.headline}
          </p>
          <div className="button-row">
            <button
              type="button"
              className="button--primary"
              onClick={() => void pressConversationButton()}
              disabled={conversationView.buttonDisabled || isToggling}
            >
              <Mic size={16} aria-hidden="true" />
              {conversationView.buttonLabel}
            </button>
          </div>
          {conversation?.detail ? <p className="field-hint">{conversation.detail}</p> : null}
          {conversationError ? (
            <Notice tone="error" title="The conversation button did not work">
              {conversationError}
            </Notice>
          ) : null}
          {lastStop ? (
            lastStop.tone === "problem" ? (
              <Notice
                tone="error"
                title={`The last conversation ended with a problem at ${formatClockTime(lastStop.atMs)}`}
              >
                {lastStop.text}
              </Notice>
            ) : (
              <p className="field-hint">
                Last conversation, {formatClockTime(lastStop.atMs)}: {lastStop.text}
              </p>
            )
          ) : null}
        </Panel>

        <Panel title="Dictation">
          <p className="field-hint">
            Click into a text field and press {dictationHotkey}. Tap it, speak and pause, or hold it
            while you speak and let go. The text is pasted where you started.
          </p>
          {dictationRow && dictationRow.tone === "warning" ? (
            <Notice tone="warning" title={`${dictationHotkey} is not registered`}>
              Another program may hold it. The Hotkeys screen shows which keys work.
            </Notice>
          ) : null}
        </Panel>

        <Panel title="Reading a window aloud">
          <p className="field-hint">{readingText}</p>
        </Panel>

        <Panel
          title="Recovery"
          actions={
            <button type="button" onClick={() => void live.refreshLastTranscript()}>
              <RefreshCw size={16} aria-hidden="true" />
              Refresh
            </button>
          }
        >
          <p className="field-hint">
            {recovery.available
              ? `The last dictation's transcript is here (${recovery.charCount} characters). ` +
                "Use it when a paste was skipped because the focus moved."
              : "No transcript yet. After a dictation, its transcript can be copied here or from the tray menu."}
          </p>
          {recovery.preview !== null ? (
            <output className="transcript-output" aria-label="Last transcript">
              {recovery.preview}
            </output>
          ) : null}
          <div className="button-row">
            <button
              type="button"
              onClick={() => void copyTranscript()}
              disabled={!recovery.available}
            >
              <ClipboardList size={16} aria-hidden="true" />
              Copy last transcript
            </button>
            <button
              type="button"
              aria-pressed={revealTranscript}
              onClick={() => setRevealTranscript((value) => !value)}
              disabled={!recovery.available}
            >
              {revealTranscript ? (
                <EyeOff size={16} aria-hidden="true" />
              ) : (
                <Eye size={16} aria-hidden="true" />
              )}
              Show transcript
            </button>
            <button
              type="button"
              onClick={() => void readAloud()}
              disabled={!recovery.available || !ttsAvailable || readAloudView.stopEnabled}
            >
              <Volume2 size={16} aria-hidden="true" />
              Read aloud with Fish S2
            </button>
            <button type="button" onClick={() => void stopReading()} disabled={!stopEnabled}>
              <Square size={16} aria-hidden="true" />
              Stop reading
            </button>
          </div>
          {live.tts && !live.tts.available ? (
            <p className="field-hint">
              Reading a transcript aloud uses the optional Fish S2 Pro voice (a 3.6 GB download, for
              personal non-commercial use only). It is not installed: {live.tts.detail} See
              docs\MODEL_SETUP.md.
            </p>
          ) : null}
          <LiveMessage message={readAloudView.notice} tone={readAloudView.tone} />
          <LiveMessage
            message={recoveryMessage?.text ?? null}
            tone={recoveryMessage?.tone ?? "status"}
          />
        </Panel>

        {import.meta.env.DEV && status ? (
          <DevSimulationPanels status={status} applyStatus={applyStatus} />
        ) : null}
      </PanelGrid>
    </>
  );
}

function ProblemBanner({
  problem,
  onDismiss,
  onCopy,
}: {
  problem: NativeProblem;
  onDismiss: () => void;
  onCopy: () => void;
}) {
  const action = problem.action;
  return (
    <Notice
      tone="error"
      title={`Something went wrong at ${formatClockTime(problem.atMs)}`}
      onDismiss={onDismiss}
      dismissLabel="Dismiss this problem"
      actions={
        action?.kind === "copy_transcript" ? (
          <button type="button" onClick={onCopy}>
            <ClipboardList size={16} aria-hidden="true" />
            Copy last transcript
          </button>
        ) : undefined
      }
    >
      <p>{problem.message}</p>
      {action?.kind === "hint" ? <p>{action.text}</p> : null}
    </Notice>
  );
}
