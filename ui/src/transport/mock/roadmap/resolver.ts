/**
 * Mock of `Resolve::*` (CONTRACTS.md §14.2).
 *
 * The mock engine's "installed" plugins are its fake plugins ([`MOCK_INSTALLED_PLUGINS`]:
 * "Mock Mega"). Matching is a small subset of the engine's rules (same id, then the same
 * family name without version numbers and vendor); missing audio files are the media
 * references' missing set, searched in the mock library by file name. `Apply` is one undo
 * step. `addMissingPlugin` (e2e/tests, through `__etherMock.resolver`) puts a device of an
 * uninstalled plugin ("Mock Mega 1") on a track. `Replace` swaps any device but a rack for
 * a built-in or plugin of the same role (a subset of the engine's rules: the same plugin or
 * built-in type keeps everything, else parameters by normalized name when both descriptors
 * are known; the sidechain goes when the new device has no input for it), one undo step.
 * Missing devices (`DeviceKind::Missing` placeholders; `addMissingDevice` puts a "Hybrid
 * Reverb" with an automated Dry/Wet on a track) are proposed with built-in suggestions by
 * name and replaced by `Apply` or `Replace` (built-ins or plugins), parameters matched by
 * name.
 */
import type {
  AutomationTarget,
  BuiltinDeviceType,
  Device,
  DeviceCandidate,
  DeviceCategory,
  DeviceKind,
  MissingDevice,
  MissingDeviceGroup,
  Replacement,
  DeviceId,
  Event,
  MediaCandidate,
  MediaRef,
  MissingPlugin,
  PluginCandidate,
  ParamInfo,
  ParamMatch,
  PluginDescriptor,
  Project,
  ReplaceReport,
  ResolveCommand,
  ReplyValue,
  TrackId,
} from "@/generated";
import { builtinDescriptor, missingDescriptor, newBuiltinDevice } from "../builtinDevices";
import { defaultParams } from "../demoProject";
import { fail } from "../documentReducer";
import { findLibraryFile, LIBRARY_FILES, LIBRARY_ID, normalize } from "../library";
import { MOCK_MEGA_PLUGIN_ID, MOCK_MINI_PLUGIN_ID, mockPluginDescriptor } from "../plugins";
import type { Tx } from "../tx";
import { libraryPath } from "./mediaReferences";

/** The mock engine's installed plugins (what `Resolve::Propose` gets on the mock). */
export const MOCK_INSTALLED_PLUGINS: PluginDescriptor[] = [
  {
    format: "Clap",
    id: MOCK_MEGA_PLUGIN_ID,
    name: "Mock Mega",
    vendor: "Ethereal",
    version: "1.0.0",
    description: "The mock engine's 10,000-parameter plugin",
    features: ["audio-effect"],
    category: "AudioEffect",
    path: MOCK_MEGA_PLUGIN_ID,
    sidechain_inputs: 0,
  },
  {
    format: "Clap",
    id: MOCK_MINI_PLUGIN_ID,
    name: "Mock Mini",
    vendor: "Ethereal",
    version: "1.0.0",
    description: "The mock engine's 4-parameter plugin",
    features: ["audio-effect"],
    category: "AudioEffect",
    path: MOCK_MINI_PLUGIN_ID,
    sidechain_inputs: 0,
  },
];

/** The uninstalled plugin `addMissingPlugin` uses. */
export const MOCK_MISSING_PLUGIN_ID = "dev.ethereal.mock.mega.v1";

export interface ResolverHost {
  project(): Project;
  emit(event: Event): void;
  newId(): string;
  /** Media of the open project that are missing. */
  isMissing(m: MediaRef): boolean;
  libraryHash(rel: string): string;
  /** Upsert devices and media as one undo step. */
  commit(label: string, devices: Device[], media: MediaRef[]): void;
  /** Any edit as one undo step. */
  edit(label: string, f: (tx: Tx) => void): void;
}

