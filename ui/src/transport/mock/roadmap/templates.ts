/**
 * Mock of `Template::*` (v0.3, owned by `templates`; CONTRACTS.md §13.10), same rules as the
 * engine (`crates/ether-controller/src/templates`):
 * - factory track templates `"factory/<slug>"` (read-only, the same four as the engine);
 * - user templates in an in-memory library per MockTransport, ids `"projects/<name>"` /
 *   `"tracks/<name>"`, names unique per kind (case-insensitive), saves replace only with
 *   `overwrite`; every library change emits `TemplateEvent::Changed`;
 * - `SaveTracks` keeps the tracks (with group children), their devices, pads, rack chains,
 *   modulators, sends between them, track automation and modulation mappings; references
 *   to tracks outside are cut;
 * - `Insert` is a document command (one undo step): entity `i` gets `deriveId(seed, i)`,
 *   references are remapped, top-level tracks go under `parent` before `before`. The mock
 *   has no user library files: sample references are kept only when the project has that
 *   media (else cleared);
 * - `NewProject` creates and opens a project from a project template (or the default one,
 *   or an empty project).
 */

import type {
  BuiltinDeviceType,
  Device,
  Entity,
  PresetMeta,
  Project,
  ProjectId,
  ReplyValue,
  TemplateCommand,
  TemplateInfo,
  TemplateKind,
  Track,
  TrackId,
} from "@/generated";
import { deriveId } from "@/features/comping/model";
import { entityKeyOf } from "@/state/entities";
import { keyBetween, keyForInsert } from "@/state/orderKey";
import { builtinDescriptor, newBuiltinDevice } from "../builtinDevices";
import { makeTrack } from "../demoProject";
import { fail, type ReducerContext } from "../documentReducer";
import type { MockHost } from "./host";

export type InsertTemplateCommand = Extract<TemplateCommand, { type: "Insert" }>;

/** What the templates mock needs from the MockTransport. */
export interface TemplatesHost extends MockHost {
  /** Create and open project `id` named `name`: a copy of `template`, or an empty one. */
  newProject(id: ProjectId, name: string, template: Project | null): ReplyValue;
  /** Wall clock (ms). */
  now(): number;
}

type Body = { type: "Tracks"; entities: Entity[] } | { type: "Project"; project: Project };

interface Stored {
  name: string;
  meta: PresetMeta;
  body: Body;
  modified_ms: number;
}

const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

// ─── Factory templates (same as `templates/factory.rs`) ───────────────────────────────

function factoryTemplate(slug: string, name: string, tags: string[], description: string, tracks: Array<[Track["kind"], string, number, BuiltinDeviceType[], number | null]>): [string, Stored] {
  // Fixed ids (replaced on insert anyway).
  let n = 0;
  const id = () => `01KFACT${slug.length.toString().padStart(2, "0")}${String(++n).padStart(17, "0")}`.slice(0, 26).toUpperCase();
  const entities: Entity[] = [];
  const trackIds: string[] = [];
  let lastTop: string | null = null;
  const lastChild = new Map<string, string>();
  for (const [kind, tname, color, devices, parentIndex] of tracks) {
    const tid = id();
    const parent = parentIndex === null ? null : trackIds[parentIndex]!;
    const prev = parent === null ? lastTop : (lastChild.get(parent) ?? null);
    const order = keyBetween(prev, null);
    if (parent === null) lastTop = order;
    else lastChild.set(parent, order);
    trackIds.push(tid);
    entities.push({ type: "Track", value: makeTrack({ id: tid, kind, name: tname, color, order, parent }) });
    let chain: string | null = null;
    for (const ty of devices) {
      chain = keyBetween(chain, null);
      const desc = builtinDescriptor(ty);
      const params: Record<number, number> = {};
      for (const p of desc.params) params[p.id] = p.default;
      entities.push({
        type: "Device",
        value: { id: id(), track: tid, order: chain, name: desc.name, enabled: true, kind: { type: "Builtin", device: newBuiltinDevice(ty) }, params, sidechain: null, pad: null },
      });
    }
  }
  return [`factory/${slug}`, { name, meta: { tags, author: "Ethereal", description }, body: { type: "Tracks", entities }, modified_ms: 0 }];
}

