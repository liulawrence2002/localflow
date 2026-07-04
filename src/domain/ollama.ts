import { evaluateDictationNetworkUrl } from "./networkPolicy";
import { runWithTimeout } from "./timeout";
import type { LocalRefinementProvider } from "./refinementPipeline";

export const defaultOllamaApiBaseUrl = "http://127.0.0.1:11434/api";

export type OllamaErrorCode =
  | "invalid_url"
  | "remote_network_blocked"
  | "unavailable"
  | "http_error"
  | "invalid_response"
  | "model_not_found"
  | "model_not_selected";

export class OllamaProviderError extends Error {
  readonly code: OllamaErrorCode;
  readonly status?: number;

  constructor(code: OllamaErrorCode, message: string, status?: number) {
    super(message);
    this.name = "OllamaProviderError";
    this.code = code;
    this.status = status;
  }
}

export interface OllamaModelDetails {
  format?: string;
  family?: string;
  families?: string[];
  parameterSize?: string;
  quantizationLevel?: string;
}

export interface OllamaModelSummary {
  name: string;
  model: string;
  modifiedAt?: string;
  sizeBytes?: number;
  digest?: string;
  details?: OllamaModelDetails;
}

export type OllamaConnectionStatus =
  | {
      ok: true;
      baseUrl: string;
      models: OllamaModelSummary[];
    }
  | {
      ok: false;
      baseUrl: string;
      code: OllamaErrorCode;
      message: string;
    };

export interface OllamaProviderOptions {
  baseUrl?: string;
  fetchImpl?: FetchLike;
  timeoutMs?: number;
}

export interface OllamaRefinementProviderOptions extends OllamaProviderOptions {
  model: string;
  keepAlive?: string | number;
}

type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

export class OllamaRefinementProvider implements LocalRefinementProvider {
  private readonly baseUrl: string;
  private readonly fetchImpl: FetchLike;
  private readonly keepAlive: string | number;
  private readonly model: string;
  private readonly timeoutMs: number;

  constructor(options: OllamaRefinementProviderOptions) {
    this.baseUrl = options.baseUrl ?? defaultOllamaApiBaseUrl;
    this.fetchImpl = options.fetchImpl ?? resolveGlobalFetch();
    this.keepAlive = options.keepAlive ?? "5m";
    this.model = options.model.trim();
    this.timeoutMs = options.timeoutMs ?? 60_000;
  }

  async complete(prompt: string): Promise<string> {
    if (!this.model) {
      throw new OllamaProviderError(
        "model_not_selected",
        "Select a local Ollama model before enabling cleanup.",
      );
    }

    const endpoint = resolveOllamaEndpoint(this.baseUrl, "generate");
    assertLocalDictationEndpoint(endpoint);

    const payload = await requestOllamaJson(
      this.fetchImpl,
      endpoint,
      {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          model: this.model,
          prompt,
          stream: false,
          format: "json",
          keep_alive: this.keepAlive,
          options: {
            temperature: 0.1,
          },
        }),
      },
      this.timeoutMs,
      "Ollama cleanup",
      this.model,
    );

    return parseGenerateResponse(payload);
  }
}

export async function discoverOllamaModels(
  options: OllamaProviderOptions = {},
): Promise<OllamaModelSummary[]> {
  const baseUrl = options.baseUrl ?? defaultOllamaApiBaseUrl;
  const fetchImpl = options.fetchImpl ?? resolveGlobalFetch();
  const endpoint = resolveOllamaEndpoint(baseUrl, "tags");
  assertLocalDictationEndpoint(endpoint);

  const payload = await requestOllamaJson(
    fetchImpl,
    endpoint,
    { method: "GET" },
    options.timeoutMs ?? 8_000,
    "Ollama model discovery",
  );

  return parseTagsResponse(payload);
}

export async function checkOllamaStatus(
  options: OllamaProviderOptions = {},
): Promise<OllamaConnectionStatus> {
  const baseUrl = options.baseUrl ?? defaultOllamaApiBaseUrl;

  try {
    const models = await discoverOllamaModels(options);
    return { ok: true, baseUrl, models };
  } catch (error) {
    const normalized = toOllamaError(error);
    return {
      ok: false,
      baseUrl,
      code: normalized.code,
      message: normalized.message,
    };
  }
}

export function hasOllamaModel(models: OllamaModelSummary[], selectedModel: string): boolean {
  const normalized = selectedModel.trim().toLowerCase();
  if (!normalized) {
    return false;
  }

  return models.some((model) =>
    [model.name, model.model].some((name) => name.toLowerCase() === normalized),
  );
}