/** Never replaced nor a replacement (`Resolve::Replace`): their chains would go. */
const RACKS: ReadonlySet<BuiltinDeviceType> = new Set<BuiltinDeviceType>(["InstrumentRack", "AudioEffectRack", "MidiEffectRack", "DrumRack"]);
const ROLE: Record<DeviceCategory, string> = { Instrument: "an instrument", AudioEffect: "an audio effect", NoteEffect: "a MIDI effect" };

const family = (name: string) =>
  name
    .toLowerCase()
    .replace(/\(.*?\)/g, " ")
    .split(/[^a-z0-9]+/)
    .filter((w) => w && !/^(v?\d{1,2}|x64|x86|vst3?|au|clap)$/.test(w))
    .join(" ");
const vendor = (v: string) => v.toLowerCase().replace(/[^a-z0-9]/g, "");
/** The engine's `normalize::param` (synonyms included). */
const paramKey = (n: string) => {
  const c = n.toLowerCase().replace(/[^a-z0-9]/g, "");
  if (["drywet", "wet", "drywetmix", "mixdrywet", "mix"].includes(c)) return "mix";
  if (["outputgain", "output", "outgain", "gainout"].includes(c)) return "output";
  if (["inputgain", "input", "ingain", "gainin"].includes(c)) return "input";
  return c;
};

/** The engine's `resolver::devices::candidates` (a subset of its keyword table). */
const KEYWORDS: Array<[string[], BuiltinDeviceType, string]> = [
  [["reverb", "hybrid", "verb"], "Reverb", "reverb"],
  [["grain"], "GrainDelay", "grain delay"],
  [["echo", "delay"], "Delay", "delay"],
  [["pedal", "overdrive", "fuzz"], "Pedal", "pedal"],
  [["distortion", "drive", "tube", "amp", "saturator"], "Saturator", "distortion"],
  [["erosion"], "Erosion", "erosion"],
  [["redux", "crush"], "Bitcrusher", "lo-fi"],
  [["filter"], "AutoFilter", "filter"],
  [["chorus"], "Chorus", "chorus"],
  [["compressor", "glue"], "Compressor", "compressor"],
  [["drift", "operator", "wavetable", "synth"], "PolySynth", "synth"],
];

function deviceCandidates(m: MissingDevice): DeviceCandidate[] {
  const words = `${m.description} ${m.source_id}`
    .replace(/([a-z])([A-Z])/g, "$1 $2")
    .toLowerCase()
    .split(/[^a-z0-9]+/);
  const fits = (ty: BuiltinDeviceType) => builtinDescriptor(ty).category === m.role;
  const out: DeviceCandidate[] = [];
  const push = (ty: BuiltinDeviceType, confidence: number, reason: string) => {
    if (!fits(ty) || out.some((c) => c.replacement.type === "Builtin" && c.replacement.device === ty)) return;
    out.push({ replacement: { type: "Builtin", device: ty }, name: builtinDescriptor(ty).name, confidence, reason });
  };
  for (const [keys, ty, what] of KEYWORDS) if (words.some((w) => keys.includes(w))) push(ty, 0.6, `same kind of device: ${what}`);
  push(
    m.role === "Instrument" ? "PolySynth" : m.role === "NoteEffect" ? "Velocity" : "Utility",
    0.3,
    "A built-in device of the same role (its parameters may not match)",
  );
  return out;
}
const baseName = (p: string) => p.slice(p.lastIndexOf("/") + 1).toLowerCase();

export class MockResolver {
  constructor(private host: ResolverHost) {}

  /** Tests / e2e: a device of an uninstalled plugin ("Mock Mega 1") at the end of `track`. */
  addMissingPlugin(track: TrackId): DeviceId {
    const id = this.host.newId() as DeviceId;
    this.host.commit(
      "add device",
      [
        {
          id,
          track,
          order: `z${Object.keys(this.host.project().devices).length}`,
          name: "Mock Mega 1",
          enabled: true,
          kind: {
            type: "Plugin",
            plugin: {
              format: "Clap",
              plugin_id: MOCK_MISSING_PLUGIN_ID,
              name: "Mock Mega 1",
              vendor: "Ethereal",
              version: "0.9",
              sandboxed: false,
              state: null,
            },
          },
          params: { 7: 0.5 },
          sidechain: null,
          pad: null,
        } as Device,
      ],
      [],
    );
    return id;
  }

