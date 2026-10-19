// eth-allow-case-file: prompts for the model, not the UI.
/**
 * Optional AI suggestions for missing plugins the deterministic rules left unresolved or
 * unsure, through the AI chat's provider abstraction (`features/ai-chat/providers`: any
 * provider, the user's key, base URL and model; nothing new here).
 *
 * Privacy: only plugin metadata is sent (the missing plugin's name, vendor, format, id,
 * version and saved parameter names/values; the installed candidates' names, vendors,
 * formats and versions), never audio, file paths or project names. The dialog shows the
 * exact payload and asks for consent once (remembered on this machine,
 * [`AI_CONSENT_STORAGE`]). Suggestions are validated strictly ([`parseAiResponse`]) and
 * are only ever shown, marked "AI suggestion", for the user to accept.
 */
import type { MissingPlugin, ParamMap, PluginCandidate, PluginDescriptor, PluginFormat } from "@/generated";
import { createSession, PROVIDERS, type Connection, type ProviderId, type ChatSession, type ToolDef } from "@/features/ai-chat/providers";
import { activeConnection, missingSetup, useAiSettings } from "@/features/ai-chat/settings";

export const AI_CONSENT_STORAGE = "eth.resolver.aiConsent";
/** Missing plugins per request (one request = one "call"). */
export const AI_BATCH = 12;
/** Installed plugins offered per missing plugin. */
export const AI_SHORTLIST = 25;
/** Saved parameters sent per missing plugin. */
export const AI_MAX_PARAMS = 48;
/** Below this, a deterministic match is "unsure" and the AI is offered too. */
export const LOW_CONFIDENCE = 0.7;

export function hasAiConsent(): boolean {
  try {
    return localStorage.getItem(AI_CONSENT_STORAGE) === "yes";
  } catch {
    return false;
  }
}

export function setAiConsent(on: boolean): void {
  try {
    if (on) localStorage.setItem(AI_CONSENT_STORAGE, "yes");
    else localStorage.removeItem(AI_CONSENT_STORAGE);
  } catch {
    /* this session only */
  }
}

/** The configured provider, or `null` when the AI chat isn't set up. */
export function aiProvider(): { id: ProviderId; label: string } | null {
  const s = useAiSettings.getState();
  if (missingSetup(s) !== null) return null;
  return { id: s.provider, label: PROVIDERS[s.provider].label };
}

/** Does this item want an AI opinion (no candidate, or only unsure ones)? */
export function wantsAi(m: MissingPlugin): boolean {
  return m.candidates.length === 0 || m.candidates.every((c) => c.confidence < LOW_CONFIDENCE);
}

// ─── Payload ──────────────────────────────────────────────────────────────────────────────

export interface AiCandidate {
  key: string;
  format: PluginFormat;
  name: string;
  vendor: string;
  version: string;
}

export interface AiItem {
  key: string;
  format: PluginFormat;
  id: string;
  name: string;
  vendor: string;
  version: string;
  param_count: number;
  params: Array<{ id: number; name?: string; value: number }>;
  candidates: string[];
}

/** What is sent (shown verbatim in the consent panel). */
export interface AiPayload {
  items: AiItem[];
  candidates: AiCandidate[];
}

const words = (s: string) =>
  s
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter((w) => w.length > 1 && !/^\d+$/.test(w));

/** Installed plugins most likely to matter for `m` (shared words in name/vendor first). */
function shortlist(m: MissingPlugin, installed: PluginDescriptor[]): PluginDescriptor[] {
  if (installed.length <= AI_SHORTLIST) return installed;
  const mine = new Set([...words(m.name), ...words(m.vendor)]);
  const score = (p: PluginDescriptor) => [...words(p.name), ...words(p.vendor)].filter((w) => mine.has(w)).length;
  return [...installed]
    .map((p, i) => ({ p, s: score(p), i }))
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .slice(0, AI_SHORTLIST)
    .map((x) => x.p);
}

const pkey = (p: { format: string; id: string }) => `${p.format}:${p.id}`;

