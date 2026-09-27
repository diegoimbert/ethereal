import { useShallow } from "zustand/react/shallow";
import type { BuiltinDeviceType, Device, DeviceId, DrumPad, ParamScale } from "@/generated";
import { groupParams } from "@/features/devices/chainUtils";
import { builtinDevice, useBuiltinTypes, useDescriptor } from "@/features/devices/descriptors";
import { useGestureSender, useSend } from "@/features/devices/gesture";
import { ParamControl } from "@/features/devices/ParamControl";
import { scaleToNormalized, scaleToPlain } from "@/features/devices/paramScale";
import { SampleSlot } from "@/features/devices/SampleSlot";
import { Button, Knob, NumberField, openContextMenu, Select, TextInput } from "@/kit";
import { devicesOfPad, useProjectStore } from "@/state";
import { cmd, newId } from "@/transport";
import { CHOKE_OPTIONS, noteName } from "./padUtils";
import { useDrumSolo } from "./soloStore";

/** Pad volume uses the track-fader law (-144 = silence .. +6 dB). */
const VOLUME: { scale: ParamScale; min: number; max: number } = {
  scale: { type: "Fader" },
  min: -144,
  max: 6,
};

function formatDb(db: number): string {
  return db <= -144 ? "-inf dB" : `${db.toFixed(1)} dB`;
}

function formatPan(pan: number): string {
  if (Math.abs(pan) < 0.005) return "C";
  return `${Math.round(Math.abs(pan) * 100)}${pan < 0 ? "L" : "R"}`;
}

/** Settings and device chain of the selected pad. */
export function PadSettings({ pad }: { pad: DrumPad }) {
  const send = useSend();
  const sender = useGestureSender();
  const soloed = useDrumSolo((s) => s.soloed.has(pad.id));
  const setSolo = useDrumSolo((s) => s.set);
  const rename = (name: string) => {
    const trimmed = name.trim();
    if (trimmed && trimmed !== pad.name) void send(cmd("DrumRack", { type: "RenamePad", id: pad.id, name: trimmed }));
  };
  return (
    <section className="eth-drum-rack__pad" aria-label={`Pad ${pad.name}`} data-testid="pad-settings">
      <div className="eth-drum-rack__pad-controls">
        <TextInput
          key={pad.name}
          size="sm"
          aria-label="Pad name"
          defaultValue={pad.name}
          onBlur={(e) => rename(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") rename(e.currentTarget.value);
          }}
        />
        <label className="eth-drum-rack__field">
          <span className="eth-drum-rack__label">Note</span>
          <NumberField
            size="sm"
            aria-label="Pad note"
            value={pad.note}
            min={0}
            max={127}
            onChange={(note) =>
              void send(
                cmd("DrumRack", {
                  type: "SetPadNote",
                  id: pad.id,
                  note: Math.round(note),
                }),
              )
            }
          />
          <span className="eth-drum-rack__hint">{noteName(pad.note)}</span>
        </label>
        <label className="eth-drum-rack__field">
          <span className="eth-drum-rack__label">Choke</span>
          <Select
            size="sm"
            aria-label="Choke group"
            options={CHOKE_OPTIONS}
            value={pad.choke_group === null ? "" : String(pad.choke_group)}
            onChange={(v) =>
              void send(
                cmd("DrumRack", {
                  type: "SetChokeGroup",
                  id: pad.id,
                  group: v === "" ? null : Number(v),
                }),
              )
            }
          />
        </label>
        <div className="eth-drum-rack__knob">
          <Knob
            size="sm"
            label="Pad volume"
            value={scaleToNormalized(VOLUME.scale, VOLUME.min, VOLUME.max, pad.volume)}
            defaultValue={scaleToNormalized(VOLUME.scale, VOLUME.min, VOLUME.max, 0)}
            valueText={formatDb(pad.volume)}
            onChange={(n) =>
              void sender.send(
                cmd("DrumRack", {
                  type: "SetPadVolume",
                  id: pad.id,
                  volume: scaleToPlain(VOLUME.scale, VOLUME.min, VOLUME.max, n),
                }),
              )
            }
            onChangeStart={sender.begin}
            onChangeEnd={sender.end}
          />
          <span className="eth-drum-rack__hint">{formatDb(pad.volume)}</span>
        </div>
        <div className="eth-drum-rack__knob">
          <Knob
            size="sm"
            bipolar
            label="Pad pan"
            value={(pad.pan + 1) / 2}
            valueText={formatPan(pad.pan)}
            onChange={(n) =>
              void sender.send(
                cmd("DrumRack", {
                  type: "SetPadPan",
                  id: pad.id,
                  pan: n * 2 - 1,
                }),
              )
            }
            onChangeStart={sender.begin}
            onChangeEnd={sender.end}
          />
          <span className="eth-drum-rack__hint">{formatPan(pad.pan)}</span>
        </div>
        <Button
          size="sm"
          active={pad.mute}
          aria-label="Mute pad"
          onClick={() =>
            void send(
              cmd("DrumRack", {
                type: "SetPadMute",
                id: pad.id,
                mute: !pad.mute,
              }),
            )
          }
        >
          M
        </Button>
        <Button
          size="sm"
          active={soloed}
          aria-label="Solo pad"
          onClick={() => {
            setSolo(pad.id, !soloed);
            void send(
              cmd("DrumRack", {
                type: "SetPadSolo",
                id: pad.id,
                solo: !soloed,
              }),
            );
          }}
        >
          S
        </Button>
        <Button
          size="sm"
          tone="danger"
          aria-label="Delete pad"
          onClick={() => void send(cmd("DrumRack", { type: "RemovePad", id: pad.id }))}
        >
          Delete
        </Button>
      </div>
      <PadChain pad={pad} />
    </section>
  );
}

