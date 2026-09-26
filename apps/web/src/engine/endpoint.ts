// Main-thread side of the browser engine: compiles the wasm module once, creates the two
// SAB rings, starts the AudioWorklet (engine), the OPFS Worker and the controller Worker,
// and exposes them to `WasmTransport` as a `WasmEndpoint`.
import wasmUrl from "@ether-wasm/ether_wasm_bg.wasm?url";
import { Emitter } from "@/transport/EngineTransport";
import type { WasmEndpoint } from "@/transport/wasm/WasmTransport";
import workletUrl from "./engine.worklet.ts?worker&url";
import {
  CONTROL_RING_BYTES,
  FS_CHUNK_BYTES,
  FS_HEADER_BYTES,
  REPORT_RING_BYTES,
  ringBuffer,
  type ControllerMode,
  type FromController,
  type ToController,
} from "./protocol";

export interface WebEndpointOptions {
  /** `"ether"`: the real controller (default); `"fake"`: the smoke-test controller. */
  controller?: ControllerMode;
}

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

export function createWebEndpoint(opts: WebEndpointOptions = {}): WasmEndpoint & { handles(): WebEngineHandles | null } {
  const batches = new Emitter<string>();
  const fatal = new Emitter<Error>();
  let handles: WebEngineHandles | null = null;
  let cleanup: (() => void)[] = [];
  let dead = false;

  const die = (message: string) => {
    if (dead) return;
    dead = true;
    console.error(`Ethereal engine: ${message}`);
    fatal.emit(new Error(message));
  };

  const start = async (): Promise<void> => {
    if (!window.crossOriginIsolated) {
      throw new Error("the page is not cross-origin isolated (COOP/COEP headers); the web engine needs SharedArrayBuffer");
    }
    if (typeof AudioWorkletNode === "undefined") throw new Error("this browser has no AudioWorklet support");

    const module = await compileWasm();
    const control = ringBuffer(CONTROL_RING_BYTES);
    const reports = ringBuffer(REPORT_RING_BYTES);
    const fsBuffer = new SharedArrayBuffer(FS_HEADER_BYTES + FS_CHUNK_BYTES);

    const context = new AudioContext({ latencyHint: "interactive" });
    await context.audioWorklet.addModule(workletUrl);
    const node = new AudioWorkletNode(context, "ether-engine", {
      numberOfInputs: 0,
      numberOfOutputs: 1,
      outputChannelCount: [2],
      processorOptions: { module, control, reports },
    });
    node.onprocessorerror = () => die("the audio worklet failed");
    node.port.onmessage = (e: MessageEvent<{ type: string; message?: string }>) => {
      if (e.data.type === "fatal") die(`audio worklet: ${e.data.message ?? "crashed"}`);
    };
    node.connect(context.destination);

    const fs = new Worker(new URL("./opfs.worker.ts", import.meta.url), { type: "module", name: "ether-opfs" });
    const channel = new MessageChannel();
    fs.postMessage({ type: "init", port: channel.port1, buffer: fsBuffer }, [channel.port1]);
    fs.onerror = (e) => die(`OPFS worker: ${e.message}`);

    const controller = new Worker(new URL("./controller.worker.ts", import.meta.url), {
      type: "module",
      name: "ether-controller",
    });
    handles = { context, node, controller, fs };
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
        }
      };
      const init: ToController = {
        type: "init",
        module,
        control,
        reports,
        fsBuffer,
        fsPort: channel.port2,
        seed: randomSeed(),
        mode: opts.controller ?? "ether",
        sampleRate: context.sampleRate,
      };
      controller.postMessage(init, [channel.port2]);
    });
  };

  return {
    start,
    post(json) {
      handles?.controller.postMessage({ type: "client", json } satisfies ToController);
    },
    onMessages: (l) => batches.on(l),
    onFatal: (l) => fatal.on(l),
    handles: () => handles,
    dispose() {
      for (const c of cleanup) c();
      cleanup = [];
      if (handles) {
        handles.controller.terminate();
        handles.fs.terminate();
        handles.node.disconnect();
        void handles.context.close();
        handles = null;
      }
      batches.clear();
      fatal.clear();
    },
  };
}
