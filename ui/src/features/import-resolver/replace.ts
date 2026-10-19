/**
 * "Replace device…" (owner requests: swap any plugin device's plugin, not only a missing
 * one; then "it should rather just be replace device and not limited to plugins": any
 * device but a rack, factory or plugin, for another of the same role; CONTRACTS.md §14.2
 * `Resolve::Replace`). The dialog picks a factory device or an installed plugin (the
 * resolver's catalog) and asks the engine for a preview of what is kept (`preview: true`)
 * whenever the choice or "Retain automations and parameters" changes; `applyReplace` sends
 * the real `Resolve::Replace` (one undo step).
 */
import { create } from "zustand";
import type { BuiltinDeviceType, Device, DeviceId, PluginDescriptor, ReplaceReport, Replacement } from "@/generated";
import type { LibraryDevicePayload } from "@/features/devices/libraryDevice";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { loadCatalog } from "./catalog";

/** The chosen replacement, with what the dialog shows for it. */
export interface ReplaceChoice {
  /** `builtin:<type>` or `format:id`. */
  key: string;
  name: string;
  to: Replacement;
  /** A plugin's catalog entry (vendor, format, version). */
  plugin?: PluginDescriptor;
}

interface ReplaceState {
  open: boolean;
  device: DeviceId | null;
  /** Installed plugins (`null` = this host has no plugin catalog: factory devices only). */
  installed: PluginDescriptor[] | null;
  /** The installed plugins are still loading. */
  loading: boolean;
  choice: ReplaceChoice | null;
  retain: boolean;
  /** What the engine would keep with the current choice (`null` = not asked yet). */
  report: ReplaceReport | null;
  previewing: boolean;
  applying: boolean;
  error: string | null;
}

const INITIAL: ReplaceState = {
  open: false,
  device: null,
  installed: null,
  loading: false,
  choice: null,
  retain: true,
  report: null,
  previewing: false,
  applying: false,
  error: null,
};

export const useReplace = create<ReplaceState>()(() => ({ ...INITIAL }));

let current: EngineTransport | null = null;
/** Only the latest preview request updates the dialog. */
let previewSeq = 0;

export function setReplaceTransport(t: EngineTransport | null): void {
  current = t;
}

export const pluginChoiceKey = (p: { format: string; id: string }) => `${p.format}:${p.id}`;

function errorText(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

/** A row picked in the device library as a replacement (`null`: a plugin no longer listed). */
export function choiceOf(pick: LibraryDevicePayload, installed: ReadonlyArray<PluginDescriptor> | null): ReplaceChoice | null {
  const d = pick.device;
  if (d.type === "Builtin") return { key: `builtin:${d.builtin}`, name: pick.name, to: { type: "Builtin", device: d.builtin } };
  const p = installed?.find((x) => x.format === d.format && x.id === d.plugin_id && (!d.name || x.name === d.name));
  if (!p) return null;
  return {
    key: pluginChoiceKey(p),
    name: p.name,
    plugin: p,
    to: { type: "Plugin", plugin: { format: p.format, plugin_id: p.id, name: p.name, vendor: p.vendor, version: p.version } },
  };
}

/** Racks hold chains (or pads) that would go with them: never replaced nor a replacement. */
const RACKS: ReadonlySet<BuiltinDeviceType> = new Set<BuiltinDeviceType>(["InstrumentRack", "AudioEffectRack", "MidiEffectRack", "DrumRack"]);
export const isRackDevice = (type: BuiltinDeviceType): boolean => RACKS.has(type);

/**
 * Whether a library row can replace `device` (its role is the picker's category filter):
 * never a rack, nor the device itself (the same factory type, or the same plugin in the
 * same format; another version or format of it is offered).
 */
export function acceptReplacement(device: Device, pick: LibraryDevicePayload): boolean {
  const d = pick.device;
  if (d.type === "Builtin") return !isRackDevice(d.builtin) && !(device.kind.type === "Builtin" && device.kind.device.type === d.builtin);
  return !(device.kind.type === "Plugin" && device.kind.plugin.format === d.format && device.kind.plugin.plugin_id === d.plugin_id);
}

/** `Resolve::Replace`'s target fields. */
const targetOf = (to: Replacement) => (to.type === "Builtin" ? { builtin: to.device } : { plugin: to.plugin });

/** Open the dialog for `device` (any device but a rack). */
export async function openReplace(device: DeviceId, transport: EngineTransport | null = current): Promise<void> {
  if (!transport) return;
  previewSeq++;
  useReplace.setState({ ...INITIAL, open: true, device, loading: true });
  try {
    const installed = await loadCatalog(transport);
    if (useReplace.getState().device === device) useReplace.setState({ installed, loading: false });
  } catch (e) {
    useReplace.setState({ installed: null, loading: false, error: errorText(e) });
  }
}

export function closeReplace(): void {
  previewSeq++;
  useReplace.setState({ ...INITIAL });
}

async function preview(transport: EngineTransport): Promise<void> {
  const s = useReplace.getState();
  if (!s.device || !s.choice) {
    useReplace.setState({ report: null, previewing: false });
    return;
  }
  const seq = ++previewSeq;
  useReplace.setState({ previewing: true, error: null });
  try {
    const reply = await transport.send(cmd("Resolve", { type: "Replace", device: s.device, ...targetOf(s.choice.to), retain: s.retain, preview: true }));
    if (seq !== previewSeq) return;
    if (reply.type !== "PluginReplaced") throw new Error("unexpected reply");
    useReplace.setState({ report: reply.report, previewing: false });
  } catch (e) {
    if (seq !== previewSeq) return;
    useReplace.setState({ report: null, previewing: false, error: errorText(e) });
  }
}

/** Choose the replacement; the preview follows. */
export function chooseReplacement(choice: ReplaceChoice | null, transport: EngineTransport | null = current): Promise<void> {
  useReplace.setState({ choice, report: null });
  return transport ? preview(transport) : Promise.resolve();
}

export function setRetain(retain: boolean, transport: EngineTransport | null = current): Promise<void> {
  useReplace.setState({ retain, report: null });
  return transport ? preview(transport) : Promise.resolve();
}

/** Replace the device (one undo step). `true` when done. */
export async function applyReplace(transport: EngineTransport | null = current): Promise<boolean> {
  const s = useReplace.getState();
  if (!transport || !s.device || !s.choice) return false;
  previewSeq++;
  useReplace.setState({ applying: true, error: null });
  try {
    await transport.send(cmd("Resolve", { type: "Replace", device: s.device, ...targetOf(s.choice.to), retain: s.retain }));
    useReplace.setState({ ...INITIAL });
    return true;
  } catch (e) {
    useReplace.setState({ applying: false, error: errorText(e) });
    return false;
  }
}

/** Tests. */
export function resetReplace(): void {
  previewSeq++;
  useReplace.setState({ ...INITIAL });
}
