/**
 * The chains of an instrument / audio effect / MIDI effect rack (Ableton-style), shown in the
 * rack's device card under its macros: the chain list (select, mute, solo, volume, pan,
 * rename/delete/zones from the context menu), "+ Chain", and the selected chain's devices
 * as nested device cards (drop a device card on a chain to move it there). The zone editor
 * sets the key / velocity / chain-selector ranges of the selected chain.
 */

import clsx from "clsx";
import { useMemo, useState, type DragEvent, type ReactNode } from "react";
import { Plus } from "lucide-react";
import type { BuiltinDeviceType, Device, DeviceId, RackChain, RackChainId, Zone } from "@/generated";
import { builtinDevice, useBuiltinTypes } from "@/features/devices/descriptors";
import { DEVICE_DRAG_TYPE } from "@/features/devices/chainUtils";
import { useGestureSender, useSend } from "@/features/devices/gesture";
import { Button, IconButton, Knob, NumberField, openContextMenu, Select, type SelectOption } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import { fitsChain, rackTypeOf, VOL_MAX, VOL_MIN } from "./model";
import "./racks.css";

const byOrder = <T extends { order: string; id: string }>(a: T, b: T) =>
  a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1;

export interface RenderDeviceProps {
  device: Device;
  prev: DeviceId | null;
  moveRightBefore: DeviceId | null | undefined;
  onDropBefore(dragged: DeviceId): void;
  onMove(before: DeviceId | null): void;
}

export interface RackPanelProps {
  rack: Device;
  /** Draws a nested device card (the device panel, passed in to avoid an import cycle). */
  renderDevice(props: RenderDeviceProps): ReactNode;
}

export function RackPanel({ rack, renderDevice }: RackPanelProps) {
  const send = useSend();
  const chainsTable = useProjectStore((s) => s.project?.rack_chains);
  const devicesTable = useProjectStore((s) => s.project?.devices);
  const chains = useMemo(() => Object.values(chainsTable ?? {}).filter((c) => c.rack === rack.id).sort(byOrder), [chainsTable, rack.id]);
  const [picked, setPicked] = useState<RackChainId | null>(null);
  const selected = chains.find((c) => c.id === picked) ?? chains[0] ?? null;
  const devices = useMemo(
    () => (selected ? Object.values(devicesTable ?? {}).filter((d) => d.chain === selected.id).sort(byOrder) : []),
    [devicesTable, selected],
  );
  const rackType = rackTypeOf(rack);
  if (!rackType) return null;

  const addChain = () => {
    const id = newId();
    setPicked(id);
    void send(cmd("Rack", { type: "AddChain", id, rack: rack.id, name: null, before: null }));
  };
  const moveInto = (chain: RackChainId, id: DeviceId, before: DeviceId | null) =>
    void send(cmd("Rack", { type: "MoveDevice", id, chain, before }));

  return (
    <div className="eth-rack" data-rack={rack.id} aria-label={`${rack.name} chains`}>
      <div className="eth-rack__chains" role="listbox" aria-label="Chains">
        {chains.map((c) => (
          <ChainRow
            key={c.id}
            chain={c}
            selected={c.id === selected?.id}
            onSelect={() => setPicked(c.id)}
            onDropDevice={(id) => moveInto(c.id, id, null)}
          />
        ))}
        {chains.length === 0 && <div className="eth-rack__empty">No chains yet: add one, or drop a device here.</div>}
        <Button size="sm" tone="ghost" className="eth-rack__add-chain" onClick={addChain}>
          <Plus /> Chain
        </Button>
      </div>
      {selected && (
        <div className="eth-rack__chain" aria-label={`${selected.name} devices`}>
          <ZoneEditor chain={selected} />
          <div className="eth-rack__devices" role="list">
            {devices.map((d, i) => (
              <div role="listitem" key={d.id} className="eth-rack__slot">
                {renderDevice({
                  device: d,
                  prev: devices[i - 1]?.id ?? null,
                  moveRightBefore: i === devices.length - 1 ? undefined : (devices[i + 2]?.id ?? null),
                  onDropBefore: (dragged) => moveInto(selected.id, dragged, d.id),
                  onMove: (before) => moveInto(selected.id, d.id, before),
                })}
              </div>
            ))}
            <ChainEnd rackType={rackType} chain={selected} onDrop={(id) => moveInto(selected.id, id, null)} empty={devices.length === 0} />
          </div>
        </div>
      )}
    </div>
  );
}

