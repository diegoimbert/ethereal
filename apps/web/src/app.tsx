// The browser app (the engine and the editor), loaded by ./main.tsx. Phones and touch-only
// tablets get the mobile UI (./mobile.ts, #522).
//
// The UI talks to the wasm engine through `WasmTransport`: the real `EtherController` runs
// in a Web Worker, the engine in an AudioWorklet, connected by SharedArrayBuffer rings
// (./engine/endpoint.ts). The standalone UI against `MockTransport` is `just dev-ui`; `?mock`
// runs this build against it too (e2e of mock-only fixtures, e.g. the 10,000-param plugin).
import { App, cmd, enableMobileUi, MockTransport, playheadStore, TransportProvider, useProjectStore, WasmTransport, type EngineTransport } from "@ethereal/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { DesktopBandFrame } from "@/features/desktop-band";
import { markWebSessionClosed } from "@/features/versions/session";
import { openSettings } from "@/features/audio-settings";
import { ensurePersistentStorage } from "@/features/storage";
import { claimStore, EngineErrorScreen, OtherTabScreen, takeOverStore, type StoreOwner } from "@/features/startup";
import { initI18n } from "@/i18n";
import { currentDevice, isDesktopMachine, preferPlaybackAudioSession, wantsMobileUi } from "./mobile";

const rootElement = document.getElementById("root");
if (!rootElement) throw new Error("#root element missing");
const root = createRoot(rootElement);

/**
 * Boot the editor; if that fails before the editor shows (#597: the engine's chunk didn't
 * load, e.g. a page left open across a deploy), say why instead of doing nothing.
 */
function startApp(): void {
  boot().catch((error: unknown) => {
    console.error("ethereal: the app didn't start", error);
    root.render(<EngineErrorScreen error={error} />);
  });
}

async function boot(): Promise<void> {
  // The touch UI (#522): phones and touch-only tablets, or asked for.
  const mobile = wantsMobileUi();
  if (mobile) {
    enableMobileUi();
    preferPlaybackAudioSession();
  }

  let transport: EngineTransport;
  if (new URLSearchParams(window.location.search).has("mock")) {
    const m = new MockTransport();
    // e2e handle: send commands the UI has no button for (e.g. insert a mock plugin).
    (window as unknown as { __etherMock?: unknown }).__etherMock = m;
    transport = m;
  } else {
    // One tab owns the project store (#563): wait here (no engine) while another one has it.
    const owner = await ownStore();
    // Ask the browser to keep the projects (#622); never holds up the start.
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
  // Read-only e2e/debug view of the UI mirror: the project document, history, dirty flag,
  // the latest meter readings and playhead (apps/web/e2e reads these to check state across
  // reloads, and to pace meter checks by transport time).
  (window as unknown as { __ether?: unknown }).__ether = {
    state: () => useProjectStore.getState(),
    meter: (track: string) => playheadStore.getMeter(track),
    playhead: () => playheadStore.getPlayhead(),
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

// The user's language first (a lazy chunk unless English), so nothing renders in English first.
await initI18n();
startApp();
