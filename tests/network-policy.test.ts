import { describe, expect, it } from "vitest";
import { evaluateDictationNetworkUrl } from "../src/domain/networkPolicy";

describe("dictation network policy", () => {
  it("allows plain HTTP to Ollama's port on this computer", () => {
    for (const url of [
      "http://127.0.0.1:11434/api/generate",
      "http://localhost:11434/api/tags",
      "http://[::1]:11434/api/tags",
    ]) {
      expect(evaluateDictationNetworkUrl(url), url).toEqual({ allowed: true, reason: "localhost" });
    }
  });

  it("blocks remote provider URLs during ordinary dictation", () => {
    expect(evaluateDictationNetworkUrl("https://api.example.com/v1/chat")).toEqual({
      allowed: false,
      reason: "remote_network_blocked",
    });
    expect(evaluateDictationNetworkUrl("http://192.168.1.20:11434/api/tags")).toMatchObject({
      allowed: false,
      reason: "remote_network_blocked",
    });
  });

  it("refuses other ports, other schemes and credentials even on localhost", () => {
    expect(evaluateDictationNetworkUrl("http://localhost:8080/completion")).toMatchObject({
      allowed: false,
      reason: "unexpected_port",
    });
    expect(evaluateDictationNetworkUrl("http://localhost/api/tags")).toMatchObject({
      allowed: false,
      reason: "unexpected_port",
    });
    expect(evaluateDictationNetworkUrl("https://127.0.0.1:11434/api/tags")).toMatchObject({
      allowed: false,
      reason: "unsupported_scheme",
    });
    expect(evaluateDictationNetworkUrl("http://user:secret@127.0.0.1:11434/api")).toMatchObject({
      allowed: false,
      reason: "invalid_url",
    });
    expect(evaluateDictationNetworkUrl("not a url")).toMatchObject({
      allowed: false,
      reason: "invalid_url",
    });
  });
});
