import { describe, expect, it } from "vitest";
import {
  OllamaRefinementProvider,
  checkOllamaStatus,
  defaultOllamaApiBaseUrl,
  discoverOllamaModels,
  hasOllamaModel,
  resolveOllamaEndpoint,
  type OllamaProviderOptions,
} from "../src/domain/ollama";
import { runLocalCleanup, type DictationCleanupInput } from "../src/domain/refinementPipeline";

const cleanupInput: DictationCleanupInput = {
  rawTranscript: "send it to james actually send it to sarah",
  deterministicText: "send it to Sarah",
  appCategory: "work_messaging",
  beforeCursor: "",
  afterCursor: "",
  cleanupLevel: "balanced",
  codeMode: false,
  dictionary: [],
  replacements: [],
  styleName: "Work messages",
};

describe("Ollama local provider", () => {
  it("resolves the default local Ollama API endpoints", () => {
    expect(resolveOllamaEndpoint("http://localhost:11434", "tags")).toBe(
      "http://localhost:11434/api/tags",
    );
    expect(resolveOllamaEndpoint(defaultOllamaApiBaseUrl, "generate")).toBe(
      "http://127.0.0.1:11434/api/generate",
    );
  });

  it("discovers installed local models from /api/tags", async () => {
    const calls: string[] = [];
    const models = await discoverOllamaModels({
      fetchImpl: async (input) => {
        calls.push(String(input));
        return jsonResponse({
          models: [
            {
              name: "llama3.1:8b-instruct",
              model: "llama3.1:8b-instruct",
              modified_at: "2026-01-02T03:04:05Z",
              size: 4_900_000_000,
              digest: "abc123",
              details: {
                format: "gguf",
                family: "llama",
                families: ["llama"],
                parameter_size: "8B",
                quantization_level: "Q4_K_M",
              },
            },
          ],
        });
      },
    });

    expect(calls).toEqual(["http://127.0.0.1:11434/api/tags"]);
    expect(models[0]).toMatchObject({
      name: "llama3.1:8b-instruct",
      sizeBytes: 4_900_000_000,
      details: { parameterSize: "8B", quantizationLevel: "Q4_K_M" },
    });
    expect(hasOllamaModel(models, "LLAMA3.1:8B-INSTRUCT")).toBe(true);
  });

  it("blocks remote Ollama URLs before fetch is called", async () => {
    const options: OllamaProviderOptions = {
      baseUrl: "https://ollama.example.com/api",
      fetchImpl: async () => {
        throw new Error("fetch should not be called");
      },
    };

    await expect(discoverOllamaModels(options)).rejects.toMatchObject({
      code: "remote_network_blocked",
    });
  });

  it("returns a clear unavailable status when local Ollama cannot be reached", async () => {
    const status = await checkOllamaStatus({
      fetchImpl: async () => {
        throw new TypeError("connection refused");
      },
    });

    expect(status).toMatchObject({
      ok: false,
      code: "unavailable",
    });
  });

  it("requires a selected local model before cleanup", async () => {
    const provider = new OllamaRefinementProvider({
      model: "",
      fetchImpl: async () => jsonResponse({ response: "{}" }),
    });

    await expect(provider.complete("{}")).rejects.toMatchObject({
      code: "model_not_selected",
    });
  });

  it("maps missing local models to a model-not-found error", async () => {
    const provider = new OllamaRefinementProvider({
      model: "missing:latest",
      fetchImpl: async () =>
        jsonResponse({ error: 'model "missing:latest" not found' }, { status: 404 }),
    });

    await expect(provider.complete("{}")).rejects.toMatchObject({
      code: "model_not_found",
      status: 404,
    });
  });

  it("sends non-streaming JSON-format generate requests", async () => {
    let requestBody: unknown;
    const provider = new OllamaRefinementProvider({
      model: "llama3.1:8b-instruct",
      fetchImpl: async (_input, init) => {
        requestBody = JSON.parse(String(init?.body));
        return jsonResponse({
          response: JSON.stringify({
            text: "Send it to Sarah.",
            confidence: 0.91,
            resolved_corrections: ["James -> Sarah"],
            warnings: [],
          }),
        });
      },
    });

    const payload = await provider.complete("cleanup prompt");

    expect(requestBody).toMatchObject({
      model: "llama3.1:8b-instruct",
      prompt: "cleanup prompt",
      stream: false,
      format: "json",
    });
    expect(payload).toContain("Send it to Sarah.");
  });

  it("never follows a redirect away from the local server", async () => {
    const inits: Array<RequestInit | undefined> = [];
    await discoverOllamaModels({
      fetchImpl: async (_input, init) => {
        inits.push(init);
        return jsonResponse({ models: [] });
      },
    });

    expect(inits[0]?.redirect).toBe("error");
  });

  it("refuses an Ollama URL on another local port before fetch is called", async () => {
    await expect(
      discoverOllamaModels({
        baseUrl: "http://127.0.0.1:8080/api",
        fetchImpl: async () => {
          throw new Error("fetch should not be called");
        },
      }),
    ).rejects.toMatchObject({ code: "remote_network_blocked" });
  });

  it("does not show Ollama's error body, which can quote the prompt", async () => {
    const provider = new OllamaRefinementProvider({
      model: "llama3.2:3b",
      fetchImpl: async () =>
        jsonResponse(
          { error: "cannot process prompt: call me at five five five" },
          { status: 500, statusText: "Internal Server Error" },
        ),
    });

    const failure = await provider.complete("{}").catch((error: Error) => error);

    expect(failure).toMatchObject({ code: "http_error", status: 500 });
    expect(String((failure as Error).message)).not.toContain("five five five");
    expect(String((failure as Error).message)).toContain("HTTP 500");
  });

  it("reports a malformed model list as an invalid response", async () => {
    const status = await checkOllamaStatus({
      fetchImpl: async () => jsonResponse({ models: "not a list" }),
    });

    expect(status.ok).toBe(false);
    if (!status.ok) {
      expect(status.code).toBe("invalid_response");
    }
  });

  it("integrates with the local cleanup JSON contract", async () => {
    const provider = new OllamaRefinementProvider({
      model: "llama3.1:8b-instruct",
      fetchImpl: async () =>
        jsonResponse({
          response: JSON.stringify({
            text: "Send it to Sarah.",
            confidence: 0.88,
            resolved_corrections: ["James -> Sarah"],
            warnings: [],
          }),
        }),
    });

    const result = await runLocalCleanup(provider, cleanupInput);

    expect(result.source).toBe("model");
    expect(result.text).toBe("Send it to Sarah.");
  });
});

function jsonResponse(body: unknown, init?: ResponseInit): Response {
  return new Response(JSON.stringify(body), {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}
