// The install scan: instantiate each plugin of a module once and read its CLAP metadata
// (the shape crates/ether-wasm/src/webclap/catalog.rs `ScannedModule` parses). Runs in a
// Worker of its own (`./scan.worker.ts`), killed when it takes too long.

import { ClapInstance, listPlugins, type ParamInfo, type PortInfo } from "./instance";
import type { ModuleInfo } from "./wasm";

export interface ScannedPlugin {
  id: string;
  name: string;
  vendor: string;
  version: string;
  description: string;
  features: string[];
  params: ParamInfo[];
  audioInputs: PortInfo[] | null;
  audioOutputs: PortInfo[] | null;
  noteInputs: number;
  noteExpressions: boolean;
  webview: boolean;
}

export interface ScannedModule {
  module: string;
  plugins: ScannedPlugin[];
}

export interface ScanResult extends ScannedModule {
  /** Plugins listed by the factory that failed to instantiate, with why. */
  failed: { id: string; error: string }[];
}

/** Read every plugin of `module` (`id`: the module's content hash). */
export function scanModule(id: string, module: WebAssembly.Module, info: ModuleInfo): ScanResult {
  const out: ScanResult = { module: id, plugins: [], failed: [] };
  for (const d of listPlugins(module, info)) {
    let inst: ClapInstance | null = null;
    try {
      inst = new ClapInstance(module, info, d.id, { sampleRate: 48_000, maxFrames: 128, activate: false });
      const notes = inst.noteInputs();
      out.plugins.push({
        id: d.id,
        name: d.name || d.id,
        vendor: d.vendor,
        version: d.version,
        description: d.description,
        features: d.features.map((f) => f.toLowerCase()),
        params: inst.paramInfos(),
        audioInputs: inst.audioPorts(true),
        audioOutputs: inst.audioPorts(false),
        noteInputs: notes.count,
        noteExpressions: notes.clapDialect,
        webview: inst.hasWebview,
      });
    } catch (e) {
      out.failed.push({ id: d.id, error: e instanceof Error ? e.message : String(e) });
    } finally {
      inst?.destroy();
    }
  }
  return out;
}
