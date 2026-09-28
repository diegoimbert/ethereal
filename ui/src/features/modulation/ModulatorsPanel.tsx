/**
 * The modulators of a device (Bitwig-style): each one's panel is drawn by the shared
 * declarative renderer from its kind's `ModulatorDescriptor::layout`, its params edited with
 * `Modulation::SetModulatorParam` (one undo step per drag). A modulator's map button (or
 * drag handle) starts mapping: every param it may reach lights up. Its targets are listed
 * with their depth. Envelope followers pick an optional sidechain track.
 */

import clsx from "clsx";
import { useEffect, useMemo } from "react";
import { Crosshair, GripVertical, Plus, X } from "lucide-react";
import type { Device, DeviceDescriptor, ModulatorDescriptor, ModulatorKind, Modulator, Project } from "@/generated";
import { DeviceLayoutView } from "@/features/devices/layout";
import { useGestureSender, useSend, type GestureSender } from "@/features/devices/gesture";
import { sidechainSources } from "@/features/sidechain/routing";
import { IconButton, Knob, openContextMenu, Select } from "@/kit";
import { useProjectStore } from "@/state";
import { builtinDescriptor, cmd } from "@/transport";
import { addModulatorEntries, useModulatorKinds } from "./kinds";
import { MOD_DRAG_TYPE, canHostModulators, modulatorsOf, sourceSlot, useModMapping } from "./model";
import "./modulation.css";

/** "+ Modulator" button of a device header (track-chain devices only). */
export function AddModulatorButton({ device }: { device: Device }) {
  const kinds = useModulatorKinds();
  const send = useSend();
  if (!canHostModulators(device) || kinds.length === 0) return null;
  return (
    <IconButton
      size="sm"
      tone="ghost"
      className="eth-mod-add"
      label={`Add modulator to ${device.name}`}
      title="Add a modulator (LFO, envelope, follower, steps, random, keytrack, velocity)"
      icon={<Plus />}
      onClick={(e) => openContextMenu(e, addModulatorEntries(kinds, device, send))}
    />
  );
}

/** A `GestureSender` that turns the renderer's `Device::SetParam` into modulator param edits. */
function useModulatorSender(modulator: string): GestureSender {
  const base = useGestureSender();
  return useMemo<GestureSender>(
    () => ({
      send(command) {
        if (command.domain === "Device" && command.command.type === "SetParam") {
          const { param, value } = command.command;
          return base.send(cmd("Modulation", { type: "SetModulatorParam", modulator, param, value }));
        }
        return base.send(command);
      },
      begin: base.begin,
      end: base.end,
      dragging: base.dragging,
    }),
    [base, modulator],
  );
}

