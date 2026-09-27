import { CHROMATIC_SCALE } from "@/domain/scales";
import { useProjectStore } from "@/state";
import { cmd } from "@/transport";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { ScaleSelect } from "./ScaleSelect";

export function ProjectScale() {
  const scale = useProjectStore((s) => s.project?.settings.scale);
  const { send, error, clearError } = useEngineCommands();
  return <span className="eth-project-scale">
    Project scale
    <ScaleSelect label="Project scale" value={scale ?? CHROMATIC_SCALE} disabled={!scale}
      onChange={(value) => void send(cmd("Project", { type: "SetScale", scale: value }))} />
    {error && <button role="alert" onClick={clearError}>{error}</button>}
  </span>;
}
