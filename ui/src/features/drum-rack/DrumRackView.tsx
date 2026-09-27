import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Device, DeviceId, DrumPad, Track } from "@/generated";
import { useSend } from "@/features/devices/gesture";
import { useSelectedTrack } from "@/features/devices/selectedTrack";
import { Button, Select } from "@/kit";
import { useDevicesOfTrack, useProjectStore } from "@/state";
import { newId } from "@/transport";
import { PadGrid } from "./PadGrid";
import { PadSettings } from "./PadSettings";
import { bankOf, DEFAULT_BANK_START, insertRackCommand, isSampler, racksOf } from "./padUtils";
import { SliceEditor } from "./SliceEditor";
import "./drumRack.css";

/** Drum rack (pads, pad chains) and sampler slicing of the selected track. */
export function DrumRackTab() {
  const track = useSelectedTrack();
  if (!track) {
    return (
      <div className="eth-drum-rack eth-drum-rack--empty" data-feature="drum-rack" data-testid="drum-rack-view">
        No project loaded
      </div>
    );
  }
  return <TrackRacks key={track.id} track={track} />;
}

function TrackRacks({ track }: { track: Track }) {
  const send = useSend();
  const chain = useDevicesOfTrack(track.id);
  const racks = racksOf(chain);
  const samplers = chain.filter(isSampler);
  const [rackId, setRackId] = useState<DeviceId | null>(null);
  const [samplerId, setSamplerId] = useState<DeviceId | null>(null);
  const rack = racks.find((r) => r.id === rackId) ?? racks[0];
  const sampler = samplers.find((s) => s.id === samplerId) ?? samplers[0];
  const midi = track.kind === "Midi";

  return (
    <div className="eth-drum-rack" data-feature="drum-rack" data-testid="drum-rack-view">
      <div className="eth-drum-rack__toolbar">
        <span className="eth-drum-rack__track">{track.name}</span>
        {racks.length > 1 && (
          <Select
            size="sm"
            aria-label="Drum rack"
            options={racks.map((r, i) => ({
              value: r.id,
              label: `${r.name} ${i + 1}`,
            }))}
            value={rack!.id}
            onChange={setRackId}
          />
        )}
        {midi && (
          <Button size="sm" onClick={() => void send(insertRackCommand(track.id, newId(), chain[0]?.id ?? null))}>
            + Drum Rack
          </Button>
        )}
      </div>
      {rack ? (
        <RackEditor key={rack.id} rack={rack} />
      ) : (
        <p className="eth-drum-rack__hint">
          {midi
            ? "No drum rack on this track: add one, then drop samples from the Browser onto its pads."
            : "Drum racks go on MIDI tracks."}
        </p>
      )}
      {samplers.length > 1 && (
        <Select
          size="sm"
          aria-label="Sampler to slice"
          options={samplers.map((s, i) => ({
            value: s.id,
            label: `${s.name} ${i + 1}`,
          }))}
          value={sampler!.id}
          onChange={setSamplerId}
        />
      )}
      {sampler && <SliceEditor key={sampler.id} sampler={sampler} />}
    </div>
  );
}

function RackEditor({ rack }: { rack: Device }) {
  const pads: DrumPad[] = useProjectStore(
    useShallow((s) =>
      s.project
        ? Object.values(s.project.drum_pads)
            .filter((p) => p.rack === rack.id)
            .sort((a, b) => a.note - b.note)
        : [],
    ),
  );
  const [bank, setBank] = useState(() => (pads[0] ? bankOf(pads[0].note) : DEFAULT_BANK_START));
  const [selected, setSelected] = useState<number | null>(() => pads[0]?.note ?? null);
  const pad = pads.find((p) => p.note === selected);
  return (
    <div className="eth-drum-rack__rack" aria-label={rack.name}>
      <PadGrid rack={rack} pads={pads} bank={bank} onBank={setBank} selected={selected} onSelect={setSelected} />
      {pad ? (
        <PadSettings key={pad.id} pad={pad} />
      ) : (
        <p className="eth-drum-rack__hint">Select a pad. Drop a sample from the Browser on a pad to load it.</p>
      )}
    </div>
  );
}