  /**
   * Tests / e2e: a missing device ("Live's Hybrid Reverb", what an Ableton import leaves) at
   * the end of `track`, its Dry/Wet automated 0.2 → 1 over 4 beats.
   */
  addMissingDevice(track: TrackId): DeviceId {
    const id = this.host.newId() as DeviceId;
    const lane = this.host.newId();
    const missing: MissingDevice = {
      description: "Live's Hybrid Reverb",
      source_id: "ableton:Hybrid",
      origin: "Ableton Live 12.2",
      role: "AudioEffect",
      params: [
        { id: 0, name: "Decay", min: 100, max: 60000 },
        { id: 1, name: "DryWet", min: 0, max: 1 },
      ],
    };
    this.host.edit("add device", (tx) => {
      tx.upsert("Device", {
        id,
        track,
        order: `z${Object.keys(this.host.project().devices).length}`,
        name: missing.description,
        enabled: true,
        kind: { type: "Missing", missing },
        params: { 0: 2400, 1: 0.2 },
        sidechain: null,
        pad: null,
      } as Device);
      tx.upsert("AutomationLane", { id: lane, owner: { type: "Track", track }, target: { type: "DeviceParam", device: id, param: 1 }, enabled: true });
      for (const [time, value] of [
        [0, 0.2],
        [4, 1],
      ] as const) {
        tx.upsert("AutomationPoint", { id: this.host.newId(), lane, time, value, curve: { type: "Linear" } });
      }
    });
    return id;
  }

  command(c: ResolveCommand): ReplyValue {
    switch (c.type) {
      case "Propose":
        return {
          type: "ResolveProposal",
          proposal: { plugins: c.plugins === false ? [] : this.plugins(c.installed), media: this.media(), devices: this.devices(), folder_search: false },
        };
      case "SearchMedia":
        if (c.folder) fail("Unsupported", "searching OS folders needs the desktop engine");
        this.search(c.media ?? null);
        return { type: "Unit" };
      case "CancelSearch":
        return { type: "Unit" };
      case "Apply":
        this.apply(c);
        return { type: "Unit" };
      case "Replace":
        return { type: "PluginReplaced", report: this.replace(c) };
    }
  }

  private devices(): MissingDeviceGroup[] {
    const groups = new Map<string, MissingDeviceGroup>();
    for (const d of Object.values(this.host.project().devices)) {
      if (d.kind.type !== "Missing") continue;
      const m = d.kind.missing;
      const g = groups.get(m.source_id);
      if (g) {
        g.devices.push(d.id);
        continue;
      }
      groups.set(m.source_id, {
        devices: [d.id],
        source_id: m.source_id,
        description: m.description,
        origin: m.origin,
        role: m.role,
        params: m.params.flatMap((p) => (d.params[p.id] !== undefined ? [{ id: p.id, value: d.params[p.id]!, name: p.name }] : [])),
        candidates: deviceCandidates(m),
      });
    }
    return [...groups.values()];
  }

