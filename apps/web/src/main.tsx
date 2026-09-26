// Browser host entry. OWNERSHIP: the `wasm-host` node owns apps/web/**.
//
// The UI talks to the wasm engine through `WasmTransport`: the controller runs in a Web
// Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). Query parameters:
//   ?controller=fake   use the thin smoke-test controller instead of EtherController
//   ?transport=mock    run the UI against MockTransport (no engine)
import { App, MockTransport, TransportProvider, WasmTransport, type EngineTransport } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { createWebEndpoint } from "./engine/endpoint";

const params = new URLSearchParams(window.location.search);

function createTransport(): EngineTransport {
  if (params.get("transport") === "mock") return new MockTransport();
  const endpoint = createWebEndpoint({ controller: params.get("controller") === "fake" ? "fake" : "ether" });
  // Debug / e2e handle on the live engine (AudioContext, worklet node, workers).
  (window as unknown as { __etherEngine?: unknown }).__etherEngine = endpoint;
  return new WasmTransport({ endpoint });
}

const transport = createTransport();
const root = document.getElementById("root");
if (!root) throw new Error("#root element missing");

createRoot(root).render(
  <StrictMode>
    <TransportProvider transport={transport}>
      <App />
    </TransportProvider>
  </StrictMode>,
);
