// Main-thread side of the browser engine: compiles the wasm module once, creates the two
// SAB rings, starts the AudioWorklet (engine), the OPFS Worker and the controller Worker,
// and exposes them to `WasmTransport` as a `WasmEndpoint`. Extensions (`@/ext` "web.engine")
// may bring their own worklet and controller Worker modules (wrapping the app's), more worklet
// outputs, init fields and messages, and methods of the endpoint.
import wasmUrl from "@ether-wasm/ether_wasm_bg.wasm?url";
import { contributions, type WebEngineExtension } from "@/ext";
import { Emitter } from "@/transport/EngineTransport";
import type { WasmEndpoint } from "@/transport/wasm/WasmTransport";
import workletUrl from "./engine.worklet.ts?worker&url";
import { CONTROL_RING_BYTES, FS_CHUNK_BYTES, FS_HEADER_BYTES, REPORT_RING_BYTES, ringBuffer, type FromController, type ToController } from "./protocol";
import { webclap } from "./webclap/manager";

/** Live handles, for debugging and the e2e test (`window.__etherEngine`). */
export interface WebEngineHandles {
  context: AudioContext;
  node: AudioWorkletNode;
  controller: Worker;
  fs: Worker;
}

async function compileWasm(): Promise<WebAssembly.Module> {
  const res = await fetch(wasmUrl);
  if (!res.ok) throw new Error(`failed to load ${wasmUrl}: ${res.status}`);
  return WebAssembly.compile(await res.arrayBuffer());
}

function randomSeed(): string {
  return crypto.getRandomValues(new BigUint64Array(1))[0]!.toString();
}

/** Resume the AudioContext on the first user gesture (browser autoplay policy). */
function resumeOnGesture(context: AudioContext): () => void {
  const resume = () => {
    if (context.state !== "running") void context.resume();
  };
  const events = ["pointerdown", "keydown", "touchend"] as const;
  for (const ev of events) window.addEventListener(ev, resume, { capture: true });
  resume();
  return () => {
    for (const ev of events) window.removeEventListener(ev, resume, { capture: true });
  };
}

/** What `createWebEndpoint` returns (`window.__etherEngine`). */
export interface WebEndpoint extends WasmEndpoint {
  handles(): WebEngineHandles | null;
}