const FACTORY: ReadonlyMap<string, Stored> = new Map([
  factoryTemplate("vocal-chain", "Vocal Chain", ["vocal"], "Audio track with EQ, compressor and reverb.", [["Audio", "Vocal", 0xe8919d, ["Eq", "Compressor", "Reverb"], null]]),
  factoryTemplate("synth-keys", "Synth Keys", ["keys", "synth"], "MIDI track with a poly synth, chorus and delay.", [["Midi", "Keys", 0x8fa8e6, ["PolySynth", "Chorus", "Delay"], null]]),
  factoryTemplate("drum-bus", "Drum Bus", ["drums"], "Drum group with bus compression and saturation, and a drum rack inside.", [
    ["Group", "Drums", 0xe6c07e, ["Compressor", "Saturator"], null],
    ["Midi", "Kit", 0xe6c07e, ["DrumRack"], 0],
  ]),
  factoryTemplate("reverb-return", "Reverb Return", ["return"], "Return track with a reverb.", [["Return", "Reverb", 0x7cc6c0, ["Reverb"], null]]),
]);

// ─── Helpers ─────────────────────────────────────────────────────────────────────────

const PREFIX: Record<TemplateKind, string> = { Project: "projects", Tracks: "tracks" };
const kindOf = (b: Body): TemplateKind => b.type;

