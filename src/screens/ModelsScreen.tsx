import { RefreshCw } from "lucide-react";
import { useState } from "react";
import { StatusPill } from "../components/ui/Buttons";
import { SelectField, TextField, Toggle, type SelectOption } from "../components/ui/Fields";
import { LiveMessage } from "../components/ui/Notice";
import { Panel, PanelGrid } from "../components/ui/Panel";
import { pinnedOllamaModel } from "../domain/defaults";
import { formatOllamaModelLabel, formatOllamaStatus } from "../domain/format";
import { hasOllamaModel, type OllamaConnectionStatus } from "../domain/ollama";
import type { LocalFlowSettings } from "../domain/types";
import { checkLocalModels } from "../services/localflowClient";
import type { UpdateSettings } from "./types";

export function ModelsScreen({
  settings,
  updateSettings,
}: {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}) {
  const [ollamaStatus, setOllamaStatus] = useState<OllamaConnectionStatus>();
  const [isChecking, setIsChecking] = useState(false);

  async function checkOllama() {
    setIsChecking(true);
    try {
      setOllamaStatus(await checkLocalModels());
    } finally {
      setIsChecking(false);
    }
  }

  const models = ollamaStatus?.ok ? ollamaStatus.models : [];
  const selected = settings.models.ollamaModel;
  const options: SelectOption<string>[] = models.map((model) => ({
    value: model.model,
    label: formatOllamaModelLabel(model.name, model.sizeBytes),
  }));
  if (selected.trim() && models.length > 0 && !hasOllamaModel(models, selected)) {
    options.unshift({ value: selected, label: `${selected} (not installed)` });
  }
  const setOllamaModel = (ollamaModel: string) =>
    updateSettings((current) => ({ ...current, models: { ...current.models, ollamaModel } }));
  const modelHint = (
    <>
      The same model answers in conversations and cleans up dictated text in the background.
      LocalFlow uses {pinnedOllamaModel} when this is empty. Models with a cloud tag are refused,
      because they run on Ollama&apos;s servers.
    </>
  );

  return (
    <PanelGrid>
      <Panel title="Speech recognition">
        <TextField
          label="Speech model file"
          value={settings.models.whisperModelPath}
          placeholder="C:\models\ggml-base.en.bin"
          spellCheck={false}
          onCommit={(whisperModelPath) =>
            updateSettings((current) => ({
              ...current,
              models: { ...current.models, whisperModelPath },
            }))
          }
          hint="Leave this empty to use the installed default: Parakeet when it is installed, otherwise Whisper tiny.en. A .gguf model runs inside LocalFlow; a ggml Whisper .bin model runs in the whisper.cpp program."
        />
      </Panel>

      <Panel title="Language model">
        {options.length > 0 ? (
          <SelectField
            label="Ollama model"
            value={selected}
            options={options}
            onChange={setOllamaModel}
            hint={modelHint}
          />
        ) : (
          <TextField
            label="Ollama model"
            value={selected}
            placeholder={pinnedOllamaModel}
            spellCheck={false}
            onCommit={setOllamaModel}
            hint={modelHint}
          />
        )}
        <div className="button-row button-row--status">
          <button type="button" onClick={() => void checkOllama()} disabled={isChecking}>
            <RefreshCw size={16} aria-hidden="true" />
            {isChecking ? "Checking" : "List installed models"}
          </button>
          {ollamaStatus ? (
            <StatusPill tone={ollamaStatus.ok ? "ok" : "error"}>
              {ollamaStatus.ok ? "Ollama is running" : "Ollama not reached"}
            </StatusPill>
          ) : null}
        </div>
        <LiveMessage
          message={ollamaStatus ? formatOllamaStatus(ollamaStatus) : null}
          tone={ollamaStatus && !ollamaStatus.ok ? "alert" : "status"}
        />
        <Toggle
          label="Low-resource mode"
          checked={settings.models.lowResourceMode}
          onChange={(lowResourceMode) =>
            updateSettings((current) => ({
              ...current,
              models: { ...current.models, lowResourceMode },
            }))
          }
          hint="Skips the background clean-up after dictation and, when Whisper tiny.en is installed, uses it instead of keeping Parakeet loaded. Conversations then recognise speech more slowly and less accurately."
        />
      </Panel>
    </PanelGrid>
  );
}