/** The modulators of `device` (renders nothing without any). */
export function ModulatorsPanel({ device }: { device: Device }) {
  const table = useProjectStore((s) => s.project?.modulators);
  const mods = useMemo(() => modulatorsOf(table, device.id), [table, device.id]);
  const kinds = useModulatorKinds();
  const stop = useModMapping((s) => s.stop);
  const mapping = useModMapping((s) => s.source !== null);
  useEffect(() => {
    if (!mapping) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && stop();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [mapping, stop]);
  if (mods.length === 0) return null;
  return (
    <div className="eth-mods" aria-label={`${device.name} modulators`}>
      {mods.map((m) => {
        const kind = kinds.find((k) => k.kind === m.kind);
        return kind ? <ModulatorCard key={m.id} host={device} modulator={m} kind={kind} /> : null;
      })}
    </div>
  );
}

function ModulatorCard({ host, modulator: m, kind }: { host: Device; modulator: Modulator; kind: ModulatorDescriptor }) {
  const send = useSend();
  const sender = useModulatorSender(m.id);
  const project = useProjectStore((s) => s.project);
  const source = useMemo(() => ({ type: "Modulator" as const, modulator: m.id }), [m.id]);
  const active = useModMapping((s) => s.source?.type === "Modulator" && s.source.modulator === m.id);
  const { start, stop } = useModMapping.getState();
  const pseudo = useMemo<Device>(
    () => ({
      id: m.id,
      track: host.track,
      order: m.order,
      name: m.name,
      enabled: true,
      kind: { type: "Builtin", device: { type: "Utility" } },
      params: m.params,
      sidechain: null,
      pad: null,
    }),
    [m, host.track],
  );
  const descriptor = useMemo<DeviceDescriptor>(
    () => ({
      device_type: { type: "Builtin", device: "Utility" },
      name: kind.name,
      category: "AudioEffect",
      params: kind.params,
      audio_inputs: 0,
      audio_outputs: 0,
      midi_input: false,
      sidechain_inputs: 0,
      ...(kind.layout ? { layout: kind.layout } : {}),
    }),
    [kind],
  );
  if (!project) return null;
  const slot = sourceSlot(project, source);
  const targets = Object.values(project.mod_mappings).filter((x) => x.source.type === "Modulator" && x.source.modulator === m.id);
  return (
    <section className={clsx("eth-mod", `eth-mod--slot-${slot}`, active && "eth-mod--mapping")} aria-label={m.name} data-modulator={m.id}>
      <header
        className="eth-mod__header"
        onContextMenu={(e) =>
          openContextMenu(e, [
            {
              label: "Rename",
              onSelect: () => {
                const name = window.prompt("Modulator name", m.name);
                if (name) void send(cmd("Modulation", { type: "RenameModulator", id: m.id, name }));
              },
            },
            "separator",
            { label: "Delete Modulator", danger: true, onSelect: () => void send(cmd("Modulation", { type: "RemoveModulator", id: m.id })) },
          ])
        }
      >
        <span
          className="eth-mod__grip"
          draggable
          role="button"
          tabIndex={-1}
          aria-label={`Drag ${m.name} onto a parameter`}
          title="Drag onto a parameter to modulate it"
          onDragStart={(e) => {
            e.dataTransfer.setData(MOD_DRAG_TYPE, JSON.stringify(source));
            e.dataTransfer.effectAllowed = "link";
            // Show the drop targets after the drag started (a DOM change in dragstart cancels it).
            window.setTimeout(() => start(source), 0);
          }}
          onDragEnd={() => stop()}
        >
          <GripVertical />
        </span>
        <span className="eth-mod__name">{m.name}</span>
        <IconButton
          size="sm"
          tone="ghost"
          active={active}
          label={active ? `Stop mapping ${m.name}` : `Map ${m.name}`}
          title="Map: click, then click the parameters to modulate (Shift keeps mapping, Esc stops)"
          icon={<Crosshair />}
          onClick={() => (active ? stop() : start(source))}
        />
        <IconButton size="sm" tone="ghost" label={`Remove ${m.name}`} icon={<X />} onClick={() => void send(cmd("Modulation", { type: "RemoveModulator", id: m.id }))} />
      </header>
      <DeviceLayoutView device={pseudo} descriptor={descriptor} sender={sender} />
      {m.kind === "EnvelopeFollower" && <FollowerSource project={project} host={host} modulator={m} kind={m.kind} />}
      {targets.length > 0 && (
        <ul className="eth-mod__targets" aria-label={`${m.name} targets`}>
          {targets.map((t) => (
            <TargetRow key={t.id} project={project} mapping={t.id} />
          ))}
        </ul>
      )}
    </section>
  );
}

function FollowerSource({ project, host, modulator: m }: { project: Project; host: Device; modulator: Modulator; kind: ModulatorKind }) {
  const send = useSend();
  const options = [
    { value: "", label: "Device input" },
    ...sidechainSources(project, { ...host, sidechain: m.sidechain ?? null }).map(({ track, cycle }) => ({
      value: track.id,
      label: cycle ? `${track.name} (would loop)` : track.name,
      disabled: cycle,
    })),
  ];
  return (
    <Select
      size="sm"
      className="eth-mod__source"
      aria-label={`${m.name} follows`}
      title="What the envelope follower listens to"
      options={options}
      value={m.sidechain ?? ""}
      onChange={(v) => void send(cmd("Modulation", { type: "SetSidechain", modulator: m.id, source: v === "" ? null : v }))}
    />
  );
}

function TargetRow({ project, mapping: id }: { project: Project; mapping: string }) {
  const send = useSend();
  const sender = useGestureSender();
  const m = project.mod_mappings[id];
  if (!m) return null;
  const d = project.devices[m.device];
  const label = `${d?.name ?? "Device"} · ${paramName(d, m.param)}`;
  return (
    <li className="eth-mod__target">
      <Knob
        size="sm"
        bipolar
        value={(m.depth + 1) / 2}
        defaultValue={0.5}
        label={label}
        valueText={`${m.depth > 0 ? "+" : ""}${Math.round(m.depth * 100)} %`}
        onChange={(v) => void sender.send(cmd("Modulation", { type: "SetDepth", id, depth: Math.round((v * 2 - 1) * 1000) / 1000 }))}
        onChangeStart={sender.begin}
        onChangeEnd={sender.end}
      />
      <IconButton size="sm" tone="ghost" label={`Remove modulation of ${label}`} icon={<X />} onClick={() => void send(cmd("Modulation", { type: "Unmap", id }))} />
    </li>
  );
}

/** Name of a param from the cached built-in descriptors (mock + engine share the tables). */
function paramName(d: Device | undefined, param: number): string {
  const info = d ? paramInfoOf(d, param) : undefined;
  return info?.name ?? `Param ${param}`;
}

function paramInfoOf(d: Device, param: number) {
  if (d.kind.type !== "Builtin") return undefined;
  return builtinDescriptor(d.kind.device).params.find((p) => p.id === param);
}
