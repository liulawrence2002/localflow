import { Play, RotateCcw, Save, Square } from "lucide-react";
import { useState } from "react";
import { Panel } from "../components/ui/Panel";
import type { AppStatus } from "../domain/types";
import { canUndoCleanup, restorePreCleanupText } from "../domain/undo";
import {
  beginMockSession,
  cancelSession,
  errorMessage,
  finishMockSession,
} from "../services/localflowClient";

/**
 * Development builds only: run the simulated pipeline without a microphone. Real dictation
 * runs from the global hotkey into the focused app.
 */
export function DevSimulationPanels({
  status,
  applyStatus,
}: {
  status: AppStatus;
  applyStatus: (status: AppStatus) => void;
}) {
  const [transcript, setTranscript] = useState(
    "meet me Tuesday no Wednesday comma then review the pie torch model",
  );
  const [isBusy, setIsBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function run(action: () => Promise<AppStatus>) {
    setIsBusy(true);
    setError(null);
    try {
      applyStatus(await action());
    } catch (reason) {
      setError(errorMessage(reason, "The simulated step failed."));
    } finally {
      setIsBusy(false);
    }
  }

  const lastCompleted = status.workflow.lastCompleted;

  function undoLastCleanup() {
    if (!lastCompleted || !canUndoCleanup(lastCompleted)) {
      return;
    }
    const restoredItem = { ...lastCompleted, finalText: restorePreCleanupText(lastCompleted) };
    applyStatus({
      ...status,
      workflow: { ...status.workflow, lastCompleted: restoredItem },
      history: status.history.map((item) => (item.id === restoredItem.id ? restoredItem : item)),
    });
  }

  return (
    <>
      <Panel
        title="Simulated test (development build)"
        description="Runs the clean-up pipeline on typed text. Real dictation runs from the global hotkey into the focused app."
      >
        <div className="field">
          <label className="field-label" htmlFor="dev-transcript">
            Test transcript
          </label>
          <textarea
            id="dev-transcript"
            value={transcript}
            onChange={(event) => setTranscript(event.currentTarget.value)}
            rows={4}
          />
        </div>
        <div className="button-row">
          <button
            type="button"
            disabled={isBusy}
            onClick={() =>
              void run(async () => ({ ...status, workflow: await beginMockSession() }))
            }
          >
            <Play size={16} aria-hidden="true" />
            Start
          </button>
          <button
            type="button"
            disabled={isBusy}
            onClick={() => void run(() => finishMockSession(transcript))}
          >
            <Save size={16} aria-hidden="true" />
            Finish
          </button>
          <button
            type="button"
            disabled={isBusy}
            onClick={() => void run(async () => ({ ...status, workflow: await cancelSession() }))}
          >
            <Square size={16} aria-hidden="true" />
            Cancel
          </button>
        </div>
        <p className="field-hint">Simulated phase: {status.workflow.phase}</p>
        {error ? (
          <p className="field-hint field-hint--error" role="alert">
            {error}
          </p>
        ) : null}
      </Panel>

      <Panel
        title="Last simulated output"
        actions={
          canUndoCleanup(lastCompleted) ? (
            <button type="button" onClick={undoLastCleanup}>
              <RotateCcw size={16} aria-hidden="true" />
              Undo clean-up
            </button>
          ) : undefined
        }
      >
        <output className="transcript-output">
          {lastCompleted?.finalText ?? "No simulated dictation yet."}
        </output>
      </Panel>
    </>
  );
}
