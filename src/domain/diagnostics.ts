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

/** What a Windows user folder is replaced with in an export. */
export const userFolderPlaceholder = "<user folder>";
/** What a microphone's name is replaced with in an export. */
export const microphonePlaceholder = "<microphone>";

/**
 * `C:\Users\<name>` (either slash), the part of a path that names the person. A name may
 * contain spaces, so everything up to the next slash is taken: removing too much is safer
 * than leaving a surname behind.
 */
const userFolderPattern = /\b[A-Za-z]:[\\/]+Users[\\/]+[^\\/"'<>|:*?\r\n]+/gi;
/** The "Last recording start" row: `<device> at <time> (<rate> Hz, <n> ch)`. */
const recordingStartPattern = /^(?<device>.+?) at (?<rest>\S+ \(\d+ Hz, \d+ ch\))$/;
/** Native messages that quote a device name: `... from "<device>" ...`. */
const quotedDevicePattern = /(from |device )"[^"]*"/gi;

/**
 * Microphone names that appear in the diagnostics, so they can be removed from an export.
 * Current native builds leave the name out; older ones put it in "Last recording start".
 */
export function findMicrophoneNames(status: AppStatus): string[] {
  const names = new Set<string>();
  const configured = status.settings.microphone.selectedDeviceName.trim();
  if (configured && configured.toLowerCase() !== "default microphone") {
    names.add(configured);
  }
  for (const metric of status.diagnostics) {
    const device = recordingStartPattern.exec(metric.value)?.groups?.device?.trim();
    if (device) {
      names.add(device);
    }
  }
  return [...names].sort((a, b) => b.length - a.length);
}

/**
 * Remove what identifies the person or their hardware from a line of diagnostics: the user
 * folder in any path, and microphone names.
 */
export function redactLocalDetails(text: string, microphoneNames: string[] = []): string {
  let redacted = text.replace(userFolderPattern, userFolderPlaceholder);
  for (const name of microphoneNames) {
    if (name) {
      redacted = redacted.split(name).join(microphonePlaceholder);
    }
  }
  const recordingStart = recordingStartPattern.exec(redacted);
  if (recordingStart?.groups?.rest && recordingStart.groups.device !== microphonePlaceholder) {
    redacted = `${microphonePlaceholder} at ${recordingStart.groups.rest}`;
  }
  return redacted.replace(quotedDevicePattern, `$1"${microphonePlaceholder}"`);
}

export function buildDiagnosticsExport(
  status: AppStatus,
  options: DiagnosticsExportOptions = {},
): DiagnosticsExport {
  const activeSession = status.workflow.activeSession;
  const microphoneNames = findMicrophoneNames(status);

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
    diagnostics: status.diagnostics.map((metric) => ({
      label: redactLocalDetails(metric.label, microphoneNames),
      value: redactLocalDetails(metric.value, microphoneNames),
      status: metric.status,
    })),
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
