export type DictationPhase =
  | "Idle"
  | "Preparing"
  | "Listening"
  | "Transcribing"
  | "Refining"
  | "Inserting"
  | "Complete"
  | "Cancelled"
  | "Error";

export type ActivationMode = "push_to_talk" | "toggle";

export type CleanupLevel = "verbatim" | "light" | "balanced" | "strong";

export type AppCategory =
  | "personal_messaging"
  | "work_messaging"
  | "email"
  | "document"
  | "code_editor"
  | "terminal"
  | "search_field"
  | "generic_text_field";

export interface TargetSnapshot {
  applicationName: string;
  windowTitle: string;
  category: AppCategory;
  protectedField: boolean;
}

export interface DictationSession {
  id: string;
  mode: ActivationMode;
  startedAt: string;
  target: TargetSnapshot;
  rawTranscript?: string;
  deterministicText?: string;
  refinedText?: string;
  insertedText?: string;
  confidence?: number;
}

export interface SessionHistoryItem {
  id: string;
  completedAt: string;
  targetApplication: string;
  rawTranscript: string;
  finalText: string;
  cleanupLevel: CleanupLevel;
}

export interface WorkflowState {
  phase: DictationPhase;
  activeSession?: DictationSession;
  lastCompleted?: SessionHistoryItem;
  warning?: string;
  error?: string;
}

export interface DictionaryEntry {
  id: string;
  phrase: string;
  pronunciationHint?: string;
  category: "name" | "acronym" | "technical" | "custom";
  caseSensitive: boolean;
}

export interface ReplacementRule {
  id: string;
  incorrect: string;
  correct: string;
  enabled: boolean;
}

export interface Snippet {
  id: string;
  trigger: string;
  expansion: string;
  enabled: boolean;
  allowCleanup: boolean;
}

export interface StyleProfile {
  id: string;
  name: string;
  category: AppCategory;
  cleanupLevel: CleanupLevel;
  conciseness: number;
  formality: number;
  contractions: boolean;
  emoji: "never" | "preserve" | "sparingly";
  paragraphLength: "short" | "medium" | "long";
  bulletPreference: "preserve" | "prefer" | "avoid";
  greetingBehavior: "preserve" | "add_when_missing" | "avoid";
  signOffBehavior: "preserve" | "add_when_missing" | "avoid";
  aggressiveFillerRemoval: boolean;
  allowSentenceFragments: boolean;
}

export interface PrivacySettings {
  historyRetention: "off" | "transcript_only" | "original_and_cleaned";
  deleteAfter: "never" | "24h" | "7d";
  activeAppDetection: boolean;
  accessibilityContext: boolean;
  selectedTextTransforms: boolean;
  contextRetention: boolean;
  deleteAudioAfterProcessing: boolean;
}

export interface HotkeySettings {
  defaultHotkey: string;
  activationMode: ActivationMode;
  commandHotkey: string;
}

export interface ModelSettings {
  whisperModelPath: string;
  language: string;
  asrThreads: number;
  ollamaModel: string;
  lowResourceMode: boolean;
}

export interface MicrophoneSettings {
  selectedDeviceId: string;
  selectedDeviceName: string;
  vadEnabled: boolean;
  endOfSpeechMs: number;
  maxRecordingSeconds: number;
}

export interface LocalFlowSettings {
  hotkeys: HotkeySettings;
  models: ModelSettings;
  microphone: MicrophoneSettings;
  privacy: PrivacySettings;
  dictionary: DictionaryEntry[];
  replacements: ReplacementRule[];
  snippets: Snippet[];
  styles: StyleProfile[];
}

export interface AppStatus {
  workflow: WorkflowState;
  settings: LocalFlowSettings;
  history: SessionHistoryItem[];
  diagnostics: DiagnosticMetric[];
}

export interface DiagnosticMetric {
  label: string;
  value: string;
  status: "ok" | "warning" | "blocked";
}