/** File stem of a name (same rules as the engine's `file_stem`). */
export function fileStem(name: string): string {
  const cleaned = [...name]
    .map((c) => (c.charCodeAt(0) < 32 || /[/\\:*?"<>|]/.test(c) ? "-" : c))
    .slice(0, 80)
    .join("");
  const trimmed = cleaned.trim().replace(/^\.+/, "").replace(/\.+$/, "").trim();
  return trimmed || "Template";
}

function normalizeMeta(meta: PresetMeta): PresetMeta {
  const tags = [...new Set(meta.tags.map((t) => t.trim().toLowerCase()).filter(Boolean))].sort();
  const text = (s: string | null) => (s?.trim() ? s.trim() : null);
  return { tags, author: text(meta.author), description: text(meta.description) };
}

function checkName(name: string): string {
  const n = name.trim();
  if (!n) fail("InvalidArgument", "a template needs a name");
  return n;
}

function childTracks(p: Project, parent: TrackId | null): Track[] {
  return Object.values(p.tracks)
    .filter((t) => t.parent === parent)
    .sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1));
}

/** The entities of a track template (engine: `templates/collect.rs`). */
export function collectTracks(p: Project, tracks: ReadonlyArray<TrackId>): Entity[] {
  if (!tracks.length) fail("InvalidArgument", "a track template needs at least one track");
  for (const id of tracks) {
    const t = p.tracks[id] ?? fail("NotFound", `track ${id}`);
    if (t.kind === "Master") fail("InvalidArgument", "the master track cannot be saved as a template");
  }
  const selected = new Set(tracks);
  const order: TrackId[] = [];
  const walk = (parent: TrackId | null) => {
    for (const t of childTracks(p, parent)) {
      order.push(t.id);
      walk(t.id);
    }
  };
  walk(null);
  const set = new Set<TrackId>();
  for (const id of order) {
    for (let cur: TrackId | null = id; cur !== null; cur = p.tracks[cur]?.parent ?? null) {
      if (selected.has(cur)) {
        set.add(id);
        break;
      }
    }
  }
  const out: Entity[] = [];
  for (const id of order.filter((i) => set.has(i))) {
    const t = clone(p.tracks[id]!);
    if (t.parent !== null && !set.has(t.parent)) t.parent = null;
    if (t.output.type === "Track" && !set.has(t.output.track)) t.output = { type: "Default" };
    if (t.input.type === "Track" && !set.has(t.input.track)) t.input = { type: "None" };
    if (t.vca && !set.has(t.vca)) delete t.vca;
    delete t.freeze;
    out.push({ type: "Track", value: t });
  }
  const devices = new Set<string>();
  const pads = new Set<string>();
  const chains = new Set<string>();
  const fixDevice = (d: Device): Device => {
    const c = clone(d);
    if (c.sidechain !== null && !set.has(c.sidechain)) c.sidechain = null;
    return c;
  };
  for (const d of Object.values(p.devices)) {
    if (set.has(d.track) && d.pad === null && !d.chain) {
      devices.add(d.id);
      out.push({ type: "Device", value: fixDevice(d) });
    }
  }
  for (let progressed = true; progressed; ) {
    progressed = false;
    for (const pad of Object.values(p.drum_pads)) {
      if (devices.has(pad.rack) && !pads.has(pad.id)) {
        pads.add(pad.id);
        out.push({ type: "DrumPad", value: clone(pad) });
        progressed = true;
      }
    }
    for (const c of Object.values(p.rack_chains)) {
      if (devices.has(c.rack) && !chains.has(c.id)) {
        chains.add(c.id);
        out.push({ type: "RackChain", value: clone(c) });
        progressed = true;
      }
    }
    for (const d of Object.values(p.devices)) {
      const parentIn = (d.pad !== null && pads.has(d.pad)) || (!!d.chain && chains.has(d.chain));
      if (parentIn && !devices.has(d.id)) {
        devices.add(d.id);
        out.push({ type: "Device", value: fixDevice(d) });
        progressed = true;
      }
    }
  }
  const modulators = new Set<string>();
  for (const m of Object.values(p.modulators)) {
    if (!devices.has(m.device)) continue;
    const c = clone(m);
    if (c.sidechain && !set.has(c.sidechain)) delete c.sidechain;
    modulators.add(c.id);
    out.push({ type: "Modulator", value: c });
  }
  const sends = new Set<string>();
  for (const s of Object.values(p.sends)) {
    if (set.has(s.from) && set.has(s.to)) {
      sends.add(s.id);
      out.push({ type: "Send", value: clone(s) });
    }
  }
  const lanes = Object.values(p.automation_lanes).filter((l) => {
    if (l.owner.type !== "Track" || !set.has(l.owner.track)) return false;
    const t = l.target;
    if (t.type === "TrackVolume" || t.type === "TrackPan") return set.has(t.track);
    if (t.type === "SendLevel") return sends.has(t.send);
    return devices.has(t.device);
  });
  for (const l of lanes) out.push({ type: "AutomationLane", value: clone(l) });
  const laneIds = new Set(lanes.map((l) => l.id));
  for (const pt of Object.values(p.automation_points)) if (laneIds.has(pt.lane)) out.push({ type: "AutomationPoint", value: clone(pt) });
  for (const m of Object.values(p.mod_mappings)) {
    const sourceIn = m.source.type === "Modulator" ? modulators.has(m.source.modulator) : devices.has(m.source.rack);
    if (sourceIn && devices.has(m.device)) out.push({ type: "ModMapping", value: clone(m) });
  }
  return out;
}

/** Rewrite every string (and object key) found in `map`. */
function rewrite(v: unknown, map: ReadonlyMap<string, string>): unknown {
  if (typeof v === "string") return map.get(v) ?? v;
  if (Array.isArray(v)) return v.map((x) => rewrite(x, map));
  if (v && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, x] of Object.entries(v)) out[map.get(k) ?? k] = rewrite(x, map);
    return out;
  }
  return v;
}

/** The template's entities with ids `deriveId(seed, i)` and remapped references. */
export function remapEntities(entities: ReadonlyArray<Entity>, seed: string): Entity[] {
  const map = new Map<string, string>();
  entities.forEach((e, i) => map.set(entityKeyOf(e).id, deriveId(seed, i)));
  return entities.map((e) => rewrite(clone(e), map) as Entity);
}

// ─── The library ─────────────────────────────────────────────────────────────────────

let active: MockTemplates | null = null;
/** The library `Insert` reads: the MockTransport that last constructed or used its own. */
const activate = (t: MockTemplates) => {
  active = t;
};

export class MockTemplates {
  private readonly user = new Map<string, Stored>();
  private defaultId: string | null = null;

  constructor(private readonly host: TemplatesHost) {
    activate(this);
  }

  /** A template by id (factory or user). */
  get(id: string): Stored {
    return FACTORY.get(id) ?? this.user.get(id) ?? fail("NotFound", `template ${id}`);
  }

  private info(id: string, s: Stored): TemplateInfo {
    return { id, kind: kindOf(s.body), name: s.name, meta: s.meta, factory: FACTORY.has(id), default: this.defaultId === id, modified_ms: s.modified_ms };
  }

