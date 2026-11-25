/** The only network address the web UI may contact: the local Ollama server. */
export const localOllamaPort = "11434";

const localHostnames = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

export type NetworkDecision =
  | { allowed: true; reason: "localhost" }
  | {
      allowed: false;
      reason: "remote_network_blocked" | "invalid_url" | "unsupported_scheme" | "unexpected_port";
    };

/**
 * Whether a URL may be fetched during ordinary dictation. Only plain HTTP to this computer, on
 * Ollama's port, with no credentials in the URL, is allowed; everything else is refused before
 * any request is made.
 */
export function evaluateDictationNetworkUrl(url: string): NetworkDecision {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { allowed: false, reason: "invalid_url" };
  }

  if (parsed.username || parsed.password) {
    return { allowed: false, reason: "invalid_url" };
  }
  if (!localHostnames.has(parsed.hostname)) {
    return { allowed: false, reason: "remote_network_blocked" };
  }
  if (parsed.protocol !== "http:") {
    return { allowed: false, reason: "unsupported_scheme" };
  }
  if (parsed.port !== localOllamaPort) {
    return { allowed: false, reason: "unexpected_port" };
  }
  return { allowed: true, reason: "localhost" };
}