function ChainRow({ chain: c, selected, onSelect, onDropDevice }: { chain: RackChain; selected: boolean; onSelect(): void; onDropDevice(id: DeviceId): void }) {
  const send = useSend();
  const sender = useGestureSender();
  const [drop, setDrop] = useState(false);
  const mix = (m: { volume?: number; pan?: number; mute?: boolean; solo?: boolean }) =>
    cmd("Rack", { type: "SetChainMix", id: c.id, volume: m.volume ?? null, pan: m.pan ?? null, mute: m.mute ?? null, solo: m.solo ?? null });
  const volNorm = (Math.min(VOL_MAX, Math.max(VOL_MIN, c.volume)) - VOL_MIN) / (VOL_MAX - VOL_MIN);
  const onDragOver = (e: DragEvent) => {
    if (!e.dataTransfer.types.includes(DEVICE_DRAG_TYPE)) return;
    e.preventDefault();
    setDrop(true);
  };
  return (
    <div
      className={clsx("eth-rack__row", selected && "eth-rack__row--selected", c.mute && "eth-rack__row--muted", drop && "eth-rack__row--drop")}
      role="option"
      aria-selected={selected}
      aria-label={c.name}
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(e) => (e.key === "Enter" || e.key === " ") && onSelect()}
      onDragOver={onDragOver}
      onDragLeave={() => setDrop(false)}
      onDrop={(e) => {
        setDrop(false);
        const id = e.dataTransfer.getData(DEVICE_DRAG_TYPE);
        if (!id) return;
        e.preventDefault();
        onDropDevice(id);
      }}
      onContextMenu={(e) =>
        openContextMenu(e, [
          {
            label: "Rename Chain",
            onSelect: () => {
              const name = window.prompt("Chain name", c.name);
              if (name) void send(cmd("Rack", { type: "RenameChain", id: c.id, name }));
            },
          },
          { label: c.solo ? "Unsolo" : "Solo", onSelect: () => void send(mix({ solo: !c.solo })) },
          "separator",
          { label: "Delete Chain", danger: true, onSelect: () => void send(cmd("Rack", { type: "RemoveChain", id: c.id })) },
        ])
      }
    >
      <span className="eth-rack__name">{c.name}</span>
      <span className="eth-rack__buttons" onClick={(e) => e.stopPropagation()}>
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-rack__mute"
          active={c.mute}
          label={c.mute ? `Unmute ${c.name}` : `Mute ${c.name}`}
          icon={<span className="eth-rack__letter">M</span>}
          onClick={() => void send(mix({ mute: !c.mute }))}
        />
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-rack__solo"
          active={c.solo}
          label={c.solo ? `Unsolo ${c.name}` : `Solo ${c.name}`}
          icon={<span className="eth-rack__letter">S</span>}
          onClick={() => void send(mix({ solo: !c.solo }))}
        />
      </span>
      <span className="eth-rack__knobs" onClick={(e) => e.stopPropagation()}>
        <Knob
          size="sm"
          label="Vol"
          value={volNorm}
          defaultValue={(0 - VOL_MIN) / (VOL_MAX - VOL_MIN)}
          valueText={`${c.volume <= VOL_MIN ? "-inf" : c.volume.toFixed(1)} dB`}
          onChange={(v) => void sender.send(mix({ volume: v <= 0 ? -144 : Math.round((VOL_MIN + v * (VOL_MAX - VOL_MIN)) * 10) / 10 }))}
          onChangeStart={sender.begin}
          onChangeEnd={sender.end}
        />
        <Knob
          size="sm"
          label="Pan"
          bipolar
          value={(c.pan + 1) / 2}
          valueText={c.pan === 0 ? "C" : `${Math.round(Math.abs(c.pan) * 100)}${c.pan < 0 ? "L" : "R"}`}
          onChange={(v) => void sender.send(mix({ pan: Math.round((v * 2 - 1) * 100) / 100 }))}
          onChangeStart={sender.begin}
          onChangeEnd={sender.end}
        />
      </span>
    </div>
  );
}

/** Key / velocity / chain-selector zones of a chain. */
function ZoneEditor({ chain: c }: { chain: RackChain }) {
  const send = useSend();
  const set = (field: "keys" | "velocities" | "select", z: Zone) =>
    void send(
      cmd("Rack", {
        type: "SetChainZones",
        id: c.id,
        keys: field === "keys" ? z : null,
        velocities: field === "velocities" ? z : null,
        select: field === "select" ? z : null,
      }),
    );
  const row = (field: "keys" | "velocities" | "select", label: string) => {
    const z = c[field];
    return (
      <span className="eth-rack__zone" key={field}>
        <span className="eth-rack__zone-label">{label}</span>
        <NumberField
          size="sm"
          aria-label={`${c.name} ${label} low`}
          value={z.lo}
          min={0}
          max={z.hi}
          step={1}
          onChange={(v) => set(field, { lo: Math.round(v), hi: z.hi })}
        />
        <NumberField
          size="sm"
          aria-label={`${c.name} ${label} high`}
          value={z.hi}
          min={z.lo}
          max={127}
          step={1}
          onChange={(v) => set(field, { lo: z.lo, hi: Math.round(v) })}
        />
      </span>
    );
  };
  return (
    <div className="eth-rack__zones" aria-label={`${c.name} zones`}>
      {row("keys", "Keys")}
      {row("velocities", "Vel")}
      {row("select", "Chain Select")}
    </div>
  );
}

/** End of the chain: drop target and the "add device" picker (types that fit the rack). */
function ChainEnd({ rackType, chain, onDrop, empty }: { rackType: BuiltinDeviceType; chain: RackChain; onDrop(id: DeviceId): void; empty: boolean }) {
  const send = useSend();
  const types = useBuiltinTypes().filter((d) => fitsChain(rackType, d));
  const [drop, setDrop] = useState(false);
  const options: SelectOption<string>[] = types.flatMap((d) =>
    d.device_type.type === "Builtin" ? [{ value: d.device_type.device as string, label: d.name }] : [],
  );
  return (
    <div
      className={clsx("eth-rack__end", drop && "eth-rack__end--drop")}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes(DEVICE_DRAG_TYPE)) return;
        e.preventDefault();
        setDrop(true);
      }}
      onDragLeave={() => setDrop(false)}
      onDrop={(e) => {
        setDrop(false);
        const id = e.dataTransfer.getData(DEVICE_DRAG_TYPE);
        if (!id) return;
        e.preventDefault();
        onDrop(id);
      }}
    >
      {empty && <span className="eth-rack__empty">Empty chain: add a device or drop one here.</span>}
      <Select
        size="sm"
        className="eth-rack__add-device"
        aria-label={`Add device to ${chain.name}`}
        value=""
        placeholder="+ Add device…"
        options={options}
        onChange={(v) =>
          void send(
            cmd("Rack", {
              type: "InsertDevice",
              id: newId(),
              chain: chain.id,
              device: { type: "Builtin", device: builtinDevice(v as BuiltinDeviceType) },
              before: null,
            }),
          )
        }
      />
    </div>
  );
}
