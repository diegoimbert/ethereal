import { useShallow } from "zustand/react/shallow";
import type { Device, DeviceCategory, DeviceId, DrumPad, ParamScale } from "@/generated";
import { useBuiltinTypes, useDescriptor } from "@/features/devices/descriptors";
import { deviceCategory } from "@/features/devices/chainUtils";
import { ChainSections } from "@/features/devices/DeviceChain";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { deviceSpec, type LibraryDevicePayload } from "@/features/devices/library";
import { useGestureSender, useSend } from "@/features/devices/gesture";
import { scaleToNormalized, scaleToPlain } from "@/domain/paramScale";
import { useT } from "@/i18n";
import { Knob, Select } from "@/kit";
import { devicesOfPad, useProjectStore } from "@/state";
import { cmd, newId, nextGestureId, useTransport } from "@/transport";
import { chokeOptions, isSampler, noteName } from "./padUtils";

/** Pad volume uses the track-fader law (-144 = silence .. +6 dB). */
const VOLUME: { scale: ParamScale; min: number; max: number } = {
  scale: { type: "Fader" },
  min: -144,
  max: 6,
};

function formatDb(db: number): string {
  return db <= -144 ? "-inf db" : `${db.toFixed(1)} db`;
}

/** Whole semitones (the param's step), rounded away from `from` so any nudge moves it. */
function wholeSemis(st: number, from: number): number {
  return st > from ? Math.ceil(st - 1e-9) : Math.floor(st + 1e-9);
}

function formatSemis(st: number): string {
  return `${st > 0 ? "+" : ""}${st} st`;
}

function formatPan(pan: number): string {
  if (Math.abs(pan) < 0.005) return "C";
  return `${Math.round(Math.abs(pan) * 100)}${pan < 0 ? "L" : "R"}`;
}

/** Settings and device chain of the selected pad. */
export function PadSettings({ pad }: { pad: DrumPad }) {
  const t = useT();
  const send = useSend();
  const sender = useGestureSender();
  return (
    <section className="eth-drum-rack__pad" aria-label={t("drum_rack.pad.label", { name: pad.name })} data-testid="pad-settings">
      <div className="eth-drum-rack__pad-controls">
        {/* Control above, caption below: in line with the knobs' value readouts. */}
        <label className="eth-drum-rack__field eth-drum-rack__field--stacked">
          <Select
            size="sm"
            aria-label={t("drum_rack.pad.choke_group")}
            options={chokeOptions()}
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
          <span className="eth-drum-rack__caption">{t("drum_rack.pad.choke")}</span>
        </label>
        <div className="eth-drum-rack__knob">
          <Knob
            size="sm"
            label={t("drum_rack.pad.volume")}
            showValue
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
        </div>
        <div className="eth-drum-rack__knob">
          <Knob
            size="sm"
            bipolar
            label={t("drum_rack.pad.pan")}
            showValue
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
        </div>
        <PadSampleControls pad={pad} />
      </div>
      <PadChain pad={pad} />
    </section>
  );
}

/** `ether_devices::sampler::params::TRANSPOSE`. */
const SAMPLER_TRANSPOSE = 2;

/**
 * The pad's sample at a glance: its first sampler's tune
 * (transpose, as "Tune") (the preview is on the pad). Nothing for a pad without a sampler.
 */
function PadSampleControls({ pad }: { pad: DrumPad }) {
  const sampler = useProjectStore((s) => (s.project ? devicesOfPad(s.project, pad.id).find(isSampler) : undefined));
  if (!sampler) return null;
  return <PadTune sampler={sampler} />;
}

function PadTune({ sampler }: { sampler: Device }) {
  const t = useT();
  const sender = useGestureSender();
  const { descriptor } = useDescriptor(sampler);
  const info = descriptor?.params.find((p) => p.id === SAMPLER_TRANSPOSE);
  const tune = info ? Math.round(sampler.params[info.id] ?? info.default) : 0;
  const span = info ? Math.max(1, info.max - info.min) : 1;
  return (
    <>
      {info && (
        <div className="eth-drum-rack__knob">
          <Knob
            size="sm"
            bipolar
            label={t("drum_rack.pad.tune")}
            showValue
            value={(tune - info.min) / span}
            defaultValue={(info.default - info.min) / span}
            valueText={formatSemis(tune)}
            onChange={(n) =>
              void sender.send(
                cmd("Device", {
                  type: "SetParam",
                  device: sampler.id,
                  param: info.id,
                  value: wholeSemis(info.min + n * span, tune),
                }),
              )
            }
            onChangeStart={sender.begin}
            onChangeEnd={sender.end}
          />
        </div>
      )}
    </>
  );
}

