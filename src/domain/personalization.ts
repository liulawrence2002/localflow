import type { ReplacementRule, Snippet } from "./types";

const spokenPunctuation = new Map<string, string>([
  ["comma", ","],
  ["period", "."],
  ["full stop", "."],
  ["question mark", "?"],
  ["colon", ":"],
  ["semicolon", ";"],
  ["new line", "\n"],
  ["new paragraph", "\n\n"],
  ["open parenthesis", "("],
  ["close parenthesis", ")"],
  ["quote", '"'],
  ["bullet point", "\n-"],
]);

export interface PersonalizationResult {
  text: string;
  appliedReplacements: string[];
  expandedSnippets: string[];
}

export function runDeterministicPersonalization(
  rawText: string,
  replacements: ReplacementRule[],
  snippets: Snippet[],
): PersonalizationResult {
  const correctedText = resolveExplicitSelfCorrections(rawText);
  const snippetResult = applySnippets(correctedText, snippets);
  const replacementResult = applyReplacements(snippetResult.text, replacements);
  const punctuated = applySpokenPunctuation(replacementResult.text);

  return {
    text: normalizeWhitespaceAroundPunctuation(punctuated),
    appliedReplacements: replacementResult.applied,
    expandedSnippets: snippetResult.expanded,
  };
}

export function applyReplacements(
  text: string,
  replacements: ReplacementRule[],
): { text: string; applied: string[] } {
  let result = text;
  const applied: string[] = [];

  for (const replacement of replacements) {
    if (!replacement.enabled || !replacement.incorrect.trim()) {
      continue;
    }

    const pattern = tokenBoundaryPattern(replacement.incorrect);
    if (pattern.test(result)) {
      result = result.replace(pattern, replacement.correct);
      applied.push(replacement.id);
    }
  }

  return { text: result, applied };
}

export function applySnippets(
  text: string,
  snippets: Snippet[],
): { text: string; expanded: string[] } {
  let result = text;
  const expanded: string[] = [];

  for (const snippet of snippets) {
    if (!snippet.enabled || !snippet.trigger.trim()) {
      continue;
    }

    const pattern = tokenBoundaryPattern(snippet.trigger);
    if (pattern.test(result)) {
      result = result.replace(pattern, snippet.expansion);
      expanded.push(snippet.id);
    }
  }

  return { text: result, expanded };
}

export function applySpokenPunctuation(text: string): string {
  const escapedCommands = [...spokenPunctuation.keys()]
    .sort((a, b) => b.length - a.length)
    .map(escapeRegExp)
    .join("|");
  const pattern = new RegExp(`(^|\\s)(${escapedCommands})(?=\\s|$)`, "giu");

  return text.replace(pattern, (match, prefix: string, command: string) => {
    const punctuation = spokenPunctuation.get(command.toLowerCase());
    if (!punctuation) {
      return match;
    }
    if (punctuation.startsWith("\n")) {
      return punctuation;
    }
    return punctuation === "(" ? `${prefix}${punctuation}` : punctuation;
  });
}

export function normalizeWhitespaceAroundPunctuation(text: string): string {
  return text
    .replace(/\n{3,}-/g, "\n\n-")
    .replace(/\s+([,.;:?])/g, "$1")
    .replace(/([,.;:?])(?=\S)/g, "$1 ")
    .replace(/\(\s+/g, "(")
    .replace(/\s+\)/g, ")")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n[ \t]+/g, "\n")
    .replace(/[ \t]{2,}/g, " ")
    .trim();
}

export function resolveExplicitSelfCorrections(text: string): string {
  const normalized = text.trim().replace(/\s+/g, " ");
  if (!normalized) {
    return normalized;
  }

  const restartMatch = normalized.match(/\blet me restart\b[,.]?\s*(?<restart>.+)$/iu);
  if (restartMatch?.groups?.restart) {
    return restartMatch.groups.restart.trim();
  }

  const actuallyMatch = normalized.match(/^(?<before>.+?)\s+actually\s+(?<after>.+)$/iu);
  if (actuallyMatch?.groups && wordCount(actuallyMatch.groups.before) >= 3) {
    return actuallyMatch.groups.after.trim();
  }

  const sorryMatch = normalized.match(/^(?<before>.+?)\s+sorry\s+(?<after>.+)$/iu);
  if (sorryMatch?.groups) {
    return replaceLastToken(sorryMatch.groups.before, sorryMatch.groups.after);
  }

  const noMatch = normalized.match(
    /^(?<before>(?:\S+\s+){2,}\S+)\s+no\s+(?<after>\S+(?:\s+\S+){0,2})$/iu,
  );
  if (noMatch?.groups) {
    return replaceLastToken(noMatch.groups.before, noMatch.groups.after);
  }

  return normalized;
}

function replaceLastToken(before: string, after: string): string {
  const prefix = before
    .trim()
    .replace(/\s+\S+$/u, "")
    .trim();
  return `${prefix} ${after.trim()}`.trim();
}

function wordCount(text: string): number {
  return text.trim().split(/\s+/).filter(Boolean).length;
}

function tokenBoundaryPattern(phrase: string): RegExp {
  return new RegExp(`(?<![\\p{L}\\p{N}_])${escapeRegExp(phrase)}(?![\\p{L}\\p{N}_])`, "giu");
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
