/**
 * Mock of `Preset::*` (v0.2, owned by `presets`; CONTRACTS.md §12.5), same rules as the
 * engine (`crates/ether-controller/src/presets`):
 * - factory presets = the real `.etherpreset` files embedded in `ether-devices` (every
 *   device node's folder, picked up by `import.meta.glob`), ids `"<device-key>/<slug>"`;
 * - user presets = an in-memory user library per MockTransport, ids
 *   `"<device-key>/<name>.etherpreset"` (plugins: `plugins/<format>/<id>/…`);
 * - `Load` is one undo step (every descriptor param set: the preset's value clamped, or the
 *   default when missing); plugins get the param mirror (the mock has no plugin state);
 * - `Save`/`Rename`/`Delete`/`SetMeta` are runtime and emit `PresetEvent::Changed`; names
 *   are unique per device type (case-insensitive), `Save` replaces only with `overwrite`.
 */

import type { BuiltinDeviceType, Command, Device, PresetCommand, PresetDevice, PresetInfo, PresetMeta, PresetRef, ReplyValue } from "@/generated";
import { cmd } from "../../cmd";
import { builtinDescriptor, clampParam } from "../builtinDevices";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

/** The fields of an `.etherpreset` file the mock uses. */
interface PresetFileJson {
  format: string;
  preset: {
    name: string;
    device: PresetDevice;
    meta?: Partial<PresetMeta>;
    params?: Record<string, number>;
  };
}

interface StoredPreset {
  name: string;
  device: PresetDevice;
  meta: PresetMeta;
  params: Record<number, number>;
}

const FILES = import.meta.glob<string>("../../../../../crates/ether-devices/presets/*/*.etherpreset", {
  query: "?raw",
  import: "default",
  eager: true,
});

/** `BuiltinDeviceType::key()`: "PolySynth" → "poly-synth". */
export const deviceKey = (t: BuiltinDeviceType) => t.replace(/(?!^)([A-Z])/g, "-$1").toLowerCase();

function parseFile(json: string): StoredPreset | null {
  try {
    const f = JSON.parse(json) as PresetFileJson;
    if (f.format !== "ethereal-preset") return null;
    const params: Record<number, number> = {};
    for (const [k, v] of Object.entries(f.preset.params ?? {})) params[Number(k)] = v;
    return {
      name: f.preset.name,
      device: f.preset.device,
      meta: {
        tags: f.preset.meta?.tags ?? [],
        author: f.preset.meta?.author ?? null,
        description: f.preset.meta?.description ?? null,
      },
      params,
    };
  } catch {
    return null;
  }
}

/** Factory presets by id, parsed once. */
const FACTORY: ReadonlyMap<string, StoredPreset> = new Map(
  Object.entries(FILES).flatMap(([path, json]) => {
    const m = /presets\/([^/]+)\/([^/]+)\.etherpreset$/.exec(path);
    const p = parseFile(json);
    return m && p ? [[`${m[1]}/${m[2]}`, p] as const] : [];
  }),
);

export function sameDevice(a: PresetDevice, b: PresetDevice): boolean {
  if (a.type === "Builtin" && b.type === "Builtin") return a.device === b.device;
  if (a.type === "Plugin" && b.type === "Plugin") return a.format === b.format && a.plugin_id === b.plugin_id;
  return false;
}

function presetDevice(d: Device): PresetDevice {
  if (d.kind.type === "Builtin") return { type: "Builtin", device: d.kind.device.type };
  const { format, plugin_id, name, vendor } = d.kind.plugin;
  return { type: "Plugin", format, plugin_id, name, vendor };
}

function deviceDir(d: PresetDevice): string {
  if (d.type === "Builtin") return deviceKey(d.device);
  return `plugins/${d.format.toLowerCase()}/${d.plugin_id.replace(/[^A-Za-z0-9._-]/g, "_")}`;
}

/** Same file-name rules as the engine (`presets::files::file_stem`). */
export function fileStem(name: string): string {
  const cleaned = [...name]
    .map((c) => (c.charCodeAt(0) < 0x20 || c.charCodeAt(0) === 0x7f || '/\\:*?"<>|'.includes(c) ? "-" : c))
    .slice(0, 80)
    .join("");
  const t = cleaned.trim().replace(/^\.+/, "").replace(/\.+$/, "").trim();
  return t || "Preset";
}

function normalizeMeta(m: PresetMeta): PresetMeta {
  const tags = [...new Set(m.tags.map((t) => t.trim().toLowerCase()).filter((t) => t))].sort();
  const text = (s: string | null) => (s && s.trim() ? s.trim() : null);
  return { tags, author: text(m.author), description: text(m.description) };
}

function matches(p: StoredPreset, text: string | null): boolean {
  const q = text?.trim().toLowerCase();
  if (!q) return true;
  return p.name.toLowerCase().includes(q) || p.meta.tags.some((t) => t.toLowerCase().includes(q)) || (p.meta.author ?? "").toLowerCase().includes(q);
}

const info = (source: PresetRef["source"], id: string, p: StoredPreset): PresetInfo => ({
  preset: { source, id },
  name: p.name,
  device: p.device,
  meta: p.meta,
});

const byName = (a: PresetInfo, b: PresetInfo) => a.name.toLowerCase().localeCompare(b.name.toLowerCase()) || a.preset.id.localeCompare(b.preset.id);

