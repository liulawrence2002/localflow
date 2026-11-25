import { NumberField } from "../components/ui/Fields";
import { Panel, PanelGrid } from "../components/ui/Panel";
import { dictationHotkey } from "../domain/hotkeys";
import { endOfSpeechRange } from "../domain/settings";
import type { LocalFlowSettings } from "../domain/types";
import type { UpdateSettings } from "./types";

export function DictationScreen({
  settings,
  updateSettings,
}: {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}) {
  return (
    <PanelGrid>
      <Panel title="Starting and stopping">
        <ul className="fact-list">
          <li>
            Click into a text field and press <kbd>{dictationHotkey}</kbd>.
          </li>
          <li>
            <strong>Tap</strong> it and speak: dictation stops by itself once you pause for the
            end-of-speech wait.
          </li>
          <li>
            <strong>Hold</strong> it while you speak and let go when you finish. A held hotkey is
            never stopped early, however long you pause.
          </li>
          <li>
            Press it again to stop early, or <kbd>Escape</kbd> to cancel without pasting.
          </li>
        </ul>
      </Panel>

      <Panel title="End of speech">
        <NumberField
          label="End-of-speech wait (ms)"
          value={settings.microphone.endOfSpeechMs}
          min={endOfSpeechRange.min}
          max={endOfSpeechRange.max}
          unit="ms"
          onCommit={(endOfSpeechMs) =>
            updateSettings((current) => ({
              ...current,
              microphone: { ...current.microphone, endOfSpeechMs },
            }))
          }
          hint={`After a tap, how long a pause ends the dictation, from ${endOfSpeechRange.min} to ${endOfSpeechRange.max} ms. Shorter stops sooner; longer leaves time to think mid-sentence. The default is 900 ms. Holding the hotkey ignores it.`}
        />
      </Panel>

      <Panel title="Microphone">
        <p className="field-hint">
          LocalFlow records from the Windows default microphone. To use another one, change the
          default input in Windows Settings &gt; System &gt; Sound. With a microphone that has
          several channels, the loudest one is used.
        </p>
      </Panel>
    </PanelGrid>
  );
}
