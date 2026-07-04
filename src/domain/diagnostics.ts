import { redactSensitiveText } from "./privacy";
import type { AppStatus, SessionHistoryItem, WorkflowState } from "./types";

export interface DiagnosticsExportOptions {
  generatedAt?: string;
  includeTranscriptText?: boolean;
}

export interface DiagnosticsExport {
  application: "LocalFlow";
  generatedAt: string;
  workflow: {
    phase: WorkflowState["phase"];
    activeSessionId?: string;
    targetApplication?: string;
    targetWindowTitle?: string;
  };
  settings: {
    activationMode: string;
    language: string;
    asrThreads: number;
    whisperModelConfigured: boolean;
    ollamaModelConfigured: boolean;
    lowResourceMode: boolean;
    privacy: AppStatus["settings"]["privacy"];
    dictionaryEntries: number;
    replacementRules: number;
    snippets: number;
    styleProfiles: number;
  };
  diagnostics: AppStatus["diagnostics"];
  history: Array<{
    id: string;
    completedAt: string;
    targetApplication: string;
    cleanupLevel: string;
    rawTranscript: string;
    deterministicText?: string;
    refinedText?: string;
    finalText: string;
  }>;
}

export function buildDiagnosticsExport(
  status: AppStatus,
  options: DiagnosticsExportOptions = {},
): DiagnosticsExport {
  const activeSession = status.workflow.activeSession;

  return {
    application: "LocalFlow",
    generatedAt: options.generatedAt ?? new Date().toISOString(),
    workflow: {
      phase: status.workflow.phase,
      activeSessionId: activeSession?.id,
      targetApplication: activeSession?.target.applicationName,
      targetWindowTitle: activeSession?.target.windowTitle
        ? redactSensitiveText(activeSession.target.windowTitle)
        : undefined,
    },
    settings: {
      activationMode: status.settings.hotkeys.activationMode,
      language: status.settings.models.language,
      asrThreads: status.settings.models.asrThreads,
      whisperModelConfigured: status.settings.models.whisperModelPath.trim().length > 0,
      ollamaModelConfigured: status.settings.models.ollamaModel.trim().length > 0,
      lowResourceMode: status.settings.models.lowResourceMode,
      privacy: status.settings.privacy,
      dictionaryEntries: status.settings.dictionary.length,
      replacementRules: status.settings.replacements.length,
      snippets: status.settings.snippets.length,
      styleProfiles: status.settings.styles.length,
    },
    diagnostics: status.diagnostics,
    history: status.history.map((item) => sanitizeHistoryItem(item, options.includeTranscriptText)),
  };
}

export function serializeDiagnosticsExport(
  status: AppStatus,
  options: DiagnosticsExportOptions = {},
): string {
  return JSON.stringify(buildDiagnosticsExport(status, options), null, 2);
}

function sanitizeHistoryItem(
  item: SessionHistoryItem,
  includeTranscriptText: boolean | undefined,
): DiagnosticsExport["history"][number] {
  if (includeTranscriptText) {
    return {
      id: item.id,
      completedAt: item.completedAt,
      targetApplication: item.targetApplication,
      cleanupLevel: item.cleanupLevel,
      rawTranscript: item.rawTranscript,
      deterministicText: item.deterministicText,
      refinedText: item.refinedText,
      finalText: item.finalText,
    };
  }

  return {
    id: item.id,
    completedAt: item.completedAt,
    targetApplication: item.targetApplication,
    cleanupLevel: item.cleanupLevel,
    rawTranscript: redactSensitiveText(item.rawTranscript),
    deterministicText: item.deterministicText
      ? redactSensitiveText(item.deterministicText)
      : undefined,
    refinedText: item.refinedText ? redactSensitiveText(item.refinedText) : undefined,
    finalText: redactSensitiveText(item.finalText),
  };
}
