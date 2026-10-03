// OWNERSHIP: the `remote-engine` node owns `ui/src/features/remote/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `ConnectDialog` (top bar, `data-slot="remote"`): keep the export names and keep them prop-less (read state via hooks).
export { ConnectDialog } from "./ConnectDialog";
export { RemoteEngineSettings } from "./RemoteEngineSettings";
export { uploadFile, uploadFiles, uploadStore, useUploads, withUpload, type UploadSource } from "./upload";
export { useUploadDrop, type UploadDropProps } from "./uploadDrop";
export { normalizeServerUrl } from "./url";
