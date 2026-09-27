// OWNERSHIP: the `remote-engine` node owns `ui/src/features/remote/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ConnectDialog` (top bar, `data-slot="remote"`): keep the export names and keep them prop-less (read state via hooks).

/**
 * Connect to a remote engine (WebSocket URL + token).
 *
 * Inline slot: renders nothing until implemented, so the shell layout is unchanged.
 */
export function ConnectDialog() {
  return null;
}