  /** A missing device replaced (`Apply`'s devices): parameters, lanes and mappings by name. */
  private replaceMissing(tx: Tx, d: Device, to: Replacement): string[] {
    if (d.kind.type !== "Missing") fail("InvalidArgument", `device ${d.id} is not a missing device`);
    const m = d.kind.missing;
    const from = missingDescriptor(m);
    let kind: DeviceKind;
    let name: string;
    let target: ParamInfo[] | null;
    if (to.type === "Builtin") {
      const desc = builtinDescriptor(to.device);
      if (desc.category === "Instrument" && this.host.project().tracks[d.track]?.kind !== "Midi") {
        fail("InvalidArgument", `"${d.name}" is on an audio track: an instrument can't replace it`);
      }
      kind = { type: "Builtin", device: newBuiltinDevice(to.device) };
      name = desc.name.toLowerCase(); // default names are lowercase (`resolver::replace`)
      target = desc.params;
    } else {
      kind = { type: "Plugin", plugin: { ...to.plugin, sandboxed: false, state: null } };
      name = to.plugin.name;
      target = mockPluginDescriptor(to.plugin.plugin_id)?.params ?? null;
    }
    const byName = new Map<string, number>();
    for (const x of [...(target ?? [])].reverse()) byName.set(paramKey(x.name), x.id);
    const map = new Map<number, number>();
    const taken = new Set<number>();
    for (const x of from.params) {
      const t = byName.get(paramKey(x.name));
      if (t !== undefined && !taken.has(t)) {
        taken.add(t);
        map.set(x.id, t);
      }
    }
    const info = (list: ParamInfo[] | null, id: number) => list?.find((x) => x.id === id);
    const convert = (v: number, a?: ParamInfo, b?: ParamInfo) => (a && b && a.max !== a.min ? b.min + ((v - a.min) / (a.max - a.min)) * (b.max - b.min) : v);
    const base = to.type === "Builtin" ? defaultParams(newBuiltinDevice(to.device)) : {};
    const params = {
      ...base,
      ...Object.fromEntries(
        [...map].flatMap(([f, t]) => (d.params[f] !== undefined ? [[t, convert(d.params[f]!, info(from.params, f), info(target, t))]] : [])),
      ),
    };
    tx.upsert("Device", { ...d, name: d.name === m.description ? name : d.name, kind, params });
    const lost: string[] = [];
    for (const l of tx.all("AutomationLane")) {
      if (l.target.type !== "DeviceParam" || l.target.device !== d.id) continue;
      const t = map.get(l.target.param);
      if (t !== undefined) {
        tx.upsert("AutomationLane", { ...l, target: { type: "DeviceParam", device: d.id, param: t } });
        continue;
      }
      lost.push(info(from.params, l.target.param)?.name ?? `parameter ${l.target.param}`);
      for (const pt of tx.all("AutomationPoint")) if (pt.lane === l.id) tx.remove("AutomationPoint", pt.id);
      tx.remove("AutomationLane", l.id);
    }
    return lost;
  }

