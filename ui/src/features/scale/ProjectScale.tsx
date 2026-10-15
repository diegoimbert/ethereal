import type { Command } from "@/generated";
import { CHROMATIC_SCALE } from "@/domain/scales";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useT } from "@/i18n";
import { ScaleSelect } from "./ScaleSelect";

/**
 * The project's scale (Projects menu): the default for every MIDI track's piano roll. `send`
 * reports failures through its owner (the Projects menu's error line).
 */
export function ProjectScale({ send }: { send: (command: Command) => unknown }) {
  const t = useT();
  const scale = useProjectStore((s) => s.project?.settings.scale);
  return (
    <span className="eth-project-scale">
      {t("scale.project.title")}
      <ScaleSelect
        label={t("scale.project.title")}
        value={scale ?? CHROMATIC_SCALE}
        disabled={!scale}
        onChange={(value) => void send(cmd("Project", { type: "SetScale", scale: value }))}
      />
    </span>
  );
}
