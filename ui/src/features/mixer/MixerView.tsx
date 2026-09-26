import { useMemo, useState } from "react";
import type { Track, TrackId } from "@/generated";
import { useProjectStore } from "@/state";
import { MixerStrip } from "./MixerStrip";
import { mixerLayout, type StripNode } from "./routing";
import "./mixer.css";

/** The strip layout; recomputed only when the track table changes (immer keeps it stable). */
function useLayout() {
  const tracks = useProjectStore((s) => s.project?.tracks);
  return useMemo(() => (tracks ? mixerLayout(tracks) : null), [tracks]);
}

function StripTree({
  node,
  returns,
  folded,
  toggle,
}: {
  node: StripNode;
  returns: Track[];
  folded: ReadonlySet<TrackId>;
  toggle: (id: TrackId) => void;
}) {
  const { track, children } = node;
  if (track.kind !== "Group") return <MixerStrip track={track} returns={returns} />;
  const isFolded = folded.has(track.id);
  return (
    <div className="eth-mixer__group" data-group={track.id} style={{ ["--eth-group-color" as string]: `#${track.color.toString(16).padStart(6, "0")}` }}>
      <MixerStrip track={track} returns={returns} folded={isFolded} onToggleFold={() => toggle(track.id)} />
      {!isFolded && children.length > 0 && (
        <div className="eth-mixer__group-children">
          {children.map((c) => (
            <StripTree key={c.track.id} node={c} returns={returns} folded={folded} toggle={toggle} />
          ))}
        </div>
      )}
    </div>
  );
}

export function MixerView() {
  const layout = useLayout();
  const [folded, setFolded] = useState<ReadonlySet<TrackId>>(() => new Set());
  const toggle = (id: TrackId) =>
    setFolded((f) => {
      const next = new Set(f);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  if (!layout) return <div className="eth-mixer eth-mixer--empty" data-feature="mixer">No project loaded</div>;
  return (
    <div className="eth-mixer" data-feature="mixer">
      <div className="eth-mixer__tracks">
        {layout.tracks.map((n) => (
          <StripTree key={n.track.id} node={n} returns={layout.returns} folded={folded} toggle={toggle} />
        ))}
      </div>
      {layout.returns.length > 0 && (
        <div className="eth-mixer__returns">
          {layout.returns.map((t) => (
            <MixerStrip key={t.id} track={t} returns={layout.returns} />
          ))}
        </div>
      )}
      {layout.master && (
        <div className="eth-mixer__master">
          <MixerStrip track={layout.master} returns={layout.returns} />
        </div>
      )}
    </div>
  );
}