  private replace(c: Extract<ResolveCommand, { type: "Replace" }>): ReplaceReport {
    const p = this.host.project();
    const d = p.devices[c.device] ?? fail("NotFound", `device ${c.device}`);
    const target: Replacement =
      c.plugin && !c.builtin
        ? { type: "Plugin", plugin: c.plugin }
        : c.builtin && !c.plugin
          ? { type: "Builtin", device: c.builtin }
          : fail("InvalidArgument", "replace with either a plugin or a built-in device");
    if (target.type === "Builtin" && RACKS.has(target.device)) fail("InvalidArgument", "a device can't be replaced by a rack or a drum rack");
    if (d.kind.type === "Missing") {
      if (c.preview) return { matched: "ByName", state_kept: false, kept: 0, lost: [], lanes_removed: 0 };
      let lost: string[] = [];
      this.host.edit("replace device", (tx) => {
        lost = this.replaceMissing(tx, d, target);
      });
      return { matched: "ByName", state_kept: false, kept: 0, lost, lanes_removed: lost.length };
    }
    if (d.kind.type === "Builtin" && RACKS.has(d.kind.device.type)) fail("InvalidArgument", `"${d.name}" is a rack: racks can't be replaced`);
    const old = d.kind.type === "Plugin" ? d.kind.plugin : null;
    const oldDesc = d.kind.type === "Builtin" ? builtinDescriptor(d.kind.device) : mockPluginDescriptor(d.kind.plugin.plugin_id);
    const newDesc = target.type === "Builtin" ? builtinDescriptor(target.device) : mockPluginDescriptor(target.plugin.plugin_id);
    if (oldDesc && newDesc && oldDesc.category !== newDesc.category) {
      fail("InvalidArgument", `"${d.name}" is ${ROLE[oldDesc.category]}: ${ROLE[newDesc.category]} can't replace it`);
    }
    if (newDesc?.category === "Instrument" && p.tracks[d.track]?.kind !== "Midi") {
      fail("InvalidArgument", `"${d.name}" is on an audio track: an instrument can't replace it`);
    }
    // The same plugin (any version) or the same built-in type keeps everything.
    const same =
      target.type === "Plugin"
        ? !!old && target.plugin.format === old.format && target.plugin.plugin_id.toLowerCase() === old.plugin_id.toLowerCase()
        : d.kind.type === "Builtin" && d.kind.device.type === target.device;
    // Default names: a built-in's lowercase descriptor name, a plugin's own.
    const oldName = old ? old.name : (oldDesc?.name.toLowerCase() ?? "");
    const newName = target.type === "Plugin" ? target.plugin.name : (newDesc?.name.toLowerCase() ?? "");
    const info = (desc: typeof oldDesc, id: number): ParamInfo | undefined => desc?.params.find((x) => x.id === id);
    const paramOf = (t: AutomationTarget): number | null => (t.type === "DeviceParam" && t.device === d.id ? t.param : null);
    const lanes = Object.values(p.automation_lanes).filter((l) => paramOf(l.target) !== null);
    const mods = Object.values(p.mod_mappings).filter((m) => m.device === d.id);
    const midi = Object.values(p.midi_mappings).filter((m) => m.target.type === "Param" && paramOf(m.target.target) !== null);
    const used = new Set<number>([
      ...lanes.map((l) => paramOf(l.target)!),
      ...mods.map((m) => m.param),
      ...midi.map((m) => (m.target.type === "Param" ? paramOf(m.target.target)! : 0)),
    ]);
    for (const [k, v] of Object.entries(d.params)) {
      const pi = info(oldDesc, Number(k));
      if (!pi || Math.abs(v - pi.default) > 1e-9) used.add(Number(k));
    }
    // `null` = ids kept.
    let matched: ParamMatch;
    let map: Map<number, number> | null;
    const key = (n: string) => n.toLowerCase().replace(/[^a-z0-9]/g, "");
    if (!c.retain) {
      matched = "Reset";
      map = new Map();
    } else if (same) {
      matched = "SameIds";
      map = null;
    } else if (oldDesc && newDesc) {
      matched = "ByName";
      const byName = new Map<string, number>();
      for (const x of [...newDesc.params].reverse()) byName.set(key(x.name), x.id);
      map = new Map();
      const taken = new Set<number>();
      for (const x of oldDesc.params) {
        const t = byName.get(key(x.name));
        if (t !== undefined && !taken.has(t)) {
          taken.add(t);
          map.set(x.id, t);
        }
      }
    } else {
      matched = "Unmatched";
      map = new Map();
    }
    const mapped = (id: number) => map === null || map.has(id);
    const label = (id: number) => info(oldDesc, id)?.name ?? `parameter ${id}`;
    const lost =
      matched === "Reset"
        ? []
        : [...used]
            .filter((id) => !mapped(id))
            .sort((a, b) => a - b)
            .map(label);
    if (lost.length > 8) lost.splice(6, lost.length, `and ${lost.length - 6} more parameters`);
    if (matched !== "SameIds" && matched !== "Reset" && old?.state) lost.unshift("plugin state (presets, loaded samples)");
    // The sidechain source stays when the new device has an input for it.
    const dropSidechain = !!d.sidechain && matched !== "SameIds" && newDesc?.sidechain_inputs === 0;
    if (dropSidechain) lost.push("sidechain input");
    const kind: DeviceKind =
      target.type === "Builtin"
        ? matched === "SameIds"
          ? d.kind
          : { type: "Builtin", device: newBuiltinDevice(target.device) }
        : {
            type: "Plugin",
            plugin: {
              format: target.plugin.format,
              plugin_id: target.plugin.plugin_id,
              name: target.plugin.name,
              vendor: target.plugin.vendor,
              version: target.plugin.version,
              sandboxed: old?.sandboxed ?? false,
              state: matched === "SameIds" ? (old?.state ?? null) : null,
            },
          };
    const report: ReplaceReport = {
      matched,
      state_kept: matched === "SameIds",
      kept: matched === "Reset" ? 0 : [...used].filter(mapped).length,
      lost,
      lanes_removed: lanes.filter((l) => !mapped(paramOf(l.target)!)).length,
    };
    if (c.preview) return report;
    const convert = (v: number, from?: ParamInfo, t?: ParamInfo) =>
      from && t && from.max !== from.min ? t.min + ((v - from.min) / (from.max - from.min)) * (t.max - t.min) : v;
    const params =
      map === null
        ? d.params
        : Object.fromEntries([...map].flatMap(([f, t]) => (d.params[f] !== undefined ? [[t, convert(d.params[f]!, info(oldDesc, f), info(newDesc, t))]] : [])));
    this.host.edit("replace device", (tx) => {
      tx.upsert("Device", {
        ...d,
        name: d.name === oldName && newName ? newName : d.name,
        kind,
        params,
        ...(dropSidechain && { sidechain: null, sidechain_tap: undefined }),
      });
      if (map === null) return;
      for (const l of lanes) {
        const t = map.get(paramOf(l.target)!);
        if (t !== undefined) {
          tx.upsert("AutomationLane", { ...l, target: { type: "DeviceParam", device: d.id, param: t } });
          continue;
        }
        for (const pt of tx.all("AutomationPoint")) if (pt.lane === l.id) tx.remove("AutomationPoint", pt.id);
        tx.remove("AutomationLane", l.id);
      }
      for (const m of mods) {
        const t = map.get(m.param);
        if (t !== undefined) tx.upsert("ModMapping", { ...m, param: t });
        else tx.remove("ModMapping", m.id);
      }
      for (const m of midi) {
        const t = m.target.type === "Param" ? map.get(paramOf(m.target.target)!) : undefined;
        if (t !== undefined) tx.upsert("MidiMapping", { ...m, target: { type: "Param", target: { type: "DeviceParam", device: d.id, param: t } } });
        else tx.remove("MidiMapping", m.id);
      }
    });
    return report;
  }

