// OWNERSHIP: `share-ui` owns `ui/src/features/share/**` except `endpoint/` (p2p-transport) and
// `join/` (join-flow). The app shell mounts `ShareControl` (top bar, `data-slot="collab"`).
/** Sharing (docs/SHARING.md §8): Share button, session pill, Share popover, settings, toasts. */
export { ShareControl } from "./ShareControl";
export { SharingAdvancedSettings, SharingSettings } from "./SharingSettings";
export { PeerAvatar, AvatarStack } from "./PeerAvatar";
export { sessionStatus, type SessionStatus } from "./status";
export { PEER_COLORS, useShareSettings } from "./settings";
export { hostOf, participantsOf, useJoinedRole, useShareStore, useViewOnly } from "./store";
export { shareToast } from "./toasts";
