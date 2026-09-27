/** Server address helpers for the connect dialog. */

/** `host:port` → `ws://host:port/`; keeps explicit `ws://` / `wss://` URLs. */
export function normalizeServerUrl(input: string): string | null {
  const s = input.trim();
  if (!s) return null;
  const withScheme = /^wss?:\/\//i.test(s) ? s : /^[a-z]+:\/\//i.test(s) ? null : `ws://${s}`;
  if (!withScheme) return null;
  try {
    const u = new URL(withScheme);
    return u.protocol === "ws:" || u.protocol === "wss:" ? u.toString() : null;
  } catch {
    return null;
  }
}
