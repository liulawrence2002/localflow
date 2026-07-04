import type { AppStatus, LocalFlowSettings, WorkflowState } from "./types";

export const defaultTarget = {
  applicationName: "Mock target",
  windowTitle: "LocalFlow verification field",
  category: "generic_text_field" as const,
  protectedField: false,
};

export const idleWorkflow: WorkflowState = {
  phase: "Idle",
};

export const defaultSettings: LocalFlowSettings = {
  hotkeys: {
    defaultHotkey: "Ctrl+Alt+Space",
    activationMode: "push_to_talk",
    commandHotkey: "Ctrl+Alt+Shift+Space",
  },
  models: {
    whisperModelPath: "",
    language: "auto",
    asrThreads: 4,
    ollamaModel: "",
    lowResourceMode: false,
  },
  microphone: {
    selectedDeviceId: "default",
    selectedDeviceName: "Default microphone",
    vadEnabled: true,
    endOfSpeechMs: 900,
    maxRecordingSeconds: 120,
  },
  privacy: {
    historyRetention: "original_and_cleaned",
    deleteAfter: "7d",
    activeAppDetection: true,
    accessibilityContext: false,
    selectedTextTransforms: false,
    contextRetention: false,
    deleteAudioAfterProcessing: true,
  },
  dictionary: [
    {
      id: "dict-pytorch",
      phrase: "PyTorch",
      pronunciationHint: "pie torch",
      category: "technical",
      caseSensitive: false,
    },
    {
      id: "dict-localflow",
      phrase: "LocalFlow",
      category: "custom",
      caseSensitive: false,
    },
  ],
  replacements: [
    {
      id: "replace-pytorch",
      incorrect: "pie torch",
      correct: "PyTorch",
      enabled: true,
    },
  ],
  snippets: [
    {
      id: "snippet-signature",
      trigger: "insert my signature",
      expansion: "Best,\nLocalFlow",
      enabled: true,
      allowCleanup: false,
    },
  ],
  styles: [
    {
      id: "style-work",
      name: "Work messages",
      category: "work_messaging",
      cleanupLevel: "balanced",
      conciseness: 6,
      formality: 6,
      contractions: true,
      emoji: "preserve",
      paragraphLength: "short",
      bulletPreference: "preserve",
      greetingBehavior: "preserve",
      signOffBehavior: "preserve",
      aggressiveFillerRemoval: false,
      allowSentenceFragments: true,
    },
    {
      id: "style-code",
      name: "Code and technical prompts",
      category: "code_editor",
      cleanupLevel: "light",
      conciseness: 4,
      formality: 4,
      contractions: true,
      emoji: "never",
      paragraphLength: "medium",
      bulletPreference: "preserve",
      greetingBehavior: "avoid",
      signOffBehavior: "avoid",
      aggressiveFillerRemoval: false,
      allowSentenceFragments: true,
    },
  ],
};

export const defaultStatus: AppStatus = {
  workflow: idleWorkflow,
  settings: defaultSettings,
  history: [],
  diagnostics: [
    { label: "Rust toolchain", value: "Not detected in current PATH", status: "blocked" },
    { label: "ASR provider", value: "Mock provider active", status: "warning" },
    { label: "Refinement provider", value: "Mock local provider active", status: "warning" },
    { label: "Network policy", value: "No remote dictation calls configured", status: "ok" },
  ],
};
