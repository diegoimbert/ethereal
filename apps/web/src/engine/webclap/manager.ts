// The page's side of WebCLAP: installs (from a URL or a file), the installed modules
// (IndexedDB, `./store.ts`), compiling them for the engine realms, and the plugin UIs'
// sandboxed frames. The UI package reaches it as the engine endpoint's `webclap`
// (`@/features/webclap` `WebclapApi`).
//
// Each engine start (`./../endpoint.ts`) attaches to it: the Worklet and the controller
// Worker get every compiled module and the catalog at boot, then each new one.

import type { WebclapApi, WebclapModuleView } from "@/features/webclap/api";
import type { WebclapModuleMsg, WebclapPortMsg } from "../protocol";
import { MAX_BUNDLE_BYTES, contentId, readBundle } from "./bundle";
import { MAX_UI_MESSAGE_BYTES, PLUGIN_PATH } from "./instance";
import type { ScanResult } from "./scan";
import { type InstalledModule, store } from "./store";
import { collectUiPage, dataPage, splitUri, type UiPage } from "./uiPage";
import { moduleInfo, type ModuleInfo } from "./wasm";

/** How long the install scan may take. */
const SCAN_TIMEOUT_MS = 20_000;
/** The sandboxed frame every plugin UI is written into. */
export const UI_LOADER = "/webclap-ui.html";

interface Compiled {
  module: WebAssembly.Module;
  info: ModuleInfo;
  files: Map<string, Uint8Array>;
}

/** The engine realms of the running engine. */
export interface EngineLink {
  /** The Worklet (the engine node's port). */
  worklet: MessagePort;
  /** The controller Worker. */
  controller: Worker;
}

function scanInWorker(id: string, module: WebAssembly.Module, info: ModuleInfo): Promise<ScanResult> {
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./scan.worker.ts", import.meta.url), { type: "module", name: "ether-webclap-scan" });
    const timer = setTimeout(() => {
      worker.terminate();
      reject(new Error("the plugin took too long to load"));
    }, SCAN_TIMEOUT_MS);
    worker.onmessage = (e: MessageEvent<{ ok?: ScanResult; error?: string }>) => {
      clearTimeout(timer);
      worker.terminate();
      if (e.data.ok) resolve(e.data.ok);
      else reject(new Error(e.data.error ?? "the scan failed"));
    };
    worker.onerror = (e) => {
      clearTimeout(timer);
      worker.terminate();
      reject(new Error(e.message || "the plugin crashed while loading"));
    };
    worker.postMessage({ id, module, info });
  });
}

function view(m: InstalledModule): WebclapModuleView {
  return {
    id: m.id,
    name: m.name,
    source: m.source,
    size: m.bytes.byteLength,
    installedAt: m.installedAt,
    plugins: m.scan.plugins.map((p) => ({
      id: p.id,
      name: p.name,
      vendor: p.vendor,
      version: p.version,
      features: p.features,
      webview: p.webview,
    })),
    failed: m.scan.failed,
  };
}

export class WebclapManager implements WebclapApi {
  private records: InstalledModule[] = [];
  private readonly compiled = new Map<string, Compiled>();
  private loaded: Promise<void> | null = null;
  private readonly listeners = new Set<() => void>();
  private link: EngineLink | null = null;
  private nextReq = 1;
  private readonly requests = new Map<number, (msg: WebclapPortMsg) => void>();
  private readonly frames = new Map<string, HTMLIFrameElement>();
  /** The latest `mountUi` of each device: an older one (React remounts) neither shows nor closes it. */
  private readonly mounts = new Map<string, number>();
  private mountSeq = 0;
  private views: WebclapModuleView[] = [];

  /** Read and compile the installed modules (once). A broken one is skipped, not fatal. */
  load(): Promise<void> {
    this.loaded ??= (async () => {
      let records: InstalledModule[] = [];
      try {
        records = await store.list();
      } catch (e) {
        console.warn("webclap: the installed plugins could not be read", e);
      }
      for (const r of records) {
        try {
          await this.compile(r);
          this.records.push(r);
        } catch (e) {
          console.warn(`webclap: ${r.name} could not be loaded`, e);
        }
      }
      this.changed();
    })();
    return this.loaded;
  }

  private async compile(r: InstalledModule): Promise<Compiled> {
    const bundle = await readBundle(new Uint8Array(r.bytes));
    const c: Compiled = { module: await WebAssembly.compile(bundle.wasm as BufferSource), info: r.info, files: bundle.files };
    this.compiled.set(r.id, c);
    return c;
  }

  /** The scanned modules, as `crates/ether-wasm/src/webclap/catalog.rs` reads them. */
  catalogJson(): string {
    return JSON.stringify(this.records.map((r) => ({ module: r.id, plugins: r.scan.plugins })));
  }

