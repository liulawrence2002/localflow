import { ListEditor } from "../components/ui/ListEditor";
import {
  addDictionaryEntry,
  addReplacementRule,
  addSnippet,
  removeDictionaryEntry,
  removeReplacementRule,
  removeSnippet,
  updateDictionaryEntry,
  updateReplacementRule,
  updateSnippet,
} from "../domain/settings";
import type { DictionaryEntry, LocalFlowSettings, ReplacementRule, Snippet } from "../domain/types";
import type { UpdateSettings } from "./types";

interface ListScreenProps {
  settings: LocalFlowSettings;
  updateSettings: UpdateSettings;
}

/** Only the phrase is edited: hints, categories and case matching are not used natively yet. */
export function DictionaryScreen({ settings, updateSettings }: ListScreenProps) {
  return (
    <ListEditor<DictionaryEntry>
      title="Dictionary"
      description="Words and names the speech recogniser should expect in dictation, such as product names or people's names. Each is given to the recogniser as a hint."
      itemNoun="dictionary word"
      columns={[
        {
          kind: "text",
          key: "phrase",
          label: "Word or phrase",
          requiredMessage: "Enter a word or phrase.",
          placeholder: "PyTorch",
        },
      ]}
      items={settings.dictionary}
      emptyText="No words yet. Add names and terms the recogniser gets wrong."
      createDraft={() => ({
        phrase: "",
        pronunciationHint: undefined,
        category: "custom",
        caseSensitive: false,
      })}
      nameOf={(entry) => entry.phrase}
      onAdd={(draft) => updateSettings((current) => addDictionaryEntry(current, draft))}
      onUpdate={(id, patch) =>
        updateSettings((current) => updateDictionaryEntry(current, id, patch))
      }
      onRemove={(id) => updateSettings((current) => removeDictionaryEntry(current, id))}
    />
  );
}

export function ReplacementsScreen({ settings, updateSettings }: ListScreenProps) {
  return (
    <ListEditor<ReplacementRule>
      title="Replacements"
      description="When dictation hears the first phrase as a whole word or words, in any capitalisation, it writes the second instead."
      itemNoun="replacement"
      columns={[
        {
          kind: "text",
          key: "incorrect",
          label: "Heard",
          requiredMessage: "Enter what dictation hears.",
          placeholder: "pie torch",
        },
        {
          kind: "text",
          key: "correct",
          label: "Write",
          requiredMessage: "Enter what to write instead.",
          placeholder: "PyTorch",
        },
        { kind: "toggle", key: "enabled", label: "On" },
      ]}
      items={settings.replacements}
      emptyText="No replacements yet."
      createDraft={() => ({ incorrect: "", correct: "", enabled: true })}
      nameOf={(rule) => rule.incorrect}
      onAdd={(draft) => updateSettings((current) => addReplacementRule(current, draft))}
      onUpdate={(id, patch) =>
        updateSettings((current) => updateReplacementRule(current, id, patch))
      }
      onRemove={(id) => updateSettings((current) => removeReplacementRule(current, id))}
    />
  );
}

/** The clean-up switch is not shown: snippets are applied natively without it. */
export function SnippetsScreen({ settings, updateSettings }: ListScreenProps) {
  return (
    <ListEditor<Snippet>
      title="Snippets"
      description="Say the trigger phrase while dictating and the expansion is written in its place."
      itemNoun="snippet"
      columns={[
        {
          kind: "text",
          key: "trigger",
          label: "Trigger",
          requiredMessage: "Enter the phrase you will say.",
          placeholder: "insert my signature",
        },
        {
          kind: "multiline",
          key: "expansion",
          label: "Expansion",
          requiredMessage: "Enter the text to write.",
        },
        { kind: "toggle", key: "enabled", label: "On" },
      ]}
      items={settings.snippets}
      emptyText="No snippets yet."
      createDraft={() => ({ trigger: "", expansion: "", enabled: true, allowCleanup: false })}
      nameOf={(snippet) => snippet.trigger}
      onAdd={(draft) => updateSettings((current) => addSnippet(current, draft))}
      onUpdate={(id, patch) => updateSettings((current) => updateSnippet(current, id, patch))}
      onRemove={(id) => updateSettings((current) => removeSnippet(current, id))}
    />
  );
}
