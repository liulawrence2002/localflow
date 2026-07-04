export type NetworkDecision =
  | { allowed: true; reason: "localhost" }
  | { allowed: false; reason: "remote_network_blocked" | "invalid_url" };

export function evaluateDictationNetworkUrl(url: string): NetworkDecision {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return { allowed: false, reason: "invalid_url" };
  }

  if (
    parsed.hostname === "localhost" ||
    parsed.hostname === "127.0.0.1" ||
    parsed.hostname === "[::1]" ||
    parsed.hostname === "::1"
  ) {
    return { allowed: true, reason: "localhost" };
  }

  return { allowed: false, reason: "remote_network_blocked" };
}
