import { useState, type DragEvent } from "react";
import { Select, type SelectOption } from "@/kit";
import type { BuiltinDeviceType, Command, Device, DeviceDescriptor, DeviceId, Track, TrackId } from "@/generated";
import clsx from "clsx";
import { useDevicesOfTrack, useProjectStore, useSelectionStore, useTracksOrdered } from "@/state";
import { cmd, newId, useTransport, type EngineTransport } from "@/transport";
import { asOneStep } from "@/features/arrangement/editMath";
import { builtinDevice, fetchDescriptor, useBuiltinTypes } from "./descriptors";
import { DEVICE_DRAG_TYPE, insertableTypes } from "./chainUtils";
import { DeviceView } from "./DeviceView";
import { useSend } from "./gesture";
import { useSelectedTrack } from "./selectedTrack";
import "./devices.css";

function TrackPicker({ track }: { track: Track }) {
  const tracks = useTracksOrdered();
  const selectTrack = useSelectionStore((s) => s.selectTrack);
  return (
    <Select
      size="sm"
      className="eth-devices__track"
      aria-label="Track"
      value={track.id}
      onChange={(v) => selectTrack(v)}
      options={tracks.map((t) => ({ value: t.id, label: t.name }))}
    />
  );
}

/** Add-device options, grouped by category (in order of first appearance). */
function groupByCategory(types: ReadonlyArray<DeviceDescriptor>): SelectOption<string>[] {
  const order: string[] = [];
  for (const t of types) if (!order.includes(t.category)) order.push(t.category);
  return order.flatMap((category) =>
    types.flatMap((d) =>
      d.category === category && d.device_type.type === "Builtin"
        ? [{ value: d.device_type.device as string, label: d.name, group: CATEGORY_LABELS[category] ?? category }]
        : [],
    ),
  );
}

/**
 * The command adding an instrument to a chain (Ableton behaviour): it replaces the chain's
 * top-level instrument (built-in or plugin, by descriptor category), in place, as one undo
 * step; with none it goes first (its output feeds the effects). `insert(before)` builds the
 * new instrument's `Device::Insert`. A trailing instrument gets no notes (the engine only
 * forwards MIDI device to device) and would clear the new one's audio.
 */
async function addInstrumentCommand(
  transport: EngineTransport,
  devices: ReadonlyArray<Device>,
  insert: (before: DeviceId | null) => Command,
): Promise<Command> {
  for (const d of devices) {
    if (d.chain) continue;
    const descriptor = await fetchDescriptor(transport, d).catch(() => null);
    if (descriptor?.category === "Instrument")
      return asOneStep("Replace Instrument", [insert(d.id), cmd("Device", { type: "Remove", id: d.id })])!;
  }
  return insert(devices[0]?.id ?? null);
}

const CATEGORY_LABELS: Record<string, string> = { Instrument: "Instruments", AudioEffect: "Audio effects", NoteEffect: "MIDI effects" };

function AddDevice({ track, devices }: { track: Track; devices: ReadonlyArray<Device> }) {
  const send = useSend();
  const transport = useTransport();
  const types = insertableTypes(useBuiltinTypes(), track);
  const add = async (type: BuiltinDeviceType, category: DeviceDescriptor["category"]) => {
    const insert = (before: DeviceId | null): Command =>
      cmd("Device", { type: "Insert", id: newId(), track: track.id, device: { type: "Builtin", device: builtinDevice(type) }, before });
    void send(category === "Instrument" ? await addInstrumentCommand(transport, devices, insert) : insert(null));
  };
  return (
    <Select
      size="sm"
      className="eth-devices__add"
      aria-label="Add device"
      value=""
      placeholder="+ Add device…"
      onChange={(v) => {
        const d = types.find((t) => t.device_type.type === "Builtin" && t.device_type.device === v);
        if (d && d.device_type.type === "Builtin") void add(d.device_type.device, d.category);
      }}
      options={groupByCategory(types)}
    />
  );
}

export type ChainLayout = "row" | "stack";

function Chain({ track, layout, picker }: { track: Track; layout: ChainLayout; picker: boolean }) {
  const stack = layout === "stack";
  const send = useSend();
  const devices = useDevicesOfTrack(track.id);
  const [endDrop, setEndDrop] = useState(false);
  const moveTo = (id: DeviceId, before: DeviceId | null) =>
    void send(cmd("Device", { type: "Move", id, track: track.id, before }));

  const onEndDragOver = (e: DragEvent) => {
    if (!e.dataTransfer.types.includes(DEVICE_DRAG_TYPE)) return;
    e.preventDefault();
    setEndDrop(true);
  };
  const onEndDrop = (e: DragEvent) => {
    setEndDrop(false);
    const id = e.dataTransfer.getData(DEVICE_DRAG_TYPE);
    if (!id) return;
    e.preventDefault();
    moveTo(id, null);
  };

  return (
    <div className={clsx("eth-devices", stack && "eth-devices--stack")} data-feature="devices">
      {!stack && (
        <div className="eth-devices__toolbar">
          {picker && <TrackPicker track={track} />}
          <AddDevice track={track} devices={devices} />
        </div>
      )}
      <div className={clsx("eth-devices__chain", stack && "eth-devices__chain--stack")} role="list" aria-label={`${track.name} devices`}>
        {devices.map((d, i) => (
          <div role="listitem" key={d.id} className="eth-devices__slot">
            <DeviceView
              device={d}
              prev={devices[i - 1]?.id ?? null}
              moveRightBefore={i === devices.length - 1 ? undefined : (devices[i + 2]?.id ?? null)}
              onDropBefore={(dragged) => moveTo(dragged, d.id)}
              layout={layout}
            />
          </div>
        ))}
        <div
          className={endDrop ? "eth-devices__end eth-devices__end--drop" : "eth-devices__end"}
          onDragOver={onEndDragOver}
          onDragLeave={() => setEndDrop(false)}
          onDrop={onEndDrop}
        >
          {devices.length === 0 && (
            <div className="eth-devices__empty">
              <span className="eth-devices__empty-title">No devices yet</span>
              <span>Add one with “+ Add device”, or drop one here.</span>
            </div>
          )}
        </div>
      </div>
      {stack && (
        <div className="eth-devices__footer">
          <AddDevice track={track} devices={devices} />
        </div>
      )}
    </div>
  );
}

export interface DeviceChainProps {
  /** Show this track's chain (no track picker). Default: the selected track, with a picker. */
  track?: TrackId;
  /** "row" (default): cards side by side. "stack": full-width collapsible cards (inspector). */
  layout?: ChainLayout;
}

/** Device chain of a track (the selected one by default), with the generic param UI. */
export function DeviceChainView({ track: trackId, layout = "row" }: DeviceChainProps) {
  const selected = useSelectedTrack();
  const given = useProjectStore((s) => (trackId ? s.project?.tracks[trackId] : undefined));
  const track = trackId ? given : selected;
  if (!track) return <div className="eth-devices eth-devices--empty" data-feature="devices">No project loaded</div>;
  return <Chain key={track.id} track={track} layout={layout} picker={!trackId} />;
}
