import { LocateFixed, ZoomIn, ZoomOut } from "lucide-react";
import type { GridSetting } from "@/timeline";
import { animateZoom, useTimelineView } from "@/timeline";
import { Button, Select } from "@/kit";
import { useArrangement } from "./context";
import { addTrack, runClipAction } from "./actions";
import { GRID_OPTIONS } from "./helpers";
import { arrangementView, useArrangementUi } from "./uiStore";

function gridId(grid: GridSetting): string {
  return GRID_OPTIONS.find((o) => JSON.stringify(o.grid) === JSON.stringify(grid))?.id ?? "adaptive-medium";
}

export function Toolbar() {
  const { transport } = useArrangement();
  const grid = useArrangementUi((s) => s.grid);
  const follow = useTimelineView(arrangementView, (s) => s.followPlayhead);
  const run = (a: Parameters<typeof runClipAction>[1]) => () => void runClipAction(transport, a);

  return (
    <div className="eth-arr-toolbar" role="toolbar" aria-label="Arrangement tools">
      <Button size="sm" onClick={() => void addTrack(transport, "Midi")} title="Add a MIDI track with the built-in synth">
        + MIDI track
      </Button>
      <Button size="sm" onClick={() => void addTrack(transport, "Audio")} title="Add an audio track">
        + Audio track
      </Button>
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
      <span className="eth-arr-toolbar__sep" />
      <Button size="sm" onClick={run("split")} title="Split selected clips at the playhead (Ctrl/Cmd+E)">
        Split
      </Button>
      <Button size="sm" onClick={run("duplicate")} title="Duplicate selected clips (Ctrl/Cmd+D)">
        Duplicate
      </Button>
      <Button size="sm" onClick={run("loop")} title="Toggle looping of selected clips (Ctrl/Cmd+Shift+L)">
        Loop
      </Button>
      <Button size="sm" onClick={run("delete")} title="Delete selected clips (Delete)">
        Delete
      </Button>
      <span className="eth-arr-toolbar__sep" />
      <Button size="sm" aria-label="Zoom out" title="Zoom out (Ctrl/Cmd+wheel)" onClick={() => animateZoom(arrangementView, 1 / 1.5)}>
        <ZoomOut />
      </Button>
      <Button size="sm" aria-label="Zoom in" title="Zoom in (Ctrl/Cmd+wheel)" onClick={() => animateZoom(arrangementView, 1.5)}>
        <ZoomIn />
      </Button>
    </div>
  );
}
