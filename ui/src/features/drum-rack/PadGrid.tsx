import clsx from "clsx";
import { useState, type DragEvent, type MouseEvent } from "react";
import type { Device, DrumPad } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, resolveDroppedMedia } from "@/features/browser/dragPayload";
import { reportFailure } from "@/features/devices/gesture";
import { Button, openContextMenu, Select } from "@/kit";
import { devicesOfPad, useProjectStore } from "@/state";
import { cmd, newId, nextGestureId, useTransport } from "@/transport";
import { bankStarts, gridNotes, isSampler, noteName, sampleOf } from "./padUtils";
import { useDrumSolo } from "./soloStore";

const BANK_OPTIONS = bankStarts().map((s) => ({
  value: String(s),
  label: `${noteName(s)}–${noteName(Math.min(127, s + 15))}`,
}));

export interface PadGridProps {
  rack: Device;
  pads: DrumPad[];
  bank: number;
  onBank(bank: number): void;
  selected: number | null;
  onSelect(note: number): void;
}

/** 4×4 pads of one bank: click selects + auditions, drop a sample to load it. */
export function PadGrid({ rack, pads, bank, onBank, selected, onSelect }: PadGridProps) {
  const byNote = new Map(pads.map((p) => [p.note, p]));
  return (
    <div className="eth-drum-rack__pads">
      <div className="eth-drum-rack__bank">
        <Select size="sm" aria-label="Pad bank" options={BANK_OPTIONS} value={String(bank)} onChange={(v) => onBank(Number(v))} />
      </div>
      <div className="eth-drum-rack__grid" role="grid" aria-label="Drum pads">
        {gridNotes(bank).map((note, i) =>
          note === null ? (
            <div key={`empty-${i}`} className="eth-drum-rack__cell eth-drum-rack__cell--none" />
          ) : (
            <PadCell
              key={note}
              rack={rack}
              note={note}
              pad={byNote.get(note)}
              selected={selected === note}
              onSelect={() => onSelect(note)}
            />
          ),
        )}
      </div>
    </div>
  );
}

interface PadCellProps {
  rack: Device;
  note: number;
  pad: DrumPad | undefined;
  selected: boolean;
  onSelect(): void;
}

function PadCell({ rack, note, pad, selected, onSelect }: PadCellProps) {
  const transport = useTransport();
  const [over, setOver] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const soloed = useDrumSolo((s) => (pad ? s.soloed.has(pad.id) : false));
  const setSolo = useDrumSolo((s) => s.set);
  const firstSample = useProjectStore((s) => {
    if (!pad || !s.project) return null;
    const first = devicesOfPad(s.project, pad.id).find(isSampler);
    return first ? sampleOf(first) : null;
  });
  const send = (c: Parameters<typeof transport.send>[0]) => void reportFailure(transport.send(c));

  const audition = () => {
    if (firstSample)
      send(
        cmd("Media", {
          type: "Preview",
          source: { type: "Project", media: firstSample },
        }),
      );
  };

  const onDragOver = (e: DragEvent) => {
    if (!hasBrowserDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setOver(true);
  };

  /** Import (if needed) and load the sample on this pad: one undo step. */
  const onDrop = async (e: DragEvent) => {
    setOver(false);
    const payload = readBrowserDrag(e.dataTransfer);
    if (!payload) return;
    e.preventDefault();
    setError(null);
    onSelect();
    const gesture = nextGestureId();
    try {
      const media = await resolveDroppedMedia(transport, payload, { gesture });
      const project = useProjectStore.getState().project;
      const current = project ? Object.values(project.drum_pads).find((p) => p.rack === rack.id && p.note === note) : undefined;
      if (!current) {
        await transport.send(
          cmd("DrumRack", {
            type: "AddSamplePad",
            pad: newId(),
            device: newId(),
            rack: rack.id,
            note,
            media: media.id,
          }),
          { gesture },
        );
      } else {
        const chain = project ? devicesOfPad(project, current.id) : [];
        const sampler = chain.find(isSampler);
        await transport.send(
          sampler
            ? cmd("Device", {
                type: "SetSample",
                device: sampler.id,
                media: media.id,
              })
            : cmd("DrumRack", {
                type: "InsertDevice",
                id: newId(),
                pad: current.id,
                device: {
                  type: "Builtin",
                  device: {
                    type: "Sampler",
                    sample: media.id,
                    slices: { enabled: false, base_note: 36, markers: [] },
                  },
                },
                before: chain[0]?.id ?? null,
              }),
          { gesture },
        );
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
    }
  };

  const menu = (e: MouseEvent) => {
    onSelect();
    if (!pad) {
      openContextMenu(e, [
        {
          label: `Add Pad on ${noteName(note)}`,
          onSelect: () =>
            send(
              cmd("DrumRack", {
                type: "AddPad",
                id: newId(),
                rack: rack.id,
                note,
                name: null,
              }),
            ),
        },
      ]);
      return;
    }
    openContextMenu(e, [
      {
        label: pad.mute ? "Unmute" : "Mute",
        onSelect: () =>
          send(
            cmd("DrumRack", {
              type: "SetPadMute",
              id: pad.id,
              mute: !pad.mute,
            }),
          ),
      },
      {
        label: soloed ? "Unsolo" : "Solo",
        onSelect: () => {
          setSolo(pad.id, !soloed);
          send(cmd("DrumRack", { type: "SetPadSolo", id: pad.id, solo: !soloed }));
        },
      },
      {
        label: pad.choke_group === null ? "Choke Group 1" : "No Choke Group",
        onSelect: () =>
          send(
            cmd("DrumRack", {
              type: "SetChokeGroup",
              id: pad.id,
              group: pad.choke_group === null ? 1 : null,
            }),
          ),
      },
      "separator",
      {
        label: "Delete Pad",
        danger: true,
        onSelect: () => send(cmd("DrumRack", { type: "RemovePad", id: pad.id })),
      },
    ]);
  };

  const label = pad ? `${noteName(note)} ${pad.name}` : `${noteName(note)} (empty)`;
  return (
    <Button
      size="lg"
      role="gridcell"
      className={clsx(
        "eth-drum-rack__cell",
        pad ? "eth-drum-rack__cell--pad" : "eth-drum-rack__cell--empty",
        pad?.mute && "eth-drum-rack__cell--muted",
        over && "eth-drum-rack__cell--over",
      )}
      active={selected}
      aria-label={label}
      title={
        error ? `Load failed: ${error}` : pad ? `${pad.name} (${noteName(note)})` : `Drop a sample to create a pad on ${noteName(note)}`
      }
      data-note={note}
      data-pad={pad?.id}
      onClick={() => {
        onSelect();
        audition();
      }}
      onContextMenu={menu}
      onDragOver={onDragOver}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => void onDrop(e)}
    >
      <span className="eth-drum-rack__cell-name">{pad?.name ?? ""}</span>
      <span className="eth-drum-rack__cell-note">
        {noteName(note)}
        {pad?.choke_group != null && ` · G${pad.choke_group}`}
        {pad?.mute && " · M"}
        {soloed && " · S"}
      </span>
    </Button>
  );
}