  private plugins(installed: PluginDescriptor[]): MissingPlugin[] {
    const have = new Set(installed.map((p) => `${p.format}:${p.id}`));
    const groups = new Map<string, MissingPlugin>();
    for (const d of Object.values(this.host.project().devices)) {
      if (d.kind.type !== "Plugin") continue;
      const p = d.kind.plugin;
      const k = `${p.format}:${p.plugin_id}`;
      if (have.has(k)) continue;
      const g = groups.get(k);
      if (g) {
        g.devices.push(d.id);
        continue;
      }
      const candidates: PluginCandidate[] = installed
        .filter((i) => family(i.name) === family(p.name) && (!p.vendor || !i.vendor || vendor(i.vendor) === vendor(p.vendor)))
        .map((i) => ({
          format: i.format,
          id: i.id,
          name: i.name,
          vendor: i.vendor,
          version: i.version,
          rule: "SameFamily",
          confidence: i.format === p.format ? 0.85 : 0.8,
          reason: `same plugin family and vendor: ${p.name} → ${i.name} (another version)`,
          plan: { type: "Map", map: Object.keys(d.params).map((id) => ({ from: Number(id), to: Number(id) })) },
          lost: p.state ? ["plugin state (presets, loaded samples): another plugin can't read it"] : [],
        }));
      groups.set(k, {
        devices: [d.id],
        format: p.format,
        plugin_id: p.plugin_id,
        name: p.name,
        vendor: p.vendor,
        version: p.version,
        has_state: p.state !== null,
        params: Object.entries(d.params).map(([id, value]) => ({ id: Number(id), value, name: null })),
        candidates,
      });
    }
    return [...groups.values()];
  }

