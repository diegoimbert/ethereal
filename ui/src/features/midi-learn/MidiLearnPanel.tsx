import clsx from "clsx";
import { useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { MidiMapping, MidiMapTarget, Project } from "@/generated";
import { Badge, Button, IconButton, NumberField, openContextMenu, Select, Toggle } from "@/kit";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, useTransport, useTransportEvent, type EngineTransport } from "@/transport";
import { canLearn, WEB_NOTICE } from "./host";
import { useMidiLearnStore } from "./store";
import {
  describeSource,
  describeTarget,
  MODE_OPTIONS,
  modeValue,
  parseMode,
  sourceMatches,
  TRANSPORT_ACTIONS,
  type ModeValue,
  type ParamNames,
} from "./targets";
import { learn, useMidiMode } from "./useMidiMode";

function sortedMappings(project: Project | null): MidiMapping[] {
  if (!project) return [];
  return Object.values(project.midi_mappings).sort((a, b) => describeSource(a.source).localeCompare(describeSource(b.source)));
}

/** Param names of the devices used by mappings (descriptors fetched on demand). */
function useParamNames(transport: EngineTransport, mappings: ReadonlyArray<MidiMapping>): ParamNames {
  const [names, setNames] = useState<Record<string, Record<number, string>>>({});
  const devices = useMemo(
    () => [
      ...new Set(
        mappings.flatMap((m) => (m.target.type === "Param" && m.target.target.type === "DeviceParam" ? [m.target.target.device] : [])),
      ),
    ],
    [mappings],
  );
  useEffect(() => {
    let live = true;
    for (const device of devices) {
      if (names[device]) continue;
      transport
        .send(cmd("Device", { type: "GetDescriptor", device }))
        .then((reply) => {
          if (!live || reply.type !== "Descriptor") return;
          const byId = Object.fromEntries(reply.descriptor.params.map((p) => [p.id, p.name]));
          setNames((n) => ({ ...n, [device]: byId }));
        })
        .catch(() => undefined);
    }
    return () => {
      live = false;
    };
  }, [transport, devices, names]);
  return names;
}

const pct = (v: number) => Math.round(v * 100);

function MappingRow({
  mapping,
  label,
  active,
  fresh,
  transport,
}: {
  mapping: MidiMapping;
  label: string;
  active: boolean;
  fresh: boolean;
  transport: EngineTransport;
}) {
  const edit = (patch: { min?: number; max?: number; mode?: ModeValue }) =>
    void transport
      .send(
        cmd("MidiMap", {
          type: "Edit",
          id: mapping.id,
          min: patch.min ?? null,
          max: patch.max ?? null,
          mode: patch.mode ? parseMode(patch.mode) : null,
        }),
      )
      .catch((e: unknown) => console.warn("edit mapping failed", e));
  const remove = () =>
    void transport.send(cmd("MidiMap", { type: "Unmap", ids: [mapping.id] })).catch((e: unknown) => console.warn("unmap failed", e));
  return (
    <li
      className={clsx("eth-midi__row", active && "eth-midi__row--active", fresh && "eth-midi__row--fresh")}
      data-mapping={mapping.id}
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: "Learn again", onSelect: () => void learn(transport, mapping.target) },
          "separator",
          { label: "Remove mapping", danger: true, onSelect: remove },
        ])
      }
    >
      <div className="eth-midi__row-head">
        <span className="eth-midi__target" title={label}>
          {label}
        </span>
        <IconButton size="sm" tone="ghost" label={`Remove mapping ${label}`} icon="✕" onClick={remove} />
      </div>
      <div className="eth-midi__source">{describeSource(mapping.source)}</div>
      <div className="eth-midi__fields">
        <Select
          size="sm"
          aria-label={`Mode of ${label}`}
          options={MODE_OPTIONS}
          value={modeValue(mapping.mode)}
          onChange={(mode) => edit({ mode })}
        />
        <NumberField
          size="sm"
          aria-label={`Minimum of ${label}`}
          title="Output at the lowest control value (%); above the maximum inverts"
          value={pct(mapping.min)}
          min={0}
          max={100}
          unit="%"
          onChange={(v) => edit({ min: v / 100 })}
        />
        <NumberField
          size="sm"
          aria-label={`Maximum of ${label}`}
          title="Output at the highest control value (%)"
          value={pct(mapping.max)}
          min={0}
          max={100}
          unit="%"
          onChange={(v) => edit({ max: v / 100 })}
        />
      </div>
    </li>
  );
}