/** Everything but a nested drum rack (plugins included). */
const acceptInPad = (p: LibraryDevicePayload) => !(p.device.type === "Builtin" && p.device.builtin === "DrumRack");

/** The pad's device chain, in sections like a track's in the inspector (shared component). */
function PadChain({ pad }: { pad: DrumPad }) {
  const t = useT();
  const send = useSend();
  const chain = useProjectStore(useShallow((s) => (s.project ? devicesOfPad(s.project, pad.id) : [])));
  const builtins = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const categoryOf = (d: Device) => deviceCategory(d, builtins, plugins);
  /** Where a new device of `category` goes: MIDI effects before the instrument, the
   *  instrument before the audio effects, audio effects last. */
  const insertBefore = (category: DeviceCategory): DeviceId | null => {
    if (category === "AudioEffect") return null;
    const after: DeviceCategory[] = category === "NoteEffect" ? ["Instrument", "AudioEffect"] : ["AudioEffect"];
    return chain.find((d) => after.includes(categoryOf(d)))?.id ?? null;
  };
  return (
    <div className="eth-devices eth-devices--stack eth-drum-rack__chain">
      <ChainSections
        devices={chain}
        categories={ALL_SECTIONS}
        layout="stack"
        label={t("drum_rack.pad.chain", { name: pad.name })}
        accept={acceptInPad}
        onAdd={(p, category, before) =>
          void send(
            cmd("DrumRack", {
              type: "InsertDevice",
              id: newId(),
              pad: pad.id,
              device: deviceSpec(p),
              before: before !== undefined ? before : insertBefore(category),
            }),
          )
        }
        onMove={(id, before) => void send(cmd("DrumRack", { type: "MoveDevice", id, pad: pad.id, before }))}
        pasteChain={{ type: "Pad", pad: pad.id }}
        // The pad is its devices: removing the last one deletes the pad (one undo step).
        onRemove={(d) => void send(chain.length <= 1 ? cmd("DrumRack", { type: "RemovePad", id: pad.id }) : cmd("Device", { type: "Remove", id: d.id }))}
      />
    </div>
  );
}

const ALL_SECTIONS: ReadonlyArray<DeviceCategory> = ["NoteEffect", "Instrument", "AudioEffect"];
const INSTRUMENT_ONLY: ReadonlyArray<DeviceCategory> = ["Instrument"];

/**
 * An empty note of the grid: a pad is its instrument, so only the Instrument section; adding
 * one creates the pad with it (one undo step).
 */
export function EmptyPadSettings({ rack, note }: { rack: DeviceId; note: number }) {
  const t = useT();
  const transport = useTransport();
  const create = async (p: LibraryDevicePayload) => {
    const gesture = nextGestureId();
    const pad = newId();
    try {
      await transport.send(cmd("DrumRack", { type: "AddPad", id: pad, rack, note, name: p.name }), { gesture });
      await transport.send(cmd("DrumRack", { type: "InsertDevice", id: newId(), pad, device: deviceSpec(p), before: null }), { gesture });
    } finally {
      transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
    }
  };
  return (
    <section className="eth-drum-rack__pad" aria-label={t("drum_rack.pad.empty", { note: noteName(note) })} data-testid="pad-settings">
      <div className="eth-devices eth-devices--stack eth-drum-rack__chain">
        <ChainSections
          devices={[]}
          categories={INSTRUMENT_ONLY}
          layout="stack"
          label={t("drum_rack.pad.chain", { name: noteName(note) })}
          accept={acceptInPad}
          onAdd={(p) => void create(p).catch((e: unknown) => console.warn("add instrument failed", e))}
          onMove={() => {}}
        />
      </div>
    </section>
  );
}
