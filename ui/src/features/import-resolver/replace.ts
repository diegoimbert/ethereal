/**
 * "Replace plugin…" (owner request: swap any plugin device's plugin, not only a missing one;
 * CONTRACTS.md §14.2 `Resolve::Replace`). The dialog picks an installed plugin (the
 * resolver's catalog) and asks the engine for a preview of what is kept (`preview: true`)
 * whenever the choice or "Retain automations and parameters" changes; `applyReplace` sends
 * the real `Resolve::Replace` (one undo step).
 */
import { create } from "zustand";
import type { DeviceId, PluginDescriptor, ReplaceReport, ReplacementPlugin } from "@/generated";
import { cmd, commandErrorText, isCommandFailed, type EngineTransport } from "@/transport";
import { loadCatalog } from "./catalog";

interface ReplaceState {
  open: boolean;
  device: DeviceId | null;
  /** Installed plugins (`null` = this host has no plugin catalog). */
  installed: PluginDescriptor[] | null;
  /** The chosen plugin, by `format:id`. */
  choice: string | null;
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

function chosen(s: Pick<ReplaceState, "installed" | "choice">): PluginDescriptor | null {
  return s.installed?.find((p) => pluginChoiceKey(p) === s.choice) ?? null;
}

function target(p: PluginDescriptor): ReplacementPlugin {
  return { format: p.format, plugin_id: p.id, name: p.name, vendor: p.vendor, version: p.version };
}

/** Open the dialog for `device` (any plugin device). */
export async function openReplace(device: DeviceId, transport: EngineTransport | null = current): Promise<void> {
  if (!transport) return;
  previewSeq++;
  useReplace.setState({ ...INITIAL, open: true, device });
  try {
    const installed = await loadCatalog(transport);
    if (useReplace.getState().device === device) useReplace.setState({ installed: installed ?? [] });
  } catch (e) {
    useReplace.setState({ installed: [], error: errorText(e) });
  }
}

export function closeReplace(): void {
  previewSeq++;
  useReplace.setState({ ...INITIAL });
}

async function preview(transport: EngineTransport): Promise<void> {
  const s = useReplace.getState();
  const p = chosen(s);
  if (!s.device || !p) {
    useReplace.setState({ report: null, previewing: false });
    return;
  }
  const seq = ++previewSeq;
  useReplace.setState({ previewing: true, error: null });
  try {
    const reply = await transport.send(cmd("Resolve", { type: "Replace", device: s.device, plugin: target(p), retain: s.retain, preview: true }));
    if (seq !== previewSeq) return;
    if (reply.type !== "PluginReplaced") throw new Error("unexpected reply");
    useReplace.setState({ report: reply.report, previewing: false });
  } catch (e) {
    if (seq !== previewSeq) return;
    useReplace.setState({ report: null, previewing: false, error: errorText(e) });
  }
}

/** Choose the replacement (by `format:id`); the preview follows. */
export function chooseReplacement(choice: string | null, transport: EngineTransport | null = current): Promise<void> {
  useReplace.setState({ choice, report: null });
  return transport ? preview(transport) : Promise.resolve();
}

export function setRetain(retain: boolean, transport: EngineTransport | null = current): Promise<void> {
  useReplace.setState({ retain, report: null });
  return transport ? preview(transport) : Promise.resolve();
}

/** Replace the plugin (one undo step). `true` when done. */
export async function applyReplace(transport: EngineTransport | null = current): Promise<boolean> {
  const s = useReplace.getState();
  const p = chosen(s);
  if (!transport || !s.device || !p) return false;
  previewSeq++;
  useReplace.setState({ applying: true, error: null });
  try {
    await transport.send(cmd("Resolve", { type: "Replace", device: s.device, plugin: target(p), retain: s.retain }));
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
