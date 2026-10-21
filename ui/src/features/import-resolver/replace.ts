/**
 * "Replace device…" (owner requests: swap any plugin device's plugin, not only a missing
 * one; then "it should rather just be replace device and not limited to plugins": any
 * device but a rack, factory or plugin, for another of the same role; then "not in a modal:
 * a popover, as if we were selecting a device"; CONTRACTS.md §14.2 `Resolve::Replace`). The
 * popover at the device's header is the device library (factory devices and the resolver's
 * catalog of installed plugins) with "Retain automations and parameters": a row replaces
 * the device at once (one undo step); what couldn't be kept is said in a toast.
 */
import { create } from "zustand";
import type { BuiltinDeviceType, Device, DeviceId, PluginDescriptor, ReplaceReport, Replacement } from "@/generated";
import type { LibraryDevicePayload } from "@/features/devices/libraryDevice";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { loadCatalog } from "./catalog";
import { notify } from "@/features/notifications";
import { t } from "@/i18n";

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
  /** Retain automations and parameters (kept across uses). */
  retain: boolean;
  applying: boolean;
}

const INITIAL: Omit<ReplaceState, "retain"> = {
  open: false,
  device: null,
  installed: null,
  loading: false,
  applying: false,
};

export const useReplace = create<ReplaceState>()(() => ({ ...INITIAL, retain: true }));

let current: EngineTransport | null = null;

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

/** Open the popover for `device` (any device but a rack). */
export async function openReplace(device: DeviceId, transport: EngineTransport | null = current): Promise<void> {
  if (!transport) return;
  useReplace.setState({ ...INITIAL, open: true, device, loading: true });
  try {
    const installed = await loadCatalog(transport);
    if (useReplace.getState().device === device) useReplace.setState({ installed, loading: false });
  } catch {
    // No catalog: factory devices only.
    if (useReplace.getState().device === device) useReplace.setState({ installed: null, loading: false });
  }
}

export function closeReplace(): void {
  useReplace.setState({ ...INITIAL });
}

export function setRetain(retain: boolean): void {
  useReplace.setState({ retain });
}

/**
 * Replace the device with `choice` (one undo step) and close; says in a toast what couldn't
 * be kept, or why it failed. `true` when done.
 */
export async function replaceWith(choice: ReplaceChoice, transport: EngineTransport | null = current): Promise<boolean> {
  const s = useReplace.getState();
  if (!transport || !s.device || s.applying) return false;
  useReplace.setState({ applying: true });
  try {
    const reply = await transport.send(cmd("Resolve", { type: "Replace", device: s.device, ...targetOf(choice.to), retain: s.retain }));
    closeReplace();
    if (reply.type === "PluginReplaced") reportLosses(choice.name, reply.report);
    return true;
  } catch (e) {
    useReplace.setState({ applying: false });
    notify("Error", t("resolver.replace.failed", { name: choice.name, error: errorText(e) }));
    return false;
  }
}

/** What a replace couldn't keep, as an info toast (nothing when everything was kept). */
function reportLosses(name: string, report: ReplaceReport): void {
  const parts = [
    report.lost.length > 0 ? t("resolver.replace.done_lost", { name, lost: report.lost.join(", ") }) : "",
    report.lanes_removed > 0 ? t("resolver.replace.done_lanes", { count: report.lanes_removed }) : "",
  ].filter(Boolean);
  if (parts.length) notify("Info", parts.join(" "));
}

/** Tests. */
export function resetReplace(): void {
  useReplace.setState({ ...INITIAL, retain: true });
}
