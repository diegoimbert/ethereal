import type { Track } from "@/generated";
import { VcaSummary } from "./controls";

/**
 * Arrangement lane of a VCA track: no clips (a VCA carries no audio), just what it
 * controls. Its volume automation shows in the automation lanes under it, like any track.
 */
export function VcaLane({ track }: { track: Track }) {
  return (
    <div className="eth-arr-lane eth-groups-vca-lane" data-lane={track.id} data-testid="vca-lane">
      <span className="eth-groups-vca-lane__label">VCA</span>
      <VcaSummary vca={track} className="eth-groups-vca-lane__tracks" />
    </div>
  );
}
