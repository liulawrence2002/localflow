import { parseRefinementResponse, type RefinementResponse } from "./refinement";

export type CommandIntent =
  | "make_concise"
  | "bullet_points"
  | "fix_grammar"
  | "professional"
  | "translate"
  | "friendly_message"
  | "custom";

export interface CommandModeInput {
  selectedText: string;
  instruction: string;
  appCategory: string;
  previewThresholdChars: number;
}

export interface CommandModePlan {
  ok: true;
  selectedText: string;
  instruction: string;
  intent: CommandIntent;
  previewRequired: boolean;
  prompt: string;
}

export interface CommandModeRejection {
  ok: false;
  reason: "selected_text_required" | "instruction_required" | "operating_system_command_rejected";
}

export interface CommandModeResult {
  text: string;
  previewRequired: boolean;
  undoText: string;
  confidence: number;
  warnings: string[];
}

export function planCommandMode(input: CommandModeInput): CommandModePlan | CommandModeRejection {
  const selectedText = input.selectedText.trim();
  const instruction = input.instruction.trim();

  if (!selectedText) {
    return { ok: false, reason: "selected_text_required" };
  }
  if (!instruction) {
    return { ok: false, reason: "instruction_required" };
  }
  if (looksLikeOperatingSystemCommand(instruction)) {
    return { ok: false, reason: "operating_system_command_rejected" };
  }

  const intent = classifyCommandIntent(instruction);

  return {
    ok: true,
    selectedText,
    instruction,
    intent,
    previewRequired: selectedText.length >= input.previewThresholdChars,
    prompt: buildCommandPrompt({
      selectedText,
      instruction,
      intent,
      appCategory: input.appCategory,
    }),
  };
}

export function parseCommandModeResponse(
  payload: string,
  plan: CommandModePlan,
): CommandModeResult | CommandModeRejection {
  const parsed = parseRefinementResponse(payload);
  if (!parsed.ok) {
    return { ok: false, reason: "instruction_required" };
  }

  return toCommandModeResult(parsed.response, plan);
}

export function classifyCommandIntent(instruction: string): CommandIntent {
  const normalized = instruction.toLowerCase();

  if (/\b(concise|shorter|trim)\b/u.test(normalized)) {
    return "make_concise";
  }
  if (/\b(bullet|bullets|list)\b/u.test(normalized)) {
    return "bullet_points";
  }
  if (/\b(grammar|typo|proofread)\b/u.test(normalized)) {
    return "fix_grammar";
  }
  if (/\b(professional|formal)\b/u.test(normalized)) {
    return "professional";
  }
  if (/\btranslate\b/u.test(normalized)) {
    return "translate";
  }
  if (/\b(friendly|slack|message)\b/u.test(normalized)) {
    return "friendly_message";
  }

  return "custom";
}

function buildCommandPrompt({
  selectedText,
  instruction,
  intent,
  appCategory,
}: {
  selectedText: string;
  instruction: string;
  intent: CommandIntent;
  appCategory: string;
}): string {
  return JSON.stringify({
    task: "localflow.command_mode",
    contract:
      "Transform only the selected text according to the instruction. Return strict JSON with text, confidence, resolved_corrections, and warnings. Do not execute OS commands.",
    selectedText,
    instruction,
    intent,
    appCategory,
  });
}

function looksLikeOperatingSystemCommand(instruction: string): boolean {
  return /\b(powershell|cmd\.exe|bash|delete file|remove file|run command|execute|shutdown|format disk|registry)\b/iu.test(
    instruction,
  );
}

function toCommandModeResult(
  response: RefinementResponse,
  plan: CommandModePlan,
): CommandModeResult {
  return {
    text: response.text,
    previewRequired: plan.previewRequired,
    undoText: plan.selectedText,
    confidence: response.confidence,
    warnings: response.warnings,
  };
}
