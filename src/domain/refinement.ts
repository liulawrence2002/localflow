export interface RefinementResponse {
  text: string;
  confidence: number;
  resolved_corrections: string[];
  warnings: string[];
}

export interface ParsedRefinement {
  ok: true;
  response: RefinementResponse;
}

export interface FailedRefinement {
  ok: false;
  error: string;
}

export function parseRefinementResponse(payload: string): ParsedRefinement | FailedRefinement {
  try {
    const parsed: unknown = JSON.parse(payload);
    if (!isRecord(parsed)) {
      return { ok: false, error: "Refinement response was not an object." };
    }

    if (typeof parsed.text !== "string") {
      return { ok: false, error: "Refinement response is missing text." };
    }

    if (typeof parsed.confidence !== "number" || parsed.confidence < 0 || parsed.confidence > 1) {
      return { ok: false, error: "Refinement confidence must be between 0 and 1." };
    }

    if (!isStringArray(parsed.resolved_corrections) || !isStringArray(parsed.warnings)) {
      return {
        ok: false,
        error: "Refinement corrections and warnings must be string arrays.",
      };
    }

    return {
      ok: true,
      response: {
        text: parsed.text,
        confidence: parsed.confidence,
        resolved_corrections: parsed.resolved_corrections,
        warnings: parsed.warnings,
      },
    };
  } catch (error) {
    return {
      ok: false,
      error: error instanceof Error ? error.message : "Invalid refinement JSON.",
    };
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}