export function createWebEndpoint(extensions: ReadonlyArray<WebEngineExtension> = contributions("web.engine")): WebEndpoint {
  const batches = new Emitter<string>();
  const fatal = new Emitter<Error>();
  let handles: WebEngineHandles | null = null;
  let cleanup: (() => void)[] = [];
  let dead = false;
  // `client` messages posted, and handled by the Worker (`drain`).
  const handled = new Int32Array(new SharedArrayBuffer(4));
  let posted = 0;

  const die = (message: string) => {
    if (dead) return;
    dead = true;
    console.error(`Ethereal engine: ${message}`);
    fatal.emit(new Error(message));
  };

  /** Stop whatever is running (the workers, the audio context); the emitters stay. */
  const teardown = () => {
    for (const c of cleanup) c();
    cleanup = [];
    webclap.attach(null);
    if (handles) {
      handles.controller.terminate();
      handles.fs.terminate();
      handles.node.disconnect();
      void handles.context.close();
      handles = null;
    }
  };

  // A failed start leaves nothing running, and can be tried again.
  const start = async (): Promise<void> => {
    teardown();
    dead = false;
    posted = 0;
    Atomics.store(handled, 0, 0);
    try {
      await boot();
    } catch (e) {
      teardown();
      throw e;
    }
  };

  const boot = async (): Promise<void> => {
    if (!window.crossOriginIsolated) {
      throw new Error("the page is not cross-origin isolated (coop/coep headers); the web engine needs SharedArrayBuffer");
    }
    if (typeof AudioWorkletNode === "undefined") throw new Error("this browser has no AudioWorklet support");

    // The installed WebCLAP modules, compiled, go to both engine realms at boot.
    const [module] = await Promise.all([compileWasm(), webclap.load()]);
    const control = ringBuffer(CONTROL_RING_BYTES);
    const reports = ringBuffer(REPORT_RING_BYTES);
    const fsBuffer = new SharedArrayBuffer(FS_HEADER_BYTES + FS_CHUNK_BYTES);

    const context = new AudioContext({ latencyHint: "interactive" });
    // Until `handles` holds them, a failed start closes what it made here (`teardown`).
    cleanup.push(() => {
      if (!handles) void context.close().catch(() => undefined);
    });
    await context.audioWorklet.addModule(extensions.find((x) => x.workletUrl)?.workletUrl ?? workletUrl);
    // Output 0 → speakers; an extension's outputs after it (each stereo).
    const outputs = 1 + extensions.reduce((n, x) => n + (x.extraOutputs ?? 0), 0);
    const node = new AudioWorkletNode(context, "ether-engine", {
      numberOfInputs: 0,
      numberOfOutputs: outputs,
      outputChannelCount: Array.from({ length: outputs }, () => 2),
      processorOptions: {
        module,
        control,
        reports,
        webclap: webclap.moduleMessages(),
        ...Object.assign({}, ...extensions.map((x) => x.processorOptions?.() ?? {})),
      },
    });
    node.onprocessorerror = () => die("the audio worklet failed");
    node.port.onmessage = (e: MessageEvent<{ type: string; message?: string }>) => {
      if (e.data.type === "fatal") die(`audio worklet: ${e.data.message ?? "crashed"}`);
    };
    node.connect(context.destination, 0);

    const fs = new Worker(new URL("./opfs.worker.ts", import.meta.url), { type: "module", name: "ether-opfs" });
    const channel = new MessageChannel();
    fs.postMessage({ type: "init", port: channel.port1, buffer: fsBuffer }, [channel.port1]);
    fs.onerror = (e) => die(`opfs worker: ${e.message}`);
    cleanup.push(() => {
      if (!handles) fs.terminate();
    });

    const controller =
      extensions.find((x) => x.controllerWorker)?.controllerWorker?.() ??
      new Worker(new URL("./controller.worker.ts", import.meta.url), {
        type: "module",
        name: "ether-controller",
      });
    handles = { context, node, controller, fs };
    const h = handles;
    webclap.attach({ worklet: node.port, controller });
    cleanup.push(resumeOnGesture(context));

    await new Promise<void>((resolve, reject) => {
      controller.onerror = (e) => {
        reject(new Error(`controller worker: ${e.message}`));
        die(`controller worker: ${e.message}`);
      };
      controller.onmessage = (e: MessageEvent<FromController>) => {
        const msg = e.data;
        switch (msg.type) {
          case "ready":
            resolve();
            break;
          case "server":
            batches.emit(msg.json);
            break;
          case "fatal":
            reject(new Error(msg.message));
            die(`controller: ${msg.message}`);
            break;
          default:
            for (const x of extensions) x.onControllerMessage?.(msg, h);
            break;
        }
      };
      // The extensions' init fields and transferables (and what to stop with the engine).
      const extra: Record<string, unknown> = {};
      const transfer: Transferable[] = [channel.port2];
      for (const x of extensions) {
        const i = x.init?.(h);
        if (!i) continue;
        Object.assign(extra, i.fields ?? {});
        transfer.push(...(i.transfer ?? []));
        if (i.cleanup) cleanup.push(i.cleanup);
      }
      const init: ToController = {
        type: "init",
        module,
        control,
        reports,
        fsBuffer,
        fsPort: channel.port2,
        seed: randomSeed(),
        sampleRate: context.sampleRate,
        mediaStepsPerTick: debugMediaSteps(),
        handled: handled.buffer as SharedArrayBuffer,
        webclapCatalog: webclap.catalogJson(),
        webclapModules: webclap.moduleMessages(),
        ...extra,
      };
      controller.postMessage(init, transfer);
    });
  };

  return {
    start,
    post(json) {
      if (!handles) return;
      posted += 1;
      handles.controller.postMessage({ type: "client", json } satisfies ToController);
    },
    drain(timeoutMs) {
      // The main thread may not `Atomics.wait`: spin (only a closing page does this).
      const deadline = performance.now() + timeoutMs;
      while (!dead && Atomics.load(handled, 0) < posted) {
        if (performance.now() >= deadline) return false;
      }
      return !dead;
    },
    onMessages: (l) => batches.on(l),
    webclap,
    onFatal: (l) => fatal.on(l),
    handles: () => handles,
    // The extensions' methods (a stream tap, probes, …).
    ...Object.assign({}, ...extensions.map((x) => x.methods?.(() => handles) ?? {})),
    dispose() {
      teardown();
      batches.clear();
      fatal.clear();
    },
  };
}

/**
 * Debug override of the media decode steps per controller tick (`localStorage`
 * `eth.debug.media-steps`): e2e specs that watch a load in progress set it to 1 so the
 * decode lasts long enough to look at. `undefined` = the controller's default.
 */
function debugMediaSteps(): number | undefined {
  try {
    const v = Number(globalThis.localStorage?.getItem("eth.debug.media-steps"));
    return Number.isInteger(v) && v > 0 ? v : undefined;
  } catch {
    return undefined;
  }
}