/** The payload for `items` (keys `p1…` for missing plugins, `c1…` for candidates). */
export function buildAiPayload(
  items: MissingPlugin[],
  installed: PluginDescriptor[],
): { payload: AiPayload; items: Map<string, MissingPlugin>; candidates: Map<string, PluginDescriptor> } {
  const candKeys = new Map<string, string>();
  const candidates: AiCandidate[] = [];
  const byKey = new Map<string, PluginDescriptor>();
  const outItems: AiItem[] = [];
  const itemMap = new Map<string, MissingPlugin>();
  items.forEach((m, i) => {
    const key = `p${i + 1}`;
    itemMap.set(key, m);
    const list = shortlist(m, installed).map((p) => {
      let k = candKeys.get(pkey(p));
      if (!k) {
        k = `c${candidates.length + 1}`;
        candKeys.set(pkey(p), k);
        candidates.push({ key: k, format: p.format, name: p.name, vendor: p.vendor, version: p.version });
        byKey.set(k, p);
      }
      return k;
    });
    outItems.push({
      key,
      format: m.format,
      id: m.plugin_id,
      name: m.name,
      vendor: m.vendor,
      version: m.version,
      param_count: m.params.length,
      params: m.params.slice(0, AI_MAX_PARAMS).map((p) => ({ id: p.id, ...(p.name ? { name: p.name } : {}), value: Math.round(p.value * 1000) / 1000 })),
      candidates: list,
    });
  });
  return { payload: { items: outItems, candidates }, items: itemMap, candidates: byKey };
}

export const AI_SYSTEM = [
  "You match audio plugins that are missing from a music project to plugins installed on the user's computer.",
  "For each item, pick the installed candidate (by its key, from that item's `candidates` list only) that is the same plugin or its closest replacement (another version or format of the same plugin first, then a plugin of the same kind), or null when none fits.",
  "When you pick a candidate, optionally map saved parameter ids (`from`, from the item's `params`) to the candidate's parameter ids (`to`) only when you are confident they control the same thing.",
  'Reply with JSON only, no prose, exactly: {"matches":[{"item":"p1","candidate":"c3","confidence":0.8,"reason":"short reason","params":[{"from":0,"to":0}]}]}. confidence is 0..1.',
];

export function aiUserMessage(payload: AiPayload): string {
  return JSON.stringify(payload);
}

// ─── Strict validation ────────────────────────────────────────────────────────────────────

export interface AiSuggestion {
  item: string;
  candidate: string | null;
  confidence: number;
  reason: string;
  params: ParamMap[];
}

export class AiResponseError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "AiResponseError";
  }
}

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const isU32 = (v: unknown): v is number => typeof v === "number" && Number.isInteger(v) && v >= 0 && v <= 0xffff_ffff;
const onlyKeys = (o: Record<string, unknown>, allowed: string[]) => Object.keys(o).every((k) => allowed.includes(k));

/**
 * Parse and validate a model's reply against what was sent. The whole reply must be one
 * JSON object (a single surrounding code fence is tolerated) with only `matches`; a match
 * that names an unknown item or a candidate not offered for it, a confidence outside 0..1,
 * extra fields, or parameter ids the item doesn't have is rejected (listed in `rejected`).
 */
