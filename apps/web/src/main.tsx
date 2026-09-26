// Browser host entry.
//
// The UI talks to the wasm engine through `WasmTransport`: the real `EtherController` runs
// in a Web Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). The standalone UI against `MockTransport` is `just dev-ui`.
import { App, TransportProvider, WasmTransport } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createWebEndpoint } from "./engine/endpoint";

const endpoint = createWebEndpoint();
// Debug / e2e handle on the live engine (AudioContext, worklet node, workers).
(window as unknown as { __etherEngine?: unknown }).__etherEngine = endpoint;
const transport = new WasmTransport({ endpoint });

const root = document.getElementById("root");
if (!root) throw new Error("#root element missing");

createRoot(root).render(
  <StrictMode>
    <TransportProvider transport={transport}>
      <App />
    </TransportProvider>
  </StrictMode>,
);
