import type { AppCategory, CleanupLevel, DictionaryEntry, ReplacementRule } from "./types";
import { parseRefinementResponse, type RefinementResponse } from "./refinement";

export interface DictationCleanupInput {
  rawTranscript: string;
  deterministicText: string;
  appCategory: AppCategory;
  beforeCursor: string;
  afterCursor: string;
  cleanupLevel: CleanupLevel;
  codeMode: boolean;
  dictionary: DictionaryEntry[];
  replacements: ReplacementRule[];
  styleName: string;
}

export interface LocalRefinementProvider {
  complete(prompt: string): Promise<string>;
}

export interface CleanupResult {
  text: string;
  rawTranscript: string;
  deterministicText: string;
  confidence: number;
  source: "model" | "repaired_model" | "deterministic_fallback";
  resolvedCorrections: string[];
  warnings: string[];
}

export async function runLocalCleanup(
  provider: LocalRefinementProvider,
  input: DictationCleanupInput,
): Promise<CleanupResult> {
  const firstPayload = await provider.complete(buildCleanupPrompt(input));
  const first = parseRefinementResponse(firstPayload);

  if (first.ok) {
    return toCleanupResult(first.response, input, "model");
  }

  const repairPayload = await provider.complete(buildRepairPrompt(firstPayload, first.error));
  const repaired = parseRefinementResponse(repairPayload);

  if (repaired.ok) {
    return toCleanupResult(repaired.response, input, "repaired_model");
  }

  return {
    text: input.deterministicText,
    rawTranscript: input.rawTranscript,
    deterministicText: input.deterministicText,
    confidence: 0,
    source: "deterministic_fallback",
    resolvedCorrections: [],
    warnings: [
      "Local cleanup model returned invalid JSON twice; deterministic transcript was used.",
      first.error,
      repaired.error,
    ],
  };
}

export function buildCleanupPrompt(input: DictationCleanupInput): string {
  return JSON.stringify({
    task: "localflow.dictation_cleanup",
    contract: "Return only strict JSON with text, confidence, resolved_corrections, and warnings.",
    rawTranscript: input.rawTranscript,
    deterministicText: input.deterministicText,
    appCategory: input.appCategory,
    beforeCursor: input.beforeCursor,
    afterCursor: input.afterCursor,
    cleanupLevel: input.cleanupLevel,
    codeMode: input.codeMode,
    dictionary: input.dictionary.map((entry) => entry.phrase),
    replacements: input.replacements
      .filter((replacement) => replacement.enabled)
      .map((replacement) => ({
        incorrect: replacement.incorrect,
        correct: replacement.correct,
      })),
    styleName: input.styleName,
  });
}

function buildRepairPrompt(payload: string, error: string): string {
  return JSON.stringify({
    task: "localflow.repair_cleanup_json",
    error,
    invalidPayload: payload,
    instruction:
      "Convert the invalid payload to strict JSON only. Do not change the intended cleaned text.",
  });
}

function toCleanupResult(
  response: RefinementResponse,
  input: DictationCleanupInput,
  source: CleanupResult["source"],
): CleanupResult {
  return {
    text: response.text,
    rawTranscript: input.rawTranscript,
    deterministicText: input.deterministicText,
    confidence: response.confidence,
    source,
    resolvedCorrections: response.resolved_corrections,
    warnings: response.warnings,
  };
}
