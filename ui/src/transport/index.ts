// Engine connection: the `EngineTransport` contract, its implementations, and the React
// provider/hooks. Features import from "@/transport" and "@/state" only.
export { cmd, type CommandDomain, type DomainCommand } from "./cmd";
export {
  TransportContext,
  useConnectionStatus,
  useTransport,
  useTransportEvent,
  type ConnectionStatus,
  type TransportContextValue,
} from "./context";
export { createDefaultTransport, isTauri } from "./createDefaultTransport";
export {
  CommandFailedError,
  Emitter,
  isCommandFailed,
  type EngineTransport,
  type SendOptions,
  type Unsubscribe,
} from "./EngineTransport";
export { newId, nextGestureId } from "./ids";
export { BUILTIN_DESCRIPTORS, builtinDescriptor, clampParam } from "./mock/builtinDevices";
export { createDemoProject, createEmptyProject } from "./mock/demoProject";
export { ETHER_FORMAT, ETHER_VERSION, MockTransport, parseEtherFile, type MockTransportOptions } from "./mock/MockTransport";
export { TauriTransport } from "./tauri/TauriTransport";
export { TransportProvider, type TransportProviderProps } from "./TransportProvider";
export { WasmTransport } from "./wasm/WasmTransport";
