import type { Command } from "@/generated";
import { CHROMATIC_SCALE } from "@/domain/scales";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { ScaleSelect } from "./ScaleSelect";

/**
 * The project's scale (Projects menu): the default for every MIDI track's piano roll. `send`
 * reports failures through its owner (the Projects menu's error line).
 */
export function ProjectScale({ send }: { send: (command: Command) => unknown }) {
  const scale = useProjectStore((s) => s.project?.settings.scale);
  return (
    <span className="eth-project-scale">
      Project scale
      <ScaleSelect
        label="Project scale"
        value={scale ?? CHROMATIC_SCALE}
        disabled={!scale}
        onChange={(value) => void send(cmd("Project", { type: "SetScale", scale: value }))}
      />
    </span>
  );
}
