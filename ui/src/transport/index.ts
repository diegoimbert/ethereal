// Engine connection: the `EngineTransport` contract, its implementations, and the React
// provider/hooks. Features import from "@/transport" and "@/state" only. The mock engine is
// `@/transport/mock` (not re-exported here: the app loads it only when it runs against it).
export { cmd, type CommandDomain, type DomainCommand } from "./cmd";
export { TransportContext, useConnectionStatus, useTransport, useTransportEvent, type ConnectionStatus, type TransportContextValue } from "./context";
export { createDefaultTransport, isTauri, loadMockTransport } from "./createDefaultTransport";
export { CommandFailedError, commandErrorText, Emitter, isCommandFailed, type EngineTransport, type SendOptions, type Unsubscribe } from "./EngineTransport";
export { openExternal } from "./external";
export { listProjects } from "./projectList";
export { deriveId, newId, newProjectId, nextGestureId, uuidv7 } from "./ids";
export { BUILTIN_DESCRIPTORS, builtinDescriptor, clampParam } from "./mock/builtinDevices";
export { TauriTransport } from "./tauri/TauriTransport";
export { TransportProvider, type TransportProviderProps } from "./TransportProvider";
export { WasmTransport } from "./wasm/WasmTransport";
export { WsTransport, type WsTransportOptions } from "./ws/WsTransport";
