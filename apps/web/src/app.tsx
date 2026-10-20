// The browser app (the engine and the editor), loaded by ./main.tsx. Phones and touch-only
// tablets get the mobile UI (./mobile.ts).
//
// The UI talks to the wasm engine through `WasmTransport`: the real `EtherController` runs
// in a Web Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). The standalone UI against `MockTransport` is `just dev-ui`; `?mock`
// runs this build against it too (e2e of mock-only fixtures, e.g. the 10,000-param plugin).
//
// Extensions (`@/ext`, docs/EXTENSIONS.md) see the boot ("web.boot", with the invite a web route
// started it for), may wrap the transport ("web.transport") and add to the debug handle
// ("web.debug").
import {
  App,
  cmd,
  enableMobileUi,
  loadMockTransport,
  playheadStore,
  TransportProvider,
  useProjectStore,
  WasmTransport,
  type EngineTransport,
} from "@ethereal/ui";
import { StrictMode } from "react";
import { DesktopBandFrame } from "@/features/desktop-band";
import { markWebSessionClosed } from "@/features/versions/session";
import { openSettings } from "@/features/audio-settings";
import { ensurePersistentStorage } from "@/features/storage";
import { claimStore, EngineErrorScreen, OtherTabScreen, takeOverStore, type StoreOwner } from "@/features/startup";
import { initI18n } from "@/i18n";
import { contributions, emit } from "@/ext";
import { applyMobileUi, currentDevice, isDesktopMachine } from "./mobile";
import { appRoot } from "./root";

const root = appRoot();

/** Boot the editor once the user's language is loaded (`./main.tsx`). */
export async function runApp(invite: string | null): Promise<void> {
  // The user's language first (a lazy chunk unless English), so nothing renders in English first.
  await initI18n();
  startApp(invite);
}

/**
 * Boot the editor; if that fails before the editor shows (the engine's chunk didn't
 * load, e.g. a page left open across a deploy), say why instead of doing nothing.
 */
function startApp(invite: string | null): void {
  boot(invite).catch((error: unknown) => {
    console.error("ethereal: the app didn't start", error);
    root.render(<EngineErrorScreen error={error} />);
  });
}

async function boot(invite: string | null): Promise<void> {
  // The touch UI: phones and touch-only tablets, or asked for.
  const mobile = applyMobileUi(enableMobileUi);
  emit("web.boot", invite);

  let transport: EngineTransport;
  if (new URLSearchParams(window.location.search).has("mock")) {
    // The mock engine, from its own chunk (not loaded otherwise), with the extensions' domains.
    const { MockTransport } = await loadMockTransport();
    const m = new MockTransport();
    // e2e handle: send commands the UI has no button for (e.g. insert a mock plugin).
    (window as unknown as { __etherMock?: unknown }).__etherMock = m;
    transport = m;
  } else {
    // One tab owns the project store: wait here (no engine) while another one has it.
    const owner = await ownStore();
    // Ask the browser to keep the projects; never holds up the start.
    void ensurePersistentStorage({ onOpenSettings: () => openSettings("general") });
    const { createWebEndpoint } = await import("./engine/endpoint");
    const endpoint = createWebEndpoint();
    // Debug / e2e handle on the live engine (AudioContext, worklet node, workers).
    (window as unknown as { __etherEngine?: unknown }).__etherEngine = endpoint;
    const wasm = new WasmTransport({ endpoint });
    transport = wasm;
    owner.onHandover(async () => {
      // Another tab takes over: save every change, stop the engine, then it has the store.
      if (useProjectStore.getState().project) await wasm.send(cmd("Project", { type: "Save" })).catch(() => undefined);
      wasm.dispose();
      showMoved();
      markWebSessionClosed();
    });
    // Taken without a handover (this tab didn't answer in time): stop writing at once.
    owner.onLost(() => {
      wasm.dispose();
      showMoved();
    });
  }
  for (const wrap of contributions("web.transport")) transport = (await wrap(transport)) ?? transport;
  // Read-only e2e/debug view of the UI mirror: the project document, history, dirty flag,
  // the latest meter readings and playhead (apps/web/e2e reads these to check state across
  // reloads, and to pace meter checks by transport time).
  (window as unknown as { __ether?: unknown }).__ether = {
    state: () => useProjectStore.getState(),
    meter: (track: string) => playheadStore.getMeter(track),
    playhead: () => playheadStore.getPlayhead(),
    ...Object.assign({}, ...contributions("web.debug").map((extra) => extra(transport))),
  };

  root.render(
    <StrictMode>
      {/* "Open on desktop", above the editor on desktop machines. */}
      <DesktopBandFrame show={!mobile && isDesktopMachine(currentDevice())}>
        <TransportProvider transport={transport}>
          <App />
        </TransportProvider>
      </DesktopBandFrame>
    </StrictMode>,
  );
}

/** The store, once this tab owns it: at once, or after "use here" on the other-tab screen. */
async function ownStore(): Promise<StoreOwner> {
  const owner = await claimStore();
  if (owner) return owner;
  return new Promise((resolve) => {
    root.render(<OtherTabScreen reason="busy" onUseHere={() => takeOverStore().then(resolve)} />);
  });
}

/** This tab handed the store over: its editor is gone; "use here" takes it back (a fresh load). */
function showMoved(): void {
  root.render(
    <OtherTabScreen
      reason="moved"
      onUseHere={() =>
        takeOverStore().then((owner) => {
          // The reload starts a fresh editor, which claims the store again once this page lets go.
          owner.release();
          location.reload();
        })
      }
    />,
  );
}