export function parseAiResponse(text: string, payload: AiPayload): { suggestions: AiSuggestion[]; rejected: string[] } {
  const trimmed = text.trim();
  const fenced = /^```(?:json)?\s*\n([\s\S]*?)\n?```$/.exec(trimmed);
  let data: unknown;
  try {
    data = JSON.parse(fenced ? fenced[1]! : trimmed);
  } catch {
    throw new AiResponseError("the AI reply is not JSON");
  }
  if (!isObj(data) || !onlyKeys(data, ["matches"]) || !Array.isArray(data.matches)) {
    throw new AiResponseError('the AI reply is not {"matches": [...]}');
  }
  const items = new Map(payload.items.map((i) => [i.key, i]));
  const seen = new Set<string>();
  const suggestions: AiSuggestion[] = [];
  const rejected: string[] = [];
  data.matches.forEach((m: unknown, n: number) => {
    const reject = (why: string) => rejected.push(`match ${n + 1}: ${why}`);
    if (!isObj(m)) return reject("not an object");
    if (!onlyKeys(m, ["item", "candidate", "confidence", "reason", "params"])) return reject("unexpected fields");
    const item = typeof m.item === "string" ? items.get(m.item) : undefined;
    if (!item) return reject("unknown item");
    if (seen.has(item.key)) return reject(`${item.key} answered twice`);
    if (m.candidate !== null && (typeof m.candidate !== "string" || !item.candidates.includes(m.candidate))) {
      return reject(`${item.key}: a candidate that was not offered`);
    }
    if (typeof m.confidence !== "number" || !Number.isFinite(m.confidence) || m.confidence < 0 || m.confidence > 1) {
      return reject(`${item.key}: confidence must be 0..1`);
    }
    if (m.reason !== undefined && (typeof m.reason !== "string" || m.reason.length > 400)) return reject(`${item.key}: bad reason`);
    const params: ParamMap[] = [];
    if (m.params !== undefined) {
      if (!Array.isArray(m.params)) return reject(`${item.key}: params must be a list`);
      const ids = new Set(item.params.map((p) => p.id));
      const targets = new Set<number>();
      for (const p of m.params as unknown[]) {
        if (!isObj(p) || !onlyKeys(p, ["from", "to"]) || !isU32(p.from) || !isU32(p.to)) return reject(`${item.key}: bad parameter mapping`);
        if (!ids.has(p.from)) return reject(`${item.key}: parameter ${p.from} was not sent`);
        if (targets.has(p.to)) return reject(`${item.key}: parameter ${p.to} mapped twice`);
        targets.add(p.to);
        params.push({ from: p.from, to: p.to });
      }
    }
    seen.add(item.key);
    suggestions.push({
      item: item.key,
      candidate: m.candidate as string | null,
      confidence: m.confidence,
      reason: (m.reason as string | undefined) ?? "",
      params,
    });
  });
  return { suggestions, rejected };
}

// ─── Running ──────────────────────────────────────────────────────────────────────────────

export interface AiRunOptions {
  provider: ProviderId;
  conn: Connection;
  signal: AbortSignal;
  /** Tests: a fake provider. */
  session?: (system: string[], tools: ToolDef[]) => ChatSession;
  /** Called after each request (calls so far). */
  onCall?: (calls: number) => void;
}

/** One request with `payload`; the model's whole text. */
async function ask(payload: AiPayload, o: AiRunOptions): Promise<string> {
  const session = o.session ? o.session(AI_SYSTEM, []) : createSession(o.provider, AI_SYSTEM, []);
  session.addUser([aiUserMessage(payload)]);
  let text = "";
  await session.step(o.conn, o.signal, (e) => {
    if (e.type === "text") text += e.delta;
  });
  return text;
}

/** An AI candidate for the dialog, from a validated suggestion. */
export function suggestionCandidate(s: AiSuggestion, target: PluginDescriptor, providerLabel: string): PluginCandidate {
  return {
    format: target.format,
    id: target.id,
    name: target.name,
    vendor: target.vendor,
    version: target.version,
    rule: "Ai",
    confidence: s.confidence,
    reason: `ai suggestion (${providerLabel})${s.reason ? `: ${s.reason}` : ""}`,
    plan: s.params.length > 0 ? { type: "Map", map: s.params } : { type: "Auto" },
    lost: [],
  };
}

export interface AiResult {
  /** Suggested candidates per missing plugin (by `format:plugin_id`). */
  suggestions: Map<string, PluginCandidate>;
  rejected: string[];
  calls: number;
}

/** Ask the provider about `items` in batches (cancellable through `signal`). */
export async function runAi(items: MissingPlugin[], installed: PluginDescriptor[], o: AiRunOptions, providerLabel: string): Promise<AiResult> {
  const out: AiResult = { suggestions: new Map(), rejected: [], calls: 0 };
  for (let i = 0; i < items.length; i += AI_BATCH) {
    const batch = items.slice(i, i + AI_BATCH);
    const built = buildAiPayload(batch, installed);
    const text = await ask(built.payload, o);
    out.calls += 1;
    o.onCall?.(out.calls);
    const { suggestions, rejected } = parseAiResponse(text, built.payload);
    out.rejected.push(...rejected);
    for (const s of suggestions) {
      const m = built.items.get(s.item);
      const target = s.candidate ? built.candidates.get(s.candidate) : undefined;
      if (m && target) out.suggestions.set(pkey({ format: m.format, id: m.plugin_id }), suggestionCandidate(s, target, providerLabel));
    }
  }
  return out;
}

/** The active provider's connection (re-exported for the store). */
export { activeConnection };
