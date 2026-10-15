import { useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { Device, DeviceId, DrumPad, Track } from "@/generated";
import { useSend } from "@/features/devices/gesture";
import { useSelectedTrack } from "@/features/devices/selectedTrack";
import { useT } from "@/i18n";
import { Button, Select } from "@/kit";
import { useDevicesOfTrack, useProjectStore } from "@/state";
import { newId } from "@/transport";
import { useDrumFocus } from "./focusStore";
import { PadGrid } from "./PadGrid";
import { EmptyPadSettings, PadSettings } from "./PadSettings";
import { bankOf, DEFAULT_BANK_START, gridNotes, insertRackCommand, racksOf } from "./padUtils";
import "./drumRack.css";

/** The drum racks of the selected track (pads, pad chains): no longer in the editor drawer (e6d68591); the tests drive the rack editor through it. */
export function DrumRackTab() {
  const t = useT();
  const track = useSelectedTrack();
  if (!track) {
    return (
      <div className="eth-drum-rack eth-drum-rack--empty" data-feature="drum-rack" data-testid="drum-rack-view">
        {t("drum_rack.view.no_project")}
      </div>
    );
  }
  return <TrackRacks key={track.id} track={track} />;
}

function TrackRacks({ track }: { track: Track }) {
  const t = useT();
  const send = useSend();
  const chain = useDevicesOfTrack(track.id);
  const racks = racksOf(chain);
  const [rackId, setRackId] = useState<DeviceId | null>(null);
  const rack = racks.find((r) => r.id === rackId) ?? racks[0];
  const midi = track.kind === "Midi";

  return (
    <div className="eth-drum-rack" data-feature="drum-rack" data-testid="drum-rack-view">
      <div className="eth-drum-rack__toolbar">
        <span className="eth-drum-rack__track eth-user-text">{track.name}</span>
        {racks.length > 1 && (
          <Select
            size="sm"
            aria-label={t("drum_rack.view.picker")}
            options={racks.map((r, i) => ({
              value: r.id,
              label: `${r.name} ${i + 1}`,
              userText: true,
            }))}
            value={rack!.id}
            onChange={setRackId}
          />
        )}
        {midi && (
          <Button size="sm" onClick={() => void send(insertRackCommand(track.id, newId(), chain[0]?.id ?? null))}>
            {t("drum_rack.view.add")}
          </Button>
        )}
      </div>
      {rack ? (
        <RackEditor key={rack.id} rack={rack} />
      ) : (
        <p className="eth-drum-rack__hint">{midi ? t("drum_rack.view.empty") : t("drum_rack.view.midi_only")}</p>
      )}
    </div>
  );
}

/** One drum rack's editor: pads, pad chains, choke groups (also the rack's device window). */
export function RackEditor({ rack }: { rack: Device }) {
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
  // A pad shown from elsewhere (the beat editor's rows): select it, on its bank.
  const focus = useDrumFocus((s) => (s.focus?.rack === rack.id ? s.focus : null));
  // Adjusted while rendering (not in an effect) when a new focus arrives.
  const [seenFocus, setSeenFocus] = useState<typeof focus>(null);
  if (focus !== seenFocus) {
    setSeenFocus(focus);
    if (focus) {
      setSelected(focus.note);
      setBank(bankOf(focus.note));
    }
  }
  // Never nothing selected: the chosen note while it is on screen, else the bank's first pad
  // (lowest note), else its first note (bottom left).
  const onScreen = gridNotes(bank);
  const shown = selected !== null && onScreen.includes(selected) ? selected : (pads.find((p) => onScreen.includes(p.note))?.note ?? bank);
  const pad = pads.find((p) => p.note === shown);
  return (
    <div className="eth-drum-rack__rack" aria-label={rack.name}>
      <PadGrid rack={rack} pads={pads} bank={bank} onBank={setBank} selected={shown} onSelect={setSelected} />
      {pad ? <PadSettings key={pad.id} pad={pad} /> : <EmptyPadSettings key={shown} rack={rack.id} note={shown} />}
    </div>
  );
}
