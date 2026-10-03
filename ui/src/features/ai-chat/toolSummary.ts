// One-line summaries of tool calls for the chips (the full input/result is one click away).

const MAX = 64;

function clip(s: string, max = MAX): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

function value(v: unknown): string {
  if (typeof v === "string") return /^[0-9A-HJKMNP-TV-Z]{26}$/.test(v) ? "…" + v.slice(-4) : `"${clip(v, 24)}"`;
  if (typeof v === "number" || typeof v === "boolean") return String(v);
  if (Array.isArray(v)) return `${v.length} item${v.length === 1 ? "" : "s"}`;
  if (v === null) return "null";
  return "{…}";
}

/** `kind: midi, name: "Bass"` — ids shortened, arrays counted. */
export function summarizeInput(input: unknown): string {
  if (typeof input !== "object" || input === null || Array.isArray(input)) return input === undefined ? "" : value(input);
  return clip(
    Object.entries(input as Record<string, unknown>)
      .map(([k, v]) => `${k}: ${value(v)}`)
      .join(", "),
  );
}

/** The result's first line (errors), or a compact view of its JSON. */
export function summarizeResult(content: string): string {
  try {
    const parsed: unknown = JSON.parse(content);
    if (typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)) return summarizeInput(parsed);
  } catch {
    /* plain text */
  }
  return clip(content.split("\n")[0] ?? "");
}

/** Pretty JSON when it parses, else the text as is. */
export function pretty(content: unknown): string {
  if (typeof content !== "string") return JSON.stringify(content, null, 2) ?? "";
  try {
    return JSON.stringify(JSON.parse(content), null, 2);
  } catch {
    return content;
  }
}