export function resolveOllamaEndpoint(baseUrl: string, endpoint: "generate" | "tags"): string {
  let parsed: URL;
  try {
    parsed = new URL(baseUrl);
  } catch {
    throw new OllamaProviderError("invalid_url", "Ollama URL is not valid.");
  }

  const normalizedPath = parsed.pathname.replace(/\/+$/, "");
  const apiPath =
    normalizedPath === "" || normalizedPath === "/"
      ? "/api"
      : normalizedPath.endsWith("/api")
        ? normalizedPath
        : `${normalizedPath}/api`;

  parsed.pathname = `${apiPath}/${endpoint}`.replace(/\/{2,}/g, "/");
  parsed.search = "";
  parsed.hash = "";
  return parsed.toString();
}

function assertLocalDictationEndpoint(url: string): void {
  const decision = evaluateDictationNetworkUrl(url);
  if (decision.allowed) {
    return;
  }

  if (decision.reason === "invalid_url") {
    throw new OllamaProviderError("invalid_url", "Ollama URL is not valid.");
  }

  throw new OllamaProviderError(
    "remote_network_blocked",
    "LocalFlow only allows local Ollama endpoints for ordinary dictation.",
  );
}

async function requestOllamaJson(
  fetchImpl: FetchLike,
  endpoint: string,
  init: RequestInit,
  timeoutMs: number,
  label: string,
  model?: string,
): Promise<unknown> {
  let response: Response;
  try {
    response = await runWithTimeout(fetchImpl(endpoint, init), timeoutMs, label);
  } catch (error) {
    if (error instanceof OllamaProviderError) {
      throw error;
    }

    throw new OllamaProviderError(
      "unavailable",
      `${label} could not reach local Ollama. Start Ollama and try again.`,
    );
  }

  if (!response.ok) {
    const message = await readOllamaError(response);
    if (response.status === 404) {
      throw new OllamaProviderError(
        "model_not_found",
        model
          ? `Ollama model "${model}" was not found locally.`
          : "The requested Ollama endpoint or model was not found locally.",
        response.status,
      );
    }

    throw new OllamaProviderError(
      "http_error",
      message || `Ollama returned HTTP ${response.status}.`,
      response.status,
    );
  }

  try {
    return await response.json();
  } catch {
    throw new OllamaProviderError(
      "invalid_response",
      "Ollama returned a response that was not valid JSON.",
    );
  }
}

async function readOllamaError(response: Response): Promise<string> {
  try {
    const payload: unknown = await response.json();
    if (isRecord(payload) && typeof payload.error === "string") {
      return payload.error;
    }
  } catch {
    // Fall through to the generic status message.
  }

  return response.statusText;
}

function parseTagsResponse(payload: unknown): OllamaModelSummary[] {
  if (!isRecord(payload) || !Array.isArray(payload.models)) {
    throw new OllamaProviderError(
      "invalid_response",
      "Ollama model list did not include a models array.",
    );
  }

  return payload.models.map((model) => parseModelSummary(model));
}

function parseModelSummary(payload: unknown): OllamaModelSummary {
  if (!isRecord(payload)) {
    throw new OllamaProviderError("invalid_response", "Ollama model entry was not an object.");
  }

  const name = typeof payload.name === "string" ? payload.name : "";
  const model = typeof payload.model === "string" ? payload.model : name;
  if (!name && !model) {
    throw new OllamaProviderError("invalid_response", "Ollama model entry was missing a name.");
  }

  return {
    name: name || model,
    model: model || name,
    modifiedAt: typeof payload.modified_at === "string" ? payload.modified_at : undefined,
    sizeBytes: typeof payload.size === "number" ? payload.size : undefined,
    digest: typeof payload.digest === "string" ? payload.digest : undefined,
    details: parseModelDetails(payload.details),
  };
}

function parseModelDetails(payload: unknown): OllamaModelDetails | undefined {
  if (!isRecord(payload)) {
    return undefined;
  }

  return {
    format: typeof payload.format === "string" ? payload.format : undefined,
    family: typeof payload.family === "string" ? payload.family : undefined,
    families: isStringArray(payload.families) ? payload.families : undefined,
    parameterSize: typeof payload.parameter_size === "string" ? payload.parameter_size : undefined,
    quantizationLevel:
      typeof payload.quantization_level === "string" ? payload.quantization_level : undefined,
  };
}

function parseGenerateResponse(payload: unknown): string {
  if (!isRecord(payload) || typeof payload.response !== "string") {
    throw new OllamaProviderError(
      "invalid_response",
      "Ollama generate response did not include text output.",
    );
  }

  return payload.response;
}

function toOllamaError(error: unknown): OllamaProviderError {
  if (error instanceof OllamaProviderError) {
    return error;
  }

  return new OllamaProviderError(
    "unavailable",
    error instanceof Error ? error.message : "Ollama is unavailable.",
  );
}

function resolveGlobalFetch(): FetchLike {
  if (typeof globalThis.fetch !== "function") {
    throw new OllamaProviderError(
      "unavailable",
      "This environment does not provide fetch for Ollama requests.",
    );
  }

  return globalThis.fetch.bind(globalThis);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStringArray(value: unknown): value is string[] {
  return Array.isArray(value) && value.every((item) => typeof item === "string");
}