/** The pad's device chain: generic param UI, add/move/bypass/remove. */
function PadChain({ pad }: { pad: DrumPad }) {
  const send = useSend();
  const chain = useProjectStore(useShallow((s) => (s.project ? devicesOfPad(s.project, pad.id) : [])));
  const types = useBuiltinTypes().filter((d) => d.device_type.type === "Builtin" && d.device_type.device !== "DrumRack");
  const add = (type: BuiltinDeviceType) =>
    void send(
      cmd("DrumRack", {
        type: "InsertDevice",
        id: newId(),
        pad: pad.id,
        device: { type: "Builtin", device: builtinDevice(type) },
        before: null,
      }),
    );
  const move = (id: DeviceId, before: DeviceId | null) => void send(cmd("DrumRack", { type: "MoveDevice", id, pad: pad.id, before }));
  return (
    <div className="eth-drum-rack__chain" role="list" aria-label={`${pad.name} chain`}>
      {chain.map((d, i) => (
        <div role="listitem" key={d.id}>
          <PadDevice
            device={d}
            onLeft={i > 0 ? () => move(d.id, chain[i - 1]!.id) : undefined}
            onRight={i < chain.length - 1 ? () => move(d.id, chain[i + 2]?.id ?? null) : undefined}
          />
        </div>
      ))}
      <Select<string>
        size="sm"
        className="eth-drum-rack__add"
        aria-label="Add device to pad"
        value=""
        options={[
          { value: "", label: "+ Add device…" },
          ...types.flatMap((d) => (d.device_type.type === "Builtin" ? [{ value: d.device_type.device, label: d.name }] : [])),
        ]}
        onChange={(v) => v && add(v as BuiltinDeviceType)}
      />
      {chain.length === 0 && <p className="eth-drum-rack__hint">Empty pad: drop a sample on it or add a device.</p>}
    </div>
  );
}

function PadDevice({ device, onLeft, onRight }: { device: Device; onLeft?: () => void; onRight?: () => void }) {
  const send = useSend();
  const sender = useGestureSender();
  const { descriptor } = useDescriptor(device);
  const toggle = () =>
    void send(
      cmd("Device", {
        type: "SetEnabled",
        id: device.id,
        enabled: !device.enabled,
      }),
    );
  const remove = () => void send(cmd("Device", { type: "Remove", id: device.id }));
  return (
    <section className="eth-drum-rack__device" aria-label={device.name} data-device={device.id}>
      <header
        className="eth-drum-rack__device-header"
        onContextMenu={(e) =>
          openContextMenu(e, [
            { label: device.enabled ? "Bypass" : "Enable", onSelect: toggle },
            "separator",
            { label: "Delete Device", danger: true, onSelect: remove },
          ])
        }
      >
        <Button
          size="sm"
          tone="ghost"
          active={device.enabled}
          aria-label={device.enabled ? `Bypass ${device.name}` : `Enable ${device.name}`}
          onClick={toggle}
        >
          ⏻
        </Button>
        <span className="eth-drum-rack__device-name">{device.name}</span>
        <Button size="sm" tone="ghost" aria-label={`Move ${device.name} left`} disabled={!onLeft} onClick={onLeft}>
          ◀
        </Button>
        <Button size="sm" tone="ghost" aria-label={`Move ${device.name} right`} disabled={!onRight} onClick={onRight}>
          ▶
        </Button>
        <Button size="sm" tone="ghost" aria-label={`Remove ${device.name}`} onClick={remove}>
          ✕
        </Button>
      </header>
      <SampleSlot device={device} />
      <div className="eth-drum-rack__params">
        {descriptor
          ? groupParams(descriptor.params).flatMap(({ params }) =>
              params.map((p) => <ParamControl key={p.id} device={device} info={p} sender={sender} />),
            )
          : null}
      </div>
    </section>
  );
}