type ExtraTarget = "" | `transport:${string}` | "arm" | "mute" | "solo";

/** Targets without a clickable control here: transport actions and the selected track's arm. */
function extraTarget(v: ExtraTarget, track: string | null): MidiMapTarget | null {
  if (v.startsWith("transport:")) {
    const action = v.slice("transport:".length) as (typeof TRANSPORT_ACTIONS)[number]["action"];
    return { type: "Transport", action };
  }
  if (!track) return null;
  if (v === "arm") return { type: "TrackArm", track };
  if (v === "mute") return { type: "TrackMute", track };
  if (v === "solo") return { type: "TrackSolo", track };
  return null;
}

/** MIDI learn: MIDI mode switch, learn status and the list of mappings. */
export function MidiLearnPanel() {
  const transport = useTransport();
  const project = useProjectStore((s) => s.project);
  const mappings = useProjectStore(useShallow((s) => sortedMappings(s.project)));
  const selectedTrack = useSelectionStore((s) => s.selectedTrack);
  const { enabled, learning, learned, activity, setEnabled, onEvent } = useMidiLearnStore();
  const supported = canLearn(transport);
  const names = useParamNames(transport, mappings);

  useTransportEvent(onEvent);
  useMidiMode(transport, enabled && supported && !!project);

  // Connect the hardware ports (the host opens MIDI inputs when they are listed).
  useEffect(() => {
    if (supported) void transport.send(cmd("Recording", { type: "ListInputs" })).catch(() => undefined);
  }, [transport, supported]);

  const selectedName = selectedTrack ? project?.tracks[selectedTrack]?.name : undefined;
  const extraOptions = useMemo(
    () => [
      { value: "" as ExtraTarget, label: "Learn other target…" },
      ...TRANSPORT_ACTIONS.map((a) => ({ value: `transport:${a.action}` as ExtraTarget, label: `Transport · ${a.label}` })),
      ...(selectedName
        ? (["arm", "mute", "solo"] as const).map((k) => ({
            value: k as ExtraTarget,
            label: `${selectedName} · ${k === "arm" ? "Arm" : k === "mute" ? "Mute" : "Solo"}`,
          }))
        : []),
    ],
    [selectedName],
  );

  return (
    <div className="eth-midi" data-feature="midi-learn">
      <div className="eth-midi__header">
        <Toggle
          checked={enabled && supported}
          disabled={!supported || !project}
          onChange={setEnabled}
          label="MIDI mode"
          aria-label="MIDI mode"
        />
        {learning && <Badge tone="accent">Learning</Badge>}
      </div>

      {!supported ? (
        <p className="eth-midi__notice" role="note">
          {WEB_NOTICE}. Mappings saved in the project still work there.
        </p>
      ) : learning ? (
        <div className="eth-midi__status" role="status">
          <span>
            Move a control on your MIDI device to map <strong>{describeTarget(learning, project, names)}</strong>.
          </span>
          <Button size="sm" onClick={() => void learn(transport, null)}>
            Cancel
          </Button>
        </div>
      ) : (
        <p className="eth-midi__hint">
          {enabled ? "Click a highlighted control, then move a knob or press a key on your MIDI device." : "Turn on MIDI mode to map controls."}
        </p>
      )}

      {supported && enabled && (
        <Select
          size="sm"
          aria-label="Learn other target"
          options={extraOptions}
          value=""
          onChange={(v) => {
            const t = extraTarget(v, selectedTrack);
            if (t) void learn(transport, t);
          }}
        />
      )}

      {mappings.length === 0 ? (
        <p className="eth-midi__empty">No MIDI mappings.</p>
      ) : (
        <ul className="eth-midi__list" aria-label="MIDI mappings">
          {mappings.map((m) => (
            <MappingRow
              key={m.id}
              mapping={m}
              label={describeTarget(m.target, project, names)}
              active={!!activity && sourceMatches(m.source, activity)}
              fresh={m.id === learned}
              transport={transport}
            />
          ))}
        </ul>
      )}

      {activity && (
        <p className="eth-midi__activity" aria-live="off">
          Last input: {describeSource(activity)}
        </p>
      )}
    </div>
  );
}

