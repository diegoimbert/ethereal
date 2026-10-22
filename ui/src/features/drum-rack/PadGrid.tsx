import clsx from "clsx";
import { Headphones, Play, Volume2, VolumeX } from "lucide-react";
import { additiveClick, muteOverriddenBySolo } from "@/domain/soloPlan";
import { useState, type DragEvent, type KeyboardEvent, type MouseEvent, type PointerEvent } from "react";
import type { Device, DrumPad } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, resolveDroppedMedia } from "@/features/browser/dragPayload";
import { reportFailure } from "@/features/devices/gesture";
import { useT } from "@/i18n";
import { Button, openContextMenu, Select } from "@/kit";
import { devicesOfPad, useProjectStore } from "@/state";
import { useShallow } from "zustand/react/shallow";
import { deviceCategory } from "@/features/devices/chainUtils";
import { useBuiltinTypes } from "@/features/devices/descriptors";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { cmd, newId, nextGestureId, useTransport } from "@/transport";
import { holdNote, stopPreview } from "@/features/piano-roll/notePreview";
import { bankOf, bankStarts, gridNotes, isSampler, noteName } from "./padUtils";
import { SliceToPads } from "./SliceToPads";
import { soloPad } from "./soloActions";
import { useDrumSolo } from "./soloStore";

const BANK_OPTIONS = bankStarts().map((s) => ({
  value: String(s),
  label: `${noteName(s)}–${noteName(Math.min(127, s + 15))}`,
  userText: true,
}));

export interface PadGridProps {
  rack: Device;
  pads: DrumPad[];
  bank: number;
  onBank(bank: number): void;
  selected: number | null;
  onSelect(note: number): void;
}

/** Audition velocity (MIDI). */
const AUDITION_VELOCITY = 100;

