export interface CursorContext {
  beforeCursor: string;
  afterCursor: string;
  atSentenceStart: boolean;
  codeMode: boolean;
}

export function composeInsertion(text: string, context: CursorContext): string {
  if (context.codeMode) {
    return text;
  }

  let result = text.trim();
  const before = context.beforeCursor;
  const after = context.afterCursor;

  if (!context.atSentenceStart && shouldLowercaseFirstWord(before, result)) {
    result = result.replace(/^([A-Z])/, (match) => match.toLowerCase());
  }

  if (needsLeadingSpace(before, result)) {
    result = ` ${result}`;
  }

  const lastInsertionCharacter = result.charAt(result.length - 1);
  if (after.startsWith(lastInsertionCharacter) && /[.,;:!?]/.test(lastInsertionCharacter)) {
    result = result.slice(0, -1);
  }

  return result;
}

function needsLeadingSpace(beforeCursor: string, insertion: string): boolean {
  if (!beforeCursor || !insertion) {
    return false;
  }

  const previous = beforeCursor.charAt(beforeCursor.length - 1);
  const first = insertion.charAt(0);

  if (/\s/.test(previous) || /[.,;:!?\n)]/.test(first)) {
    return false;
  }

  return /[\p{L}\p{N})"']$/u.test(beforeCursor);
}

function shouldLowercaseFirstWord(beforeCursor: string, insertion: string): boolean {
  return /[\p{L}\p{N},"']\s*$/u.test(beforeCursor) && /^[A-Z][a-z]/.test(insertion);
}
