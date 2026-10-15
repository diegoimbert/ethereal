import clsx from "clsx";
import { useContext, useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import type { MidiMapping, MidiMapTarget, Project } from "@/generated";
import { Badge, Button, IconButton, NumberField, openContextMenu, Select, Toggle } from "@/kit";
import { useProjectStore, useSelectionStore } from "@/state";
import { cmd, TransportContext, type EngineTransport } from "@/transport";
import { canLearn, isSimulated, mockNotice } from "./host";
import { Trans, useT } from "@/i18n";
import { useMidiLearnStore } from "./store";
import {
  describeSource,
  describeTarget,
  MODE_OPTIONS,
  modeValue,
  parseMode,
  sourceMatches,
  TRANSPORT_ACTIONS,
  transportTargetLabel,
  type ModeValue,
  type ParamNames,
} from "./targets";
import { learn } from "./useMidiMode";

function sortedMappings(project: Project | null): MidiMapping[] {
  if (!project) return [];
  return Object.values(project.midi_mappings).sort((a, b) => describeSource(a.source).localeCompare(describeSource(b.source)));
}

/** Param names of the devices used by mappings (descriptors fetched on demand). */
function useParamNames(transport: EngineTransport, mappings: ReadonlyArray<MidiMapping>): ParamNames {
  const [names, setNames] = useState<Record<string, Record<number, string>>>({});
  const devices = useMemo(
    () => [...new Set(mappings.flatMap((m) => (m.target.type === "Param" && m.target.target.type === "DeviceParam" ? [m.target.target.device] : [])))],
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
  const t = useT();
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
  const remove = () => void transport.send(cmd("MidiMap", { type: "Unmap", ids: [mapping.id] })).catch((e: unknown) => console.warn("unmap failed", e));
  return (
    <li
      className={clsx("eth-midi__row", active && "eth-midi__row--active", fresh && "eth-midi__row--fresh")}
      data-mapping={mapping.id}
      onContextMenu={(e) =>
        openContextMenu(e, [
          { label: t("midi_learn.row.learn_again"), onSelect: () => void learn(transport, mapping.target) },
          "separator",
          { label: t("midi_learn.row.remove"), danger: true, onSelect: remove },
        ])
      }
    >
      <div className="eth-midi__row-head">
        <span className="eth-midi__target eth-user-text" title={label}>
          {label}
        </span>
        <IconButton size="sm" tone="ghost" label={t("midi_learn.row.remove_named", { name: label })} icon="✕" onClick={remove} />
      </div>
      <div className="eth-midi__source">{describeSource(mapping.source)}</div>
      <div className="eth-midi__fields">
        <Select
          size="sm"
          aria-label={t("midi_learn.row.mode_of", { name: label })}
          options={MODE_OPTIONS.map((o) => ({ value: o.value, label: t(o.label) }))}
          value={modeValue(mapping.mode)}
          onChange={(mode) => edit({ mode })}
        />
        <NumberField
          size="sm"
          aria-label={t("midi_learn.row.min_of", { name: label })}
          title={t("midi_learn.row.min_title")}
          value={pct(mapping.min)}
          min={0}
          max={100}
          unit="%"
          onChange={(v) => edit({ min: v / 100 })}
        />
        <NumberField
          size="sm"
          aria-label={t("midi_learn.row.max_of", { name: label })}
          title={t("midi_learn.row.max_title")}
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
  // Tolerate a missing `<TransportProvider>` (bare App shell in tests).
  const transport = useContext(TransportContext)?.transport ?? null;
  const t = useT();
  if (!transport) {
    return (
      <div className="eth-midi" data-feature="midi-learn">
        <p className="eth-midi__hint">{t("midi_learn.no_engine")}</p>
      </div>
    );
  }
  return <ConnectedPanel transport={transport} />;
}

function ConnectedPanel({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const project = useProjectStore((s) => s.project);
  const mappings = useProjectStore(useShallow((s) => sortedMappings(s.project)));
  const selectedTrack = useSelectionStore((s) => s.selectedTrack);
  // Learn events, MIDI mode and the port connection live in `MidiLearnRoot` (app-wide).
  const { enabled, learning, learned, activity, setEnabled } = useMidiLearnStore();
  const supported = canLearn(transport);
  const names = useParamNames(transport, mappings);

  const selectedName = selectedTrack ? project?.tracks[selectedTrack]?.name : undefined;
  const extraOptions = useMemo(
    () => [
      ...TRANSPORT_ACTIONS.map((a) => ({ value: `transport:${a.action}` as ExtraTarget, label: transportTargetLabel(a.action) })),
      ...(selectedName
        ? (["arm", "mute", "solo"] as const).map((k) => ({
            value: k as ExtraTarget,
            label: t(k === "arm" ? "midi_learn.target.arm" : k === "mute" ? "midi_learn.target.mute" : "midi_learn.target.solo", { name: selectedName }),
          }))
        : []),
    ],
    [selectedName, t],
  );

  return (
    <div className="eth-midi" data-feature="midi-learn">
      <div className="eth-midi__header">
        <Toggle
          checked={enabled && supported}
          disabled={!supported || !project}
          onChange={setEnabled}
          label={t("midi_learn.midi_mode")}
          aria-label={t("midi_learn.midi_mode")}
        />
        {learning && <Badge tone="accent">{t("midi_learn.learning")}</Badge>}
      </div>

      {supported && isSimulated(transport) && (
        <p className="eth-midi__notice" role="note">
          {mockNotice()}
        </p>
      )}

      {!supported ? (
        <p className="eth-midi__notice" role="note">
          {t("midi_learn.web_notice_panel")}
        </p>
      ) : learning ? (
        <div className="eth-midi__status" role="status">
          <span>
            <Trans k="midi_learn.move_control" values={{ target: <strong className="eth-user-text">{describeTarget(learning, project, names)}</strong> }} />
          </span>
          <Button size="sm" onClick={() => void learn(transport, null)}>
            {t("midi_learn.cancel")}
          </Button>
        </div>
      ) : (
        <p className="eth-midi__hint">{enabled ? t("midi_learn.hint_on") : t("midi_learn.hint_off")}</p>
      )}

      {supported && enabled && (
        <Select
          size="sm"
          aria-label={t("midi_learn.learn_other")}
          options={extraOptions}
          value={"" as ExtraTarget}
          placeholder={t("midi_learn.learn_other_placeholder")}
          onChange={(v) => {
            const target = extraTarget(v, selectedTrack);
            if (target) void learn(transport, target);
          }}
        />
      )}

      {mappings.length === 0 ? (
        <p className="eth-midi__empty">{t("midi_learn.no_mappings")}</p>
      ) : (
        <ul className="eth-midi__list" aria-label={t("midi_learn.mappings")}>
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
          {t("midi_learn.last_input", { source: describeSource(activity) })}
        </p>
      )}
    </div>
  );
}
