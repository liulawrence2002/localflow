export interface StabilizerUpdate {
  committedText: string;
  newCommit: string;
  provisionalText: string;
}

export class TranscriptStabilizer {
  private committedTokens: string[] = [];
  private previousTokens: string[] = [];

  update(hypothesis: string): StabilizerUpdate {
    const currentTokens = tokenize(hypothesis);
    const stablePrefix = longestCommonPrefix(this.previousTokens, currentTokens);
    const newTokens = stablePrefix.slice(this.committedTokens.length);

    if (newTokens.length > 0) {
      this.committedTokens = stablePrefix;
    }

    this.previousTokens = currentTokens;

    return {
      committedText: detokenize(this.committedTokens),
      newCommit: detokenize(newTokens),
      provisionalText: detokenize(currentTokens.slice(this.committedTokens.length)),
    };
  }

  finalize(hypothesis: string): StabilizerUpdate {
    const currentTokens = tokenize(hypothesis);
    const newTokens = currentTokens.slice(this.committedTokens.length);
    this.committedTokens = currentTokens;
    this.previousTokens = currentTokens;

    return {
      committedText: detokenize(this.committedTokens),
      newCommit: detokenize(newTokens),
      provisionalText: "",
    };
  }

  reset(): void {
    this.committedTokens = [];
    this.previousTokens = [];
  }
}

function tokenize(text: string): string[] {
  return text.trim().length === 0 ? [] : text.trim().split(/\s+/);
}

function detokenize(tokens: string[]): string {
  return tokens.join(" ");
}

function longestCommonPrefix(left: string[], right: string[]): string[] {
  const length = Math.min(left.length, right.length);
  let index = 0;

  while (index < length && left[index].toLowerCase() === right[index].toLowerCase()) {
    index += 1;
  }

  return right.slice(0, index);
}
