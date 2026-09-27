import { useState, type DragEvent } from "react";
import { Select, type SelectOption } from "@/kit";
import type { BuiltinDeviceType, DeviceDescriptor, DeviceId, Track } from "@/generated";
import { useDevicesOfTrack, useSelectionStore, useTracksOrdered } from "@/state";
import { cmd, newId } from "@/transport";
import { builtinDevice, useBuiltinTypes } from "./descriptors";
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

const CATEGORY_LABELS: Record<string, string> = { Instrument: "Instruments", AudioEffect: "Audio effects", NoteEffect: "MIDI effects" };

function AddDevice({ track, firstDevice }: { track: Track; firstDevice: DeviceId | null }) {
  const send = useSend();
  const types = insertableTypes(useBuiltinTypes(), track);
  const add = (type: BuiltinDeviceType, category: DeviceDescriptor["category"]) =>
    void send(
      cmd("Device", {
        type: "Insert",
        id: newId(),
        track: track.id,
        device: { type: "Builtin", device: builtinDevice(type) },
        // Instruments go first in the chain (their output feeds the effects).
        before: category === "Instrument" ? firstDevice : null,
      }),
    );
  return (
    <Select
      size="sm"
      className="eth-devices__add"
      aria-label="Add device"
      value=""
      placeholder="+ Add device…"
      onChange={(v) => {
        const d = types.find((t) => t.device_type.type === "Builtin" && t.device_type.device === v);
        if (d && d.device_type.type === "Builtin") add(d.device_type.device, d.category);
      }}
      options={groupByCategory(types)}
    />
  );
}

function Chain({ track }: { track: Track }) {
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
    <div className="eth-devices" data-feature="devices">
      <div className="eth-devices__toolbar">
        <TrackPicker track={track} />
        <AddDevice track={track} firstDevice={devices[0]?.id ?? null} />
      </div>
      <div className="eth-devices__chain" role="list" aria-label={`${track.name} devices`}>
        {devices.map((d, i) => (
          <div role="listitem" key={d.id} className="eth-devices__slot">
            <DeviceView
              device={d}
              prev={devices[i - 1]?.id ?? null}
              moveRightBefore={i === devices.length - 1 ? undefined : (devices[i + 2]?.id ?? null)}
              onDropBefore={(dragged) => moveTo(dragged, d.id)}
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
    </div>
  );
}

/** Device chain of the selected track, with the generic param UI. */
export function DeviceChainView() {
  const track = useSelectedTrack();
  if (!track) return <div className="eth-devices eth-devices--empty" data-feature="devices">No project loaded</div>;
  return <Chain key={track.id} track={track} />;
}
