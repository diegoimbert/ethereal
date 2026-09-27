// OWNERSHIP: the `collab` node owns `ui/src/features/collab/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `PresenceBar` (top bar, `data-slot="collab"`): keep the export names and keep them prop-less (read state via hooks).

/** Collaboration: session join/leave and the peers' presence (docs/COLLAB.md). */
export { PresenceBar } from "./PresenceBar";