  private missingMedia(): MediaRef[] {
    return Object.values(this.host.project().media).filter((m) => this.host.isMissing(m));
  }

  private media() {
    return this.missingMedia().map((m) => ({
      media: m.id,
      name: m.name,
      path: m.location.type === "External" ? m.location.path : null,
      sample_rate: m.sample_rate,
      channels: m.channels,
      frames: m.frames,
      hashed: m.hash !== null,
    }));
  }

  private search(only: string[] | null): void {
    const targets = this.missingMedia().filter((m) => !only || only.includes(m.id));
    for (const m of targets) {
      const candidates: MediaCandidate[] = LIBRARY_FILES.filter((f) => f.kind === "Audio" && baseName(f.path) === baseName(m.name)).map((f) => {
        const same = f.sample_rate === m.sample_rate && f.channels === m.channels;
        const len = Math.abs(f.frames / f.sample_rate - m.frames / m.sample_rate) < 0.05;
        return {
          source: { type: "Location", location: { type: "Library", id: LIBRARY_ID }, path: f.path },
          display: `Library/${f.path}`,
          confidence: this.host.libraryHash(f.path) === m.hash ? 1 : 0.6 + (same ? 0.15 : 0) + (len ? 0.2 : 0),
          reason: this.host.libraryHash(f.path) === m.hash ? "same content" : "same file name",
        };
      });
      this.host.emit({ type: "Resolve", event: { type: "MediaCandidates", media: m.id, candidates } });
    }
    this.host.emit({ type: "Resolve", event: { type: "MediaSearchDone", scanned: 1 } });
  }

  private apply(c: Extract<ResolveCommand, { type: "Apply" }>): void {
    const p = this.host.project();
    const devices: Device[] = [];
    for (const r of c.plugins ?? []) {
      for (const id of r.devices) {
        const d = p.devices[id] ?? fail("NotFound", `device ${id}`);
        if (d.kind.type !== "Plugin") fail("InvalidArgument", `"${d.name}" is not a plugin`);
        const old = d.kind.plugin;
        const keep = r.plan.type === "KeepState";
        const map = r.plan.type === "Map" ? r.plan.map : null;
        const params = map ? Object.fromEntries(map.flatMap((m) => (d.params[m.from] !== undefined ? [[m.to, d.params[m.from]!]] : []))) : d.params;
        devices.push({
          ...d,
          name: d.name === old.name ? r.name : d.name,
          kind: {
            type: "Plugin",
            plugin: { ...old, format: r.format, plugin_id: r.plugin_id, name: r.name, vendor: r.vendor, version: r.version, state: keep ? old.state : null },
          },
          params,
        });
      }
    }
    const media: MediaRef[] = [];
    for (const r of c.media ?? []) {
      const m = p.media[r.media] ?? fail("NotFound", `media ${r.media}`);
      const s = r.source;
      if (s.type !== "Location" || s.location.type !== "Library" || s.location.id !== LIBRARY_ID) {
        fail("Unsupported", "the mock engine relinks to library files only");
      }
      const rel = normalize(s.path);
      if (findLibraryFile(rel)?.kind !== "Audio") fail("NotFound", `file ${s.path}`);
      media.push({ ...m, location: { type: "External", path: libraryPath(rel) }, hash: this.host.libraryHash(rel) });
    }
    const replaced = (c.devices ?? []).flatMap((r) => r.devices.map((id) => [p.devices[id] ?? fail("NotFound", `device ${id}`), r.replacement] as const));
    if (replaced.length > 0) {
      // One undo step for everything.
      this.host.edit("resolve missing items", (tx) => {
        for (const [d, to] of replaced) this.replaceMissing(tx, d, to);
        for (const d of devices) tx.upsert("Device", d);
        for (const m of media) tx.upsert("Media", m);
      });
      return;
    }
    if (devices.length > 0 || media.length > 0) this.host.commit("resolve missing items", devices, media);
  }
}
