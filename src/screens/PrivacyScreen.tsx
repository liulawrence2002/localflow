import { ClipboardList } from "lucide-react";
import { useState } from "react";
import { LiveMessage } from "../components/ui/Notice";
import { Panel, PanelGrid } from "../components/ui/Panel";

export const privacyDocUrl =
  "https://github.com/liulawrence2002/localflow/blob/master/docs/PRIVACY.md";

/**
 * What LocalFlow keeps and sends, stated as facts. There are no switches here: the context and
 * retention settings are not used by the native app, and a setting that does nothing is not
 * shown.
 */
export function PrivacyScreen() {
  const [copyMessage, setCopyMessage] = useState<string | null>(null);

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(privacyDocUrl);
      setCopyMessage("Link copied. Paste it into your browser.");
    } catch {
      setCopyMessage("The link could not be copied. Select it and press Ctrl+C.");
    }
  }

  return (
    <PanelGrid>
      <Panel title="What LocalFlow keeps" wide>
        <ul className="fact-list fact-list--large">
          <li>
            <strong>Dictations are not stored.</strong> The last transcript is held in memory for
            recovery until the next dictation or until LocalFlow closes.
          </li>
          <li>
            <strong>Audio is deleted right after transcription.</strong> No recording is kept.
          </li>
          <li>
            <strong>
              Nothing leaves this computer, except a coding-agent question you confirm by voice.
            </strong>{" "}
            That question goes to Claude Code, which sends it to Anthropic under your own
            subscription. Speech recognition, the language model (Ollama on 127.0.0.1) and the
            voices all run here.
          </li>
          <li>Logs record phases, counts and timings, never what was said, read or answered.</li>
        </ul>
      </Panel>

      <Panel title="The full statement">
        <p className="field-hint">
          The privacy statement is docs\PRIVACY.md in your LocalFlow folder, and online at:
        </p>
        <p className="link-line">
          <code className="command">{privacyDocUrl}</code>
        </p>
        <div className="button-row">
          <button type="button" onClick={() => void copyLink()}>
            <ClipboardList size={16} aria-hidden="true" />
            Copy link
          </button>
        </div>
        <LiveMessage message={copyMessage} />
      </Panel>
    </PanelGrid>
  );
}