  private changed(): void {
    this.host.emit({ type: "Template", event: { type: "Changed" } });
  }

  private named(kind: TemplateKind, name: string, except?: string): string | undefined {
    const lower = name.toLowerCase();
    return [...this.user].find(([id, s]) => id !== except && kindOf(s.body) === kind && s.name.toLowerCase() === lower)?.[0];
  }

  private freeId(kind: TemplateKind, stem: string, except?: string): string {
    for (let n = 1; ; n++) {
      const id = `${PREFIX[kind]}/${n === 1 ? stem : `${stem} ${n}`}`;
      const taken = [...this.user.keys()].some((k) => k.toLowerCase() === id.toLowerCase());
      if (!taken || id.toLowerCase() === except?.toLowerCase()) return id;
    }
  }

  private save(name: string, meta: PresetMeta, overwrite: boolean, body: Body): ReplyValue {
    const n = checkName(name);
    const kind = kindOf(body);
    const existing = this.named(kind, n);
    if (existing && !overwrite) fail("InvalidArgument", `a template named "${n}" already exists`);
    const id = existing ?? this.freeId(kind, fileStem(n));
    const stored: Stored = { name: n, meta: normalizeMeta(meta), body, modified_ms: this.host.now() };
    this.user.set(id, stored);
    this.changed();
    return { type: "Template", template: this.info(id, stored) };
  }

  private userEntry(id: string): Stored {
    if (FACTORY.has(id)) fail("InvalidArgument", "factory templates are read-only");
    return this.user.get(id) ?? fail("NotFound", `template ${id}`);
  }

  command(c: Exclude<TemplateCommand, InsertTemplateCommand>): ReplyValue {
    activate(this);
    switch (c.type) {
      case "List": {
        const byName = (a: TemplateInfo, b: TemplateInfo) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.id.localeCompare(b.id);
        const want = (s: Stored) => c.kind === null || kindOf(s.body) === c.kind;
        const factory = [...FACTORY].filter(([, s]) => want(s)).map(([id, s]) => this.info(id, s));
        const user = [...this.user].filter(([, s]) => want(s)).map(([id, s]) => this.info(id, s));
        return { type: "Templates", templates: [...factory.sort(byName), ...user.sort(byName)] };
      }
      case "SaveTracks":
        return this.save(c.name, c.meta, c.overwrite, { type: "Tracks", entities: collectTracks(this.host.project(), c.tracks) });
      case "SaveProject": {
        const project = clone(this.host.project());
        project.chat = {};
        return this.save(c.name, c.meta, c.overwrite, { type: "Project", project });
      }
      case "Rename": {
        const s = this.userEntry(c.template);
        const n = checkName(c.name);
        const kind = kindOf(s.body);
        if (this.named(kind, n, c.template)) fail("InvalidArgument", `a template named "${n}" already exists`);
        const id = this.freeId(kind, fileStem(n), c.template);
        const stored = { ...s, name: n, modified_ms: this.host.now() };
        this.user.delete(c.template);
        this.user.set(id, stored);
        if (this.defaultId === c.template) this.defaultId = id;
        this.changed();
        return { type: "Template", template: this.info(id, stored) };
      }
      case "Delete":
        this.userEntry(c.template);
        this.user.delete(c.template);
        if (this.defaultId === c.template) this.defaultId = null;
        this.changed();
        return { type: "Unit" };
      case "SetDefault":
        if (c.template !== null && kindOf(this.get(c.template).body) !== "Project") fail("InvalidArgument", `${c.template} is not a project template`);
        this.defaultId = c.template;
        this.changed();
        return { type: "Unit" };
      case "NewProject": {
        let id = c.template;
        if (id === null && this.defaultId !== null && (FACTORY.has(this.defaultId) || this.user.has(this.defaultId))) id = this.defaultId;
        if (id === null) return this.host.newProject(c.id, c.name, null);
        const s = this.get(id);
        if (s.body.type !== "Project") fail("InvalidArgument", `"${s.name}" is a track template`);
        return this.host.newProject(c.id, c.name, clone(s.body.project));
      }
    }
  }
}

// ─── Insert (document command) ───────────────────────────────────────────────────────

