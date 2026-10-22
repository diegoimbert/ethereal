// Controller Worker: runs `ether-controller` (wasm instance #1). Receives JSON
// `ClientMessage`s from the UI, talks to the AudioWorklet over the SAB rings (inside Rust),
// persists projects in OPFS through `SyncFs`, and ticks at ~60 Hz to forward playhead and
// meters.
//
// An extension's worker module (`@/ext` "web.engine" `controllerWorker`) imports this one and
// calls `extendControllerWorker` (before the init message arrives) for its own messages.
import { initSync, WasmController } from "@ether-wasm/ether_wasm.js";
import type { FromController, ToController } from "./protocol";
import { SyncFs } from "./syncFs";
import { WebclapHost } from "./webclap/host";

interface Scope {
  onmessage: ((e: MessageEvent<ToController>) => void) | null;
  postMessage(msg: FromController): void;
}
const scope = self as unknown as Scope;

const TICK_MS = 16;

/** An extension of the controller Worker (`extendControllerWorker`). */
export interface ControllerWorkerExtension {
  /** The init message (with the extension's own fields), once the controller exists. */
  onInit?(init: Extract<ToController, { type: "init" }>, controller: WasmController): void;
  /** One of its own messages (`ext:*`): `post` answers the page. */
  onMessage?(msg: { type: string; [field: string]: unknown }, post: (msg: FromController) => void): void;
}

const extensions: ControllerWorkerExtension[] = [];

/** Add an extension (call it from the module that imports this one, at load). */
export function extendControllerWorker(ext: ControllerWorkerExtension): void {
  extensions.push(ext);
}

interface HttpRequest {
  id: number;
  url: string;
  max: number;
}

/**
 * Run the sample pack's HTTP GETs the controller asked for (`HostServices::http_get`)
 * with `fetch`, streaming the body with progress and the size cap, and hand the answers back.
 */
function runHttp(ctl: WasmController): void {
  const requests = JSON.parse(ctl.take_http_requests()) as HttpRequest[];
  for (const r of requests) void fetchOne(ctl, r);
}

async function fetchOne(ctl: WasmController, r: HttpRequest): Promise<void> {
  const fail = (e: unknown): void => guarded(() => ctl.http_done(r.id, 0, new Uint8Array(), e instanceof Error ? e.message : String(e)));
  try {
    const resp = await fetch(r.url, { cache: "no-cache" });
    const header = resp.headers.get("content-length");
    const total = header ? Number(header) : -1;
    if (total > r.max) return fail("the download is bigger than expected");
    const reader = resp.body?.getReader();
    const parts: Uint8Array[] = [];
    let received = 0;
    while (reader) {
      const { done, value } = await reader.read();
      if (done) break;
      received += value.length;
      if (received > r.max) {
        await reader.cancel();
        return fail("the download is bigger than expected");
      }
      parts.push(value);
      guarded(() => ctl.http_progress(r.id, received, total));
    }
    const body = new Uint8Array(received);
    let at = 0;
    for (const p of parts) {
      body.set(p, at);
      at += p.length;
    }
    guarded(() => ctl.http_done(r.id, resp.status, body, null));
  } catch (e) {
    fail(e);
  }
}
let controller: WasmController | null = null;
/** WebCLAP instances of offline renders (exports, freezes) run here, in this Worker. */
let webclap: WebclapHost | null = null;
let dead = false;
/** `client` messages handled (shared with the page, see `ToController` init). */
let handled: Int32Array | null = null;

function post(json: string): void {
  if (json !== "[]") scope.postMessage({ type: "server", json });
}

function fatal(e: unknown): void {
  if (dead) return;
  dead = true;
  const message = e instanceof Error ? e.message : String(e);
  scope.postMessage({ type: "fatal", message });
}

/** A Rust panic aborts the wasm instance: every later call would fail too. */
function guarded(f: () => void): void {
  if (dead) return;
  try {
    f();
  } catch (e) {
    fatal(e);
  }
}

scope.onmessage = (e) => {
  const msg = e.data;
  switch (msg.type) {
    case "init":
      guarded(() => {
        const { memory } = initSync({ module: msg.module });
        handled = new Int32Array(msg.handled);
        const fs = new SyncFs(msg.fsBuffer, msg.fsPort);
        const ctl = new WasmController(BigInt(msg.seed), msg.sampleRate, msg.control, msg.reports, fs);
        controller = ctl;
        const host = new WebclapHost({
          engineMemory: () => memory,
          log: (device, severity, text) => {
            if (severity >= 2) console.warn(`webclap ${device}: ${text}`);
          },
        });
        webclap = host;
        for (const m of msg.webclapModules ?? []) host.addModule(m.id, m.module, m.info);
        ctl.set_webclap_host(host);
        if (msg.webclapCatalog) ctl.set_webclap_catalog(msg.webclapCatalog);
        for (const x of extensions) x.onInit?.(msg, ctl);
        setInterval(
          () =>
            guarded(() => {
              post(ctl.tick(Date.now()));
              runHttp(ctl);
            }),
          TICK_MS,
        );
        scope.postMessage({ type: "ready" });
      });
      break;
    case "client":
      guarded(() => {
        if (!controller) throw new Error("controller not initialized");
        post(controller.handle(msg.json));
        runHttp(controller);
      });
      if (handled) Atomics.add(handled, 0, 1);
      break;
    case "webclap:catalog":
      guarded(() => controller?.set_webclap_catalog(msg.json));
      break;
    case "webclap:module":
      webclap?.addModule(msg.id, msg.module, msg.info);
      break;
    default:
      // Not `guarded`: an extension's errors are its caller's, not the engine's.
      if (dead) break;
      for (const x of extensions) x.onMessage?.(msg, (m) => scope.postMessage(m));
      break;
  }
};
