// Presence v2 (node `presence-v2`, docs/COLLAB.md §8): live arranger pointers, activity
// hints, follow mode. The arrangement mounts `PresenceLayer`; gesture code calls
// `setActivity` (import it from "./local" to keep that module a leaf).
export { PresenceLayer, type PresenceLayerProps } from "./PresenceLayer";
export { activityLabel, presenceV2Fields, setActivity, setFollowing, useLocalPresence } from "./local";
export { usePointerStore } from "./pointers";