/** 4×4 pads of one bank: press selects + auditions (held), drop a sample to load it. */
export function PadGrid({ rack, pads, bank, onBank, selected, onSelect }: PadGridProps) {
  const t = useT();
  const byNote = new Map(pads.map((p) => [p.note, p]));
  // A pad is active when it has an instrument (one without reads as an empty note).
  const builtins = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const playing = useProjectStore(
    useShallow((s) =>
      s.project ? pads.filter((p) => devicesOfPad(s.project!, p.id).some((d) => deviceCategory(d, builtins, plugins) === "Instrument")).map((p) => p.id) : [],
    ),
  );
  return (
    <div className="eth-drum-rack__pads">
      <div className="eth-drum-rack__pads-bar">
        <div className="eth-drum-rack__bank">
          <Select size="sm" aria-label={t("drum_rack.pads.bank")} options={BANK_OPTIONS} value={String(bank)} onChange={(v) => onBank(Number(v))} />
        </div>
        <SliceToPads
          rack={rack}
          onSliced={(note) => {
            onBank(bankOf(note));
            onSelect(note);
          }}
        />
      </div>
      <div className="eth-drum-rack__grid" role="grid" aria-label={t("drum_rack.pads.grid")}>
        {gridNotes(bank).map((note, i) =>
          note === null ? (
            <div key={`empty-${i}`} className="eth-drum-rack__cell eth-drum-rack__cell--none" />
          ) : (
            <PadCell
              key={note}
              rack={rack}
              note={note}
              pad={byNote.get(note)}
              active={!!byNote.get(note) && playing.includes(byNote.get(note)!.id)}
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
  /** It has an instrument. */
  active: boolean;
  selected: boolean;
  onSelect(): void;
}

function PadCell({ rack, note, pad, active, selected, onSelect }: PadCellProps) {
  const t = useT();
  const transport = useTransport();
  const [over, setOver] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const soloed = useDrumSolo((s) => (pad ? s.soloed.has(pad.id) : false));
  const [renaming, setRenaming] = useState(false);
  const send = (c: Parameters<typeof transport.send>[0]) => void reportFailure(transport.send(c));

  /**
   * Audition: the pad's note as a live note on the rack's track, held until the
   * pointer / key is released. It goes through the track's chain to the rack and the pad's
   * own chain (sampler slice, effects, choke group, pad mix) exactly like playback, armed
   * or not, transport stopped or not (`Recording::PreviewNote`, never recorded).
   */
  const startAudition = () => {
    if (pad) holdNote(transport, rack.track, note, AUDITION_VELOCITY);
  };
  const onPointerDown = (e: PointerEvent) => {
    if (e.button === 0) startAudition();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    if ((e.key === " " || e.key === "Enter") && !e.repeat) startAudition();
  };
  const onKeyUp = (e: KeyboardEvent) => {
    if (e.key === " " || e.key === "Enter") stopPreview();
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

  const rename = (name: string) => {
    setRenaming(false);
    // Empty: back to its default name (its note, "C1").
    const trimmed = name.trim() || noteName(note);
    if (pad && trimmed !== pad.name) send(cmd("DrumRack", { type: "RenamePad", id: pad.id, name: trimmed }));
  };

  const menu = (e: MouseEvent) => {
    onSelect();
    if (!pad) return;
    openContextMenu(e, [
      { label: t("drum_rack.pad.rename"), onSelect: () => setRenaming(true) },
      "separator",
      {
        label: pad.mute ? t("drum_rack.pad.unmute") : t("drum_rack.pad.mute"),
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
        label: soloed ? t("drum_rack.pad.unsolo") : t("drum_rack.pad.solo"),
        onSelect: () => soloPad(transport, pad, false),
      },
      {
        label: pad.choke_group === null ? t("drum_rack.pad.choke_group_1") : t("drum_rack.pad.no_choke_group"),
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
        label: t("drum_rack.pad.delete"),
        danger: true,
        onSelect: () => send(cmd("DrumRack", { type: "RemovePad", id: pad.id })),
      },
    ]);
  };

  const label = pad ? `${noteName(note)} ${pad.name}` : t("drum_rack.pad.empty_label", { note: noteName(note) });
  const noteLine = (
    <span className="eth-drum-rack__cell-note">
      <span className="eth-user-text">{noteName(note)}</span>
      {pad?.choke_group != null && ` · g${pad.choke_group}`}
    </span>
  );
  // Renaming in place: the cell becomes a field (Enter / blur keeps, Escape cancels).
  if (pad && renaming) {
    return (
      <div role="gridcell" className={clsx("eth-drum-rack__cell", "eth-drum-rack__cell--pad", "eth-drum-rack__cell--renaming")} data-note={note}>
        <input
          className="eth-drum-rack__rename eth-user-text"
          aria-label={t("drum_rack.pad.name")}
          defaultValue={pad.name}
          autoFocus
          onFocus={(e) => e.currentTarget.select()}
          onBlur={(e) => rename(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") rename(e.currentTarget.value);
            else if (e.key === "Escape") {
              e.stopPropagation();
              setRenaming(false);
            }
          }}
        />
        {noteLine}
      </div>
    );
  }
  // Mute / solo right on the pad, as on a track's header (same icons and states).
  const toggles = pad && active && (
    <span className="eth-drum-rack__toggles">
      <button
        type="button"
        className="eth-arr-header__toggle eth-arr-header__mute eth-drum-rack__toggle"
        aria-pressed={pad.mute}
        data-solo-override={muteOverriddenBySolo(pad.mute, soloed) || undefined}
        aria-label={t("drum_rack.pad.mute_label", { name: pad.name })}
        title={muteOverriddenBySolo(pad.mute, soloed) ? t("mixer.mute_overridden") : pad.mute ? t("drum_rack.pad.unmute") : t("drum_rack.pad.mute")}
        onClick={() => send(cmd("DrumRack", { type: "SetPadMute", id: pad.id, mute: !pad.mute }))}
      >
        {pad.mute ? <VolumeX /> : <Volume2 />}
      </button>
      <button
        type="button"
        className="eth-arr-header__toggle eth-arr-header__solo eth-drum-rack__toggle"
        aria-pressed={soloed}
        aria-label={t("drum_rack.pad.solo_label", { name: pad.name })}
        title={t("drum_rack.pad.solo_title")}
        onClick={(e) => soloPad(transport, pad, additiveClick(e))}
      >
        <Headphones />
      </button>
      <button
        type="button"
        className="eth-arr-header__toggle eth-drum-rack__toggle"
        aria-label={t("drum_rack.pad.preview_label", { name: pad.name })}
        title={t("drum_rack.pad.preview_title")}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
      >
        <Play />
      </button>
    </span>
  );
  return (
    <div className="eth-drum-rack__slot">
      <Button
        size="lg"
        role="gridcell"
        className={clsx(
          "eth-drum-rack__cell",
          active ? "eth-drum-rack__cell--pad" : "eth-drum-rack__cell--empty",
          pad?.mute && "eth-drum-rack__cell--muted",
          over && "eth-drum-rack__cell--over",
        )}
        active={selected}
        aria-label={label}
        title={
          error ? t("drum_rack.pad.load_failed", { error }) : pad ? `${pad.name} (${noteName(note)})` : t("drum_rack.pad.drop_hint", { note: noteName(note) })
        }
        data-note={note}
        data-pad={pad?.id}
        onClick={onSelect}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
        onKeyUp={onKeyUp}
        onContextMenu={menu}
        onDragOver={onDragOver}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => void onDrop(e)}
      >
        <span
          className="eth-drum-rack__cell-name eth-user-text"
          onDoubleClick={(e) => {
            if (!pad) return;
            e.stopPropagation();
            setRenaming(true);
          }}
          title={pad ? t("drum_rack.pad.rename_hint") : undefined}
        >
          {pad?.name ?? ""}
        </span>
        {noteLine}
      </Button>
      {toggles}
    </div>
  );
}
