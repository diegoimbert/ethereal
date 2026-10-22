import { useContext, useSyncExternalStore } from "react";
import { TransportContext, type EngineTransport } from "@/transport";
import type { WebclapApi, WebclapModuleView } from "./api";

const NONE: WebclapModuleView[] = [];
const noop = () => () => {};

/** The transport's WebCLAP plugins (the web version's engine), `null` elsewhere. */
export function webclapOf(transport: EngineTransport | null | undefined): WebclapApi | null {
  if (transport?.kind !== "wasm") return null;
  return (transport as unknown as { webclap?: WebclapApi | null }).webclap ?? null;
}

/** `webclapOf` the current transport. */
export function useWebclap(): WebclapApi | null {
  return webclapOf(useContext(TransportContext)?.transport ?? null);
}

/** The installed modules, kept current. */
export function useWebclapModules(api: WebclapApi | null): WebclapModuleView[] {
  return useSyncExternalStore(api ? (l) => api.subscribe(l) : noop, () => api?.modules() ?? NONE);
}
