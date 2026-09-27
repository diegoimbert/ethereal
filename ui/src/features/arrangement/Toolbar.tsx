import { LocateFixed } from "lucide-react";
import type { GridSetting } from "@/timeline";
import { useTimelineView } from "@/timeline";
import { Button, Select } from "@/kit";
import { NewTrackButton } from "./newTrack";
import { GRID_OPTIONS } from "./helpers";
import { arrangementView, useArrangementUi } from "./uiStore";

function gridId(grid: GridSetting): string {
  return GRID_OPTIONS.find((o) => JSON.stringify(o.grid) === JSON.stringify(grid))?.id ?? "adaptive-medium";
}

export function Toolbar() {
  const grid = useArrangementUi((s) => s.grid);
  const follow = useTimelineView(arrangementView, (s) => s.followPlayhead);

  return (
    <div className="eth-arr-toolbar" role="toolbar" aria-label="Arrangement tools">
      <NewTrackButton />
      <span className="eth-arr-toolbar__sep" />
      <span className="eth-arr-toolbar__grid">
        Grid
        <Select
          size="sm"
          aria-label="Grid"
          value={gridId(grid)}
          options={GRID_OPTIONS.map((o) => ({ value: o.id, label: o.label }))}
          onChange={(id) => {
            const o = GRID_OPTIONS.find((x) => x.id === id);
            if (o) useArrangementUi.getState().setGrid(o.grid);
          }}
        />
      </span>
      <Button
        size="sm"
        active={follow}
        title="Follow the playhead while playing"
        onClick={() => arrangementView.getState().setFollowPlayhead(!follow)}
      >
        <LocateFixed />
        Follow
      </Button>
    </div>
  );
}