  /** Every compiled module (for the engine realms at boot). */
  moduleMessages(): Omit<WebclapModuleMsg, "type">[] {
    return this.records.flatMap((r) => {
      const c = this.compiled.get(r.id);
      return c ? [{ id: r.id, module: c.module, info: c.info }] : [];
    });
  }

  /** A (re)started engine: modules and catalog go to it from now on. */
  attach(link: EngineLink | null): void {
    this.link = link;
    link?.worklet.addEventListener("message", (e: MessageEvent<WebclapPortMsg>) => this.fromWorklet(e.data));
  }

  private fromWorklet(msg: WebclapPortMsg): void {
    switch (msg.type) {
      case "webclap:ui":
        this.frames.get(msg.device)?.contentWindow?.postMessage(msg.data, "*", [msg.data]);
        break;
      case "webclap:opened":
      case "webclap:resourced":
        this.requests.get(msg.req)?.(msg);
        this.requests.delete(msg.req);
        break;
      default:
        break;
    }
  }

  private ask<T extends WebclapPortMsg>(msg: WebclapPortMsg & { req: number }): Promise<T> {
    const link = this.link;
    if (!link) return Promise.reject(new Error("the audio engine is not running"));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.requests.delete(msg.req);
        reject(new Error("the plugin did not answer (is the audio running?)"));
      }, 5000);
      this.requests.set(msg.req, (m) => {
        clearTimeout(timer);
        resolve(m as T);
      });
      link.worklet.postMessage(msg);
    });
  }

  private changed(): void {
    this.views = this.records.map(view);
    for (const l of this.listeners) l();
  }

  // ─── WebclapApi ─────────────────────────────────────────────────────────────────────

  modules(): WebclapModuleView[] {
    return this.views;
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  hasUi(pluginId: string): boolean {
    for (let i = this.records.length - 1; i >= 0; i--) {
      const p = this.records[i]!.scan.plugins.find((x) => x.id === pluginId);
      if (p) return p.webview;
    }
    return false;
  }

  async installUrl(url: string): Promise<WebclapModuleView> {
    let u: URL;
    try {
      u = new URL(url);
    } catch {
      throw new Error("not a valid address");
    }
    if (u.protocol !== "https:" && u.hostname !== "localhost" && u.hostname !== "127.0.0.1") throw new Error("only https addresses can be installed");
    const res = await fetch(u, { credentials: "omit", referrerPolicy: "no-referrer", cache: "no-cache" });
    if (!res.ok) throw new Error(`the download failed (${res.status})`);
    const bytes = new Uint8Array(await res.arrayBuffer());
    const name = decodeURIComponent(u.pathname.split("/").filter(Boolean).pop() ?? u.hostname);
    return this.install(bytes, name, u.href);
  }

  async installFile(file: File): Promise<WebclapModuleView> {
    if (file.size > MAX_BUNDLE_BYTES) throw new Error("the file is too big");
    return this.install(new Uint8Array(await file.arrayBuffer()), file.name, `file:${file.name}`);
  }

  private async install(bytes: Uint8Array, name: string, source: string): Promise<WebclapModuleView> {
    await this.load();
    const bundle = await readBundle(bytes);
    const id = await contentId(bytes);
    const info = moduleInfo(bundle.wasm);
    const module = await WebAssembly.compile(bundle.wasm as BufferSource);
    const scan = await scanInWorker(id, module, info);
    if (scan.plugins.length === 0) throw new Error(scan.failed[0]?.error ?? "the file has no plugins");
    const record: InstalledModule = { id, name, source, installedAt: Date.now(), bytes: bytes.slice().buffer as ArrayBuffer, info, scan };
    await store.put(record);
    this.compiled.set(id, { module, info, files: bundle.files });
    this.records = [...this.records.filter((r) => r.id !== id), record];
    this.publish({ id, module, info });
    this.changed();
    return view(record);
  }

  async remove(id: string): Promise<void> {
    await this.load();
    await store.remove(id);
    this.records = this.records.filter((r) => r.id !== id);
    this.publish(null);
    this.changed();
  }

  /** A new module (or none) and the new catalog, to the running engine. */
  private publish(added: Omit<WebclapModuleMsg, "type"> | null): void {
    const link = this.link;
    if (!link) return;
    if (added) {
      const msg: WebclapModuleMsg = { type: "webclap:module", ...added };
      link.worklet.postMessage(msg);
      link.controller.postMessage(msg);
    }
    link.controller.postMessage({ type: "webclap:catalog", json: this.catalogJson() });
  }

  /** A file of the UI of `device`'s plugin: its bundle's, else `get_resource`'s. */
  private async resource(device: string, files: Map<string, Uint8Array> | null, path: string) {
    const rel = path.replace(/^\/+/, "");
    const inBundle = files?.get(rel) ?? (rel.startsWith(PLUGIN_PATH.slice(1) + "/") ? files?.get(rel.slice(PLUGIN_PATH.length)) : undefined);
    if (inBundle) return { mime: mimeOf(rel), bytes: inBundle };
    const r = await this.ask<Extract<WebclapPortMsg, { type: "webclap:resourced" }>>({ type: "webclap:resource", device, path, req: this.nextReq++ });
    return r.data ? { mime: r.mime ?? mimeOf(rel), bytes: new Uint8Array(r.data) } : null;
  }

  async mountUi(device: string, pluginId: string, iframe: HTMLIFrameElement): Promise<() => void> {
    const token = ++this.mountSeq;
    this.mounts.set(device, token);
    const current = () => this.mounts.get(device) === token;
    const opened = await this.ask<Extract<WebclapPortMsg, { type: "webclap:opened" }>>({ type: "webclap:open", device, req: this.nextReq++ });
    if (opened.error || !opened.uri) throw new Error(opened.error ?? "the plugin has no ui");
    const files = this.filesOf(pluginId);
    let uri = opened.uri;
    if (uri.startsWith("file:")) uri = uri.replace(/^file:\/*/, "/");
    let page: UiPage;
    if (uri.startsWith("data:text/html")) {
      page = dataPage(decodeDataUri(uri));
    } else if (uri.startsWith("/")) {
      const { path, query } = splitUri(uri);
      // `file:` paths are inside the bundle, under the path `clap_entry.init` got.
      const local = path.startsWith(PLUGIN_PATH + "/") ? path.slice(PLUGIN_PATH.length) : path;
      page = await collectUiPage(local, query, (p) => this.resource(device, files, p));
    } else {
      throw new Error("this plugin's ui is a web address: not supported (sandboxing)");
    }
    // Superseded meanwhile (the frame remounted): the newer mount shows it.
    if (!current()) return () => {};
    const onMessage = (e: MessageEvent) => {
      if (e.source !== iframe.contentWindow || !iframe.contentWindow) return;
      const d: unknown = e.data;
      if (d && typeof d === "object" && (d as { type?: string }).type === "ether:webclap-loader") {
        // Copies (a remount sends them again).
        iframe.contentWindow.postMessage({ type: "ether:webclap-load", page }, "*");
        return;
      }
      let data: ArrayBuffer | null = null;
      // Too big for the plugin (`receive` would drop it): not copied nor sent.
      if ((d instanceof ArrayBuffer || ArrayBuffer.isView(d)) && d.byteLength > MAX_UI_MESSAGE_BYTES) return;
      if (d instanceof ArrayBuffer) data = d;
      else if (ArrayBuffer.isView(d)) data = new Uint8Array(d.buffer, d.byteOffset, d.byteLength).slice().buffer;
      if (data) this.link?.worklet.postMessage({ type: "webclap:ui", device, data } satisfies WebclapPortMsg, [data]);
    };
    window.addEventListener("message", onMessage);
    this.frames.set(device, iframe);
    iframe.setAttribute("sandbox", "allow-scripts");
    iframe.setAttribute("referrerpolicy", "no-referrer");
    iframe.setAttribute("allow", "");
    iframe.src = UI_LOADER;
    return () => {
      window.removeEventListener("message", onMessage);
      if (!current()) return;
      this.mounts.delete(device);
      this.frames.delete(device);
      this.link?.worklet.postMessage({ type: "webclap:close", device } satisfies WebclapPortMsg);
    };
  }

  private filesOf(pluginId: string): Map<string, Uint8Array> | null {
    for (let i = this.records.length - 1; i >= 0; i--) {
      const r = this.records[i]!;
      if (r.scan.plugins.some((p) => p.id === pluginId)) return this.compiled.get(r.id)?.files ?? null;
    }
    return null;
  }
}

/** The text of a `data:` URI (not fetched: the app's policy doesn't connect to `data:`). */
export function decodeDataUri(uri: string): string {
  const comma = uri.indexOf(",");
  const meta = uri.slice(5, comma);
  const payload = uri.slice(comma + 1);
  if (/;base64$/i.test(meta)) {
    const bin = atob(payload);
    return new TextDecoder().decode(Uint8Array.from(bin, (c) => c.charCodeAt(0)));
  }
  return decodeURIComponent(payload);
}

function mimeOf(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase();
  const types: Record<string, string> = {
    html: "text/html",
    htm: "text/html",
    js: "text/javascript",
    mjs: "text/javascript",
    css: "text/css",
    json: "application/json",
    svg: "image/svg+xml",
    png: "image/png",
    jpg: "image/jpeg",
    jpeg: "image/jpeg",
    gif: "image/gif",
    webp: "image/webp",
    woff: "font/woff",
    woff2: "font/woff2",
    wasm: "application/wasm",
  };
  return types[ext ?? ""] ?? "application/octet-stream";
}

/** The page's one manager. */
export const webclap = new WebclapManager();
