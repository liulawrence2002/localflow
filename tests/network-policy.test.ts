import { describe, expect, it } from "vitest";
import { evaluateDictationNetworkUrl } from "../src/domain/networkPolicy";

describe("dictation network policy", () => {
  it("allows localhost model providers only", () => {
    expect(evaluateDictationNetworkUrl("http://127.0.0.1:11434/api/generate")).toEqual({
      allowed: true,
      reason: "localhost",
    });
    expect(evaluateDictationNetworkUrl("http://localhost:8080/completion")).toEqual({
      allowed: true,
      reason: "localhost",
    });
  });

  it("blocks remote provider URLs during ordinary dictation", () => {
    expect(evaluateDictationNetworkUrl("https://api.example.com/v1/chat")).toEqual({
      allowed: false,
      reason: "remote_network_blocked",
    });
  });
});
