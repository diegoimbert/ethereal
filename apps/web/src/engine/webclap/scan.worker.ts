// Runs the install scan (`./scan.ts`) off the page: a plugin that hangs or crashes while
// it is instantiated takes this Worker down, not the app (the page kills it after a timeout).
import { scanModule } from "./scan";
import type { ModuleInfo } from "./wasm";

interface Request {
  id: string;
  module: WebAssembly.Module;
  info: ModuleInfo;
}

const scope = self as unknown as { onmessage: ((e: MessageEvent<Request>) => void) | null; postMessage(m: unknown): void };

scope.onmessage = (e) => {
  try {
    scope.postMessage({ ok: scanModule(e.data.id, e.data.module, e.data.info) });
  } catch (err) {
    scope.postMessage({ error: err instanceof Error ? err.message : String(err) });
  }
};