/** `Template::Insert` (document command). */
export function insertTemplate(ctx: ReducerContext, c: InsertTemplateCommand): void {
  const { tx } = ctx;
  if (tx.get("Track", deriveId(c.seed, 0))) return; // retried message
  const lib = active ?? fail("InvalidState", "no template library");
  const s = lib.get(c.template);
  if (s.body.type !== "Tracks") fail("InvalidArgument", `"${s.name}" is a project template`);
  if (c.parent !== null && tx.get("Track", c.parent)?.kind !== "Group") fail("InvalidArgument", `parent ${c.parent} is not a group track`);
  if (c.before !== null && (tx.get("Track", c.before) ?? fail("NotFound", `track ${c.before}`)).parent !== c.parent) fail("InvalidArgument", `${c.before} is not a sibling`);
  const entities = remapEntities(s.body.entities, c.seed);
  if (entities[0]?.type !== "Track") fail("InvalidArgument", "a track template starts with a track");
  const tracks = new Set(entities.flatMap((e) => (e.type === "Track" ? [e.value.id] : [])));
  const sends = new Set<string>();
  const lanes = new Set<string>();
  const siblings = (parent: TrackId | null) => childTracks(tx.project, parent);
  for (const e of entities) {
    switch (e.type) {
      case "Track": {
        const t = e.value;
        if (t.kind === "Master") fail("InvalidArgument", "a track template cannot contain the master track");
        if (t.parent === null || !tracks.has(t.parent)) {
          if (c.parent !== null && t.kind === "Return") fail("InvalidArgument", "return tracks must be top-level");
          t.parent = c.parent;
          const before =
            c.before ?? (c.parent === null ? (siblings(null).find((x) => x.kind === "Master" || (t.kind !== "Return" && x.kind === "Return"))?.id ?? null) : null);
          t.order = keyForInsert(siblings(c.parent), before);
        }
        if (t.output.type === "Track" && !tracks.has(t.output.track)) t.output = { type: "Default" };
        if (t.input.type === "Track" && !tracks.has(t.input.track)) t.input = { type: "None" };
        if (t.vca && !tracks.has(t.vca)) delete t.vca;
        delete t.freeze;
        tx.upsert("Track", t);
        break;
      }
      case "Device": {
        const d = e.value;
        if (d.sidechain !== null && !tracks.has(d.sidechain)) d.sidechain = null;
        if (d.kind.type === "Builtin") {
          const k = d.kind.device;
          const has = (m: string | null | undefined) => m != null && tx.get("Media", m) !== undefined;
          if (k.type === "Sampler" && k.sample !== null && !has(k.sample)) k.sample = null;
          if (k.type === "MultiSampler") for (const z of k.zones) if (z.media !== null && !has(z.media)) z.media = null;
          if (k.type === "ConvolutionReverb" && k.ir?.type === "Media" && !has(k.ir.media)) k.ir = null;
        }
        tx.upsert("Device", d);
        break;
      }
      case "Modulator":
        if (e.value.sidechain && !tracks.has(e.value.sidechain)) delete e.value.sidechain;
        tx.upsert("Modulator", e.value);
        break;
      case "Send":
        if (tracks.has(e.value.from) && tracks.has(e.value.to)) {
          sends.add(e.value.id);
          tx.upsert("Send", e.value);
        }
        break;
      case "AutomationLane": {
        const l = e.value;
        const t = l.target;
        const targetIn =
          t.type === "TrackVolume" || t.type === "TrackPan"
            ? tracks.has(t.track)
            : t.type === "SendLevel"
              ? sends.has(t.send)
              : tracks.has(tx.get("Device", t.device)?.track ?? "");
        if (l.owner.type === "Track" && tracks.has(l.owner.track) && targetIn) {
          lanes.add(l.id);
          tx.upsert("AutomationLane", l);
        }
        break;
      }
      case "AutomationPoint":
        if (lanes.has(e.value.lane)) tx.upsert("AutomationPoint", e.value);
        break;
      case "DrumPad":
        tx.upsert("DrumPad", e.value);
        break;
      case "RackChain":
        tx.upsert("RackChain", e.value);
        break;
      case "ModMapping":
        tx.upsert("ModMapping", e.value);
        break;
      default:
        break; // not carried by track templates
    }
  }
}
