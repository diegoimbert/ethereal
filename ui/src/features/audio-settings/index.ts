// The Settings dialog (base-115: Audio | Sharing | Advanced tabs, docs/SHARING.md §8.6): audio
// devices and an input check; identity and sharing preferences; the signaling and ICE
// servers, the engine server (remote engine) and the relay session. The app shell mounts
// `AudioSettingsDialog` once; open it with `openSettings(tab)` or `openAudioSettings()`.
export { AudioSettingsDialog } from "./AudioSettingsDialog";
export { openAudioSettings, openSettings, promptForInputIfNone, useAudioSettings, type SettingsTab } from "./store";
