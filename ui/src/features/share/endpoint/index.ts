// The web share endpoint's UI agent (docs/SHARING.md §6.2): the browser's peer connections
// for the controller Worker's `ether_collab::share::web::WebPeers`, over the share port.
// Started by the web host app (`apps/web/src/engine/endpoint.ts`) with its end of the port.
export { CONNECT_TIMEOUT_MS, FLOW_POLL_MS, LOW_WATER_BYTES, sdpFingerprint, startShareEndpoint, webrtcUnavailableReason } from "./agent";
export type { ShareEndpointOptions, SharePort } from "./agent";
export { DC_LABEL, type FromUi, type ToUi } from "./protocol";