/** The runtime simulation (one per MockTransport). */
export class MockPresets {
  /** The user library: id → preset. */
  private readonly user = new Map<string, StoredPreset>();

  constructor(private readonly host: MockHost) {}

  command(c: PresetCommand): ReplyValue {
    switch (c.type) {
      case "List":
        return { type: "Presets", presets: this.list(c.device, c.text) };
      case "Load":
        this.load(c.device, c.preset);
        return { type: "Unit" };
      case "Save":
        return this.changed({
          type: "Preset",
          preset: this.save(c.device, c.name, c.meta, c.overwrite),
        });
      case "Rename":
        return this.changed({
          type: "Preset",
          preset: this.rename(c.preset, c.name),
        });
      case "Delete":
        this.userPreset(c.preset);
        this.user.delete(c.preset.id);
        return this.changed({ type: "Unit" });
      case "SetMeta": {
        const p = this.userPreset(c.preset);
        p.meta = normalizeMeta(c.meta);
        return this.changed({
          type: "Preset",
          preset: info("User", c.preset.id, p),
        });
      }
    }
  }

  private changed(r: ReplyValue): ReplyValue {
    this.host.emit({ type: "Preset", event: { type: "Changed" } });
    return r;
  }

  private list(device: PresetDevice | null, text: string | null): PresetInfo[] {
    const pick = (p: StoredPreset) => (!device || sameDevice(device, p.device)) && matches(p, text);
    const factory = [...FACTORY].filter(([, p]) => pick(p)).map(([id, p]) => info("Factory", id, p));
    const user = [...this.user].filter(([, p]) => pick(p)).map(([id, p]) => info("User", id, p));
    return [...factory.sort(byName), ...user.sort(byName)];
  }

  private userPreset(ref: PresetRef): StoredPreset {
    if (ref.source !== "User") fail("InvalidArgument", "factory presets are read-only");
    const p = this.user.get(ref.id);
    if (!p) fail("NotFound", `preset ${ref.id}`);
    return p;
  }

  private device(id: string): Device {
    const d = this.host.project().devices[id];
    if (!d) fail("NotFound", `device ${id}`);
    return d;
  }

  private named(device: PresetDevice, name: string, except?: string): string | undefined {
    const n = name.toLowerCase();
    for (const [id, p] of this.user) if (id !== except && sameDevice(p.device, device) && p.name.toLowerCase() === n) return id;
    return undefined;
  }

  private freeId(dir: string, stem: string, except?: string): string {
    for (let n = 1; ; n++) {
      const id = `${dir}/${n === 1 ? stem : `${stem} ${n}`}.etherpreset`;
      const taken = [...this.user.keys()].some((k) => k.toLowerCase() === id.toLowerCase());
      if (!taken || id === except) return id;
    }
  }

  private load(deviceId: string, ref: PresetRef): void {
    const p = ref.source === "Factory" ? FACTORY.get(ref.id) : this.userPreset(ref);
    if (!p) fail("NotFound", `factory preset ${ref.id}`);
    const d = this.device(deviceId);
    if (!sameDevice(presetDevice(d), p.device)) fail("InvalidArgument", `preset "${p.name}" is for another device type`);
    const commands: Command[] = [];
    if (d.kind.type === "Builtin") {
      for (const param of builtinDescriptor(d.kind.device).params) {
        const v = param.id in p.params ? clampParam(param, p.params[param.id]!) : param.default;
        if (d.params[param.id] !== v)
          commands.push(
            cmd("Device", {
              type: "SetParam",
              device: d.id,
              param: param.id,
              value: v,
            }),
          );
      }
    } else {
      for (const [id, v] of Object.entries(p.params))
        commands.push(
          cmd("Device", {
            type: "SetParam",
            device: d.id,
            param: Number(id),
            value: v,
          }),
        );
    }
    if (commands.length > 0) this.host.applyDocument(commands, "Load Preset");
  }

  private save(deviceId: string, rawName: string, meta: PresetMeta, overwrite: boolean): PresetInfo {
    const name = rawName.trim();
    if (!name) fail("InvalidArgument", "a preset needs a name");
    const d = this.device(deviceId);
    const device = presetDevice(d);
    const existing = this.named(device, name);
    if (existing && !overwrite) fail("InvalidArgument", `a preset named "${name}" already exists`);
    const params: Record<number, number> = {};
    if (d.kind.type === "Builtin") {
      for (const param of builtinDescriptor(d.kind.device).params) params[param.id] = d.params[param.id] ?? param.default;
    } else {
      for (const [id, v] of Object.entries(d.params)) if (v !== undefined) params[Number(id)] = v;
    }
    const id = existing ?? this.freeId(deviceDir(device), fileStem(name));
    const p: StoredPreset = { name, device, meta: normalizeMeta(meta), params };
    this.user.set(id, p);
    return info("User", id, p);
  }

  private rename(ref: PresetRef, rawName: string): PresetInfo {
    const name = rawName.trim();
    if (!name) fail("InvalidArgument", "a preset needs a name");
    const p = this.userPreset(ref);
    if (this.named(p.device, name, ref.id)) fail("InvalidArgument", `a preset named "${name}" already exists`);
    const dir = ref.id.includes("/") ? ref.id.slice(0, ref.id.lastIndexOf("/")) : "";
    const id = this.freeId(dir, fileStem(name), ref.id);
    const renamed = { ...p, name };
    this.user.delete(ref.id);
    this.user.set(id, renamed);
    return info("User", id, renamed);
  }
}
