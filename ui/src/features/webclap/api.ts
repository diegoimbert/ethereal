/**
 * WebCLAP plugins in the web version: what the host app (apps/web, `WebclapManager`)
 * offers the UI through the engine endpoint (`WasmTransport.webclap`). Installing reads and
 * scans the module in the browser; the plugins run next to the engine (docs/PLUGIN-FORMATS.md
 * "WebCLAP").
 */

/** A plugin of an installed module. */
export interface WebclapPluginView {
  id: string;
  name: string;
  vendor: string;
  version: string;
  features: string[];
  /** Has its own web ui (`clap.webview`). */
  webview: boolean;
}

/** An installed module (a `.wasm` or a `.wclap.tar.gz` bundle). */
export interface WebclapModuleView {
  id: string;
  /** The file's name. */
  name: string;
  /** The address it was installed from, or `file:<name>`. */
  source: string;
  size: number;
  installedAt: number;
  plugins: WebclapPluginView[];
  /** Plugins of the module that failed to load at install. */
  failed: { id: string; error: string }[];
}

export interface WebclapApi {
  /** The installed modules (a stable array until the next change). */
  modules(): WebclapModuleView[];
  subscribe(listener: () => void): () => void;
  installUrl(url: string): Promise<WebclapModuleView>;
  installFile(file: File): Promise<WebclapModuleView>;
  remove(id: string): Promise<void>;
  /** Whether the installed plugin `pluginId` has its own web ui. */
  hasUi(pluginId: string): boolean;
  /**
   * Show the web ui of the running plugin of `device` in `iframe` (sandboxed by the host:
   * no access to the app). Resolves to the function that closes it.
   */
  mountUi(device: string, pluginId: string, iframe: HTMLIFrameElement): Promise<() => void>;
}
