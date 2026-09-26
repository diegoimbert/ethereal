// OWNERSHIP: the `ui-mixer` node owns `ui/src/features/mixer/**`.
// Only edit files inside this folder. The app shell (ui/src/app/App.tsx) already mounts
// `Mixer`: keep this export name and keep it prop-less (read state via hooks).
import { MixerView } from "./MixerView";

/** Mixer: strips (volume, pan, mute/solo, sends, output routing), meters, groups, master. */
export function Mixer() {
  return <MixerView />;
}
