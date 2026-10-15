import { useEffect, useState, type DragEvent } from "react";
import { duration } from "@/theme/tokens";
import { MOTION } from "@/timeline/motion";
import { IconButton, Popover, Select } from "@/kit";
import { Plus } from "lucide-react";
import type { Device, DeviceCategory, DeviceDescriptor, DeviceId, Track, TrackId } from "@/generated";
import clsx from "clsx";
import { useDevicesOfTrack, useProjectStore, useReadOnly, useSelectionStore, useTracksOrdered } from "@/state";
import { cmd, useTransport } from "@/transport";
import { useT, type MessageKey } from "@/i18n";
import { useBuiltinTypes } from "./descriptors";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { CHAIN_SECTIONS, DEVICE_DRAG_TYPE, deviceCategory, draggingDevice } from "./chainUtils";
import { canHost, type LibraryDevicePayload } from "./library";
import { insertFromLibrary } from "./userInsert";
import { DevicePicker } from "./DevicePicker";
import { DeviceLibrary } from "./DeviceLibrary";
import { DeviceView } from "./DeviceView";
import { useSend } from "./gesture";
import { useSelectedTrack } from "./selectedTrack";
import "./devices.css";

function TrackPicker({ track }: { track: Track }) {
  const tracks = useTracksOrdered();
  const selectTrack = useSelectionStore((s) => s.selectTrack);
  const t = useT();
  return (
    <Select
      size="sm"
      className="eth-devices__track"
      aria-label={t("devices.chain.track")}
      value={track.id}
      onChange={(v) => selectTrack(v)}
      options={tracks.map((t) => ({ value: t.id, label: t.name, userText: true }))}
    />
  );
}

/** The row layout's "+ Add device…": the device library of every category the track can host. */
function AddDevice({ track }: { track: Track }) {
  const transport = useTransport();
  const t = useT();
  const categories = CHAIN_SECTIONS.map((c) => c.id).filter((c) => canHost(c, track));
  return (
    <DevicePicker
      className="eth-devices__add"
      aria-label={t("devices.chain.add_device")}
      placeholder
      categories={categories}
      onPick={(payload) => void insertFromLibrary(transport, track, payload)}
    >
      {t("devices.chain.add_device_button")}
    </DevicePicker>
  );
}

/** What each section's add menu calls its devices. */
const ADD_LABEL: Record<DeviceDescriptor["category"], MessageKey> = {
  NoteEffect: "devices.chain.add_midi_effect",
  Instrument: "devices.chain.add_instrument",
  AudioEffect: "devices.chain.add_audio_effect",
};

/**
 * A section's add button (stacked layout, right of its label): opens the device library of
 * its category in a popover (search, All / Factory / Plugins, Rescan); picking a device adds
 * it. An instrument goes in the empty slot; effects go at their section's place (MIDI effects
 * before the instrument, audio effects last).
 */
function AddSectionDevice({
  category,
  card = false,
  accept,
  onAdd,
  autoOpen = false,
}: {
  category: DeviceDescriptor["category"];
  card?: boolean;
  accept?(payload: LibraryDevicePayload): boolean;
  onAdd(payload: LibraryDevicePayload): void;
  /** Open the library on mount (after the surrounding fold's opening animation). */
  autoOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const t = useT();
  // A shared project's viewer adds nothing.
  const readOnly = useReadOnly();
  useEffect(() => {
    if (!autoOpen) return;
    const t = setTimeout(() => setOpen(true), MOTION.enabled ? parseFloat(duration.normal) * MOTION.speed : 0);
    return () => clearTimeout(t);
  }, [autoOpen]);
  if (readOnly) return null;
  const addLabel = t(ADD_LABEL[category]);
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      aria-label={addLabel}
      placement="bottom-end"
      trigger={(p) =>
        card ? (
          // The section's last "device": a placeholder card that adds one.
          <button type="button" className="eth-devices__add-card" aria-label={addLabel} {...p}>
            <Plus aria-hidden />
            {addLabel}
          </button>
        ) : (
          <IconButton size="sm" tone="ghost" className="eth-devices__section-add" label={addLabel} icon={<Plus />} {...p} />
        )
      }
    >
      {(close) => (
        <DeviceLibrary
          category={category}
          accept={accept}
          onClose={close}
          onPick={(payload) => {
            close();
            onAdd(payload);
          }}
        />
      )}
    </Popover>
  );
}

export type ChainLayout = "row" | "stack";

export interface ChainSectionsProps {
  /** The chain, in order. */
  devices: ReadonlyArray<Device>;
  /** The sections shown (MIDI Effects / Instrument / Audio Effects), in that order. */
  categories: ReadonlyArray<DeviceCategory>;
  layout: ChainLayout;
  label: string;
  /** Add a device picked in a section's library. */
  onAdd(payload: LibraryDevicePayload, category: DeviceCategory): void;
  /** Move `id` before `before` (null: the end); only called for moves inside a section. */
  onMove(id: DeviceId, before: DeviceId | null): void;
  /** Narrows the sections' libraries (e.g. no drum rack inside a drum pad). */
  accept?(payload: LibraryDevicePayload): boolean;
  /** Remove a device (default `Device::Remove`). */
  onRemove?(device: Device): void;
  /** Open this section's add library on mount (e.g. an empty rack chain's instrument). */
  autoOpenAdd?: DeviceCategory | null;
  /** No section headers (label and "+"): a chain of one kind of device (an effect rack's). */
  hideHeads?: boolean;
}

/**
 * A device chain in sections (MIDI effects, the instrument, audio effects): each with its
 * label, its devices (cards) and its add button / card. Shared by a track's chain (the
 * inspector) and a drum pad's chain.
 */
export function ChainSections({
  devices,
  categories,
  layout,
  label,
  onAdd,
  onMove,
  accept,
  onRemove,
  autoOpenAdd = null,
  hideHeads = false,
}: ChainSectionsProps) {
  const stack = layout === "stack";
  const t = useT();
  const builtins = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const [endDrop, setEndDrop] = useState(false);
  const categoryOf = (d: Device) => deviceCategory(d, builtins, plugins);
  // Moves stay inside the device's section (the engine's chain rule): before a device of the
  // same section, or right after the section's last one. A device of another chain goes anywhere.
  const canMove = (id: DeviceId | null, before: DeviceId | null) => {
    const dragged = id === null ? undefined : devices.find((d) => d.id === id);
    if (!dragged) return true;
    const category = categoryOf(dragged);
    const section = devices.filter((d) => categoryOf(d) === category);
    const afterSection = devices[devices.indexOf(section[section.length - 1]!) + 1]?.id ?? null;
    const target = before === null ? null : devices.find((d) => d.id === before);
    // Before itself or right after itself: no move.
    if (before === id || (before !== null && devices[devices.indexOf(dragged) + 1]?.id === before)) return false;
    if (before === null && devices.at(-1)?.id === id) return false;
    return before === afterSection || (!!target && categoryOf(target) === category);
  };
  const moveTo = (id: DeviceId, before: DeviceId | null) => {
    if (canMove(id, before)) onMove(id, before);
  };
  const sections = CHAIN_SECTIONS.filter((s) => categories.includes(s.id)).map((s) => ({
    ...s,
    label: t(s.label),
    devices: devices.filter((d) => categoryOf(d) === s.id),
  }));

  const onEndDragOver = (e: DragEvent) => {
    if (!e.dataTransfer.types.includes(DEVICE_DRAG_TYPE) || !canMove(draggingDevice(), null)) return;
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
    <div className={clsx("eth-devices__chain", stack && "eth-devices__chain--stack")} role="list" aria-label={label}>
      {sections.map((section) => (
        <div key={section.id} role="group" aria-label={section.label} className="eth-devices__section">
          {!hideHeads && (
            <div className="eth-devices__section-head">
              <span className="eth-devices__section-label" aria-hidden>
                {section.label}
              </span>
              {/* One instrument per chain: its menu only while the slot is empty. */}
              {stack && (section.id !== "Instrument" || section.devices.length === 0) && (
                <AddSectionDevice category={section.id} accept={accept} onAdd={(p) => onAdd(p, section.id)} />
              )}
            </div>
          )}
          {section.devices.map((d, i) => {
            const at = devices.indexOf(d);
            return (
              <div role="listitem" key={d.id} className="eth-devices__slot">
                <DeviceView
                  device={d}
                  prev={section.devices[i - 1]?.id ?? null}
                  moveRightBefore={i === section.devices.length - 1 ? undefined : (devices[at + 2]?.id ?? null)}
                  onDropBefore={(dragged) => moveTo(dragged, d.id)}
                  acceptsBefore={(dragged) => canMove(dragged, d.id)}
                  onMove={(before) => moveTo(d.id, before)}
                  onRemove={onRemove && (() => onRemove(d))}
                  layout={layout}
                />
              </div>
            );
          })}
          {stack && (section.id !== "Instrument" || section.devices.length === 0) && (
            <AddSectionDevice category={section.id} accept={accept} onAdd={(p) => onAdd(p, section.id)} card autoOpen={autoOpenAdd === section.id} />
          )}
        </div>
      ))}
      <div
        className={endDrop ? "eth-devices__end eth-devices__end--drop" : "eth-devices__end"}
        onDragOver={onEndDragOver}
        onDragLeave={() => setEndDrop(false)}
        onDrop={onEndDrop}
      />
    </div>
  );
}

function Chain({ track, layout, picker }: { track: Track; layout: ChainLayout; picker: boolean }) {
  const stack = layout === "stack";
  const t = useT();
  const send = useSend();
  const transport = useTransport();
  const devices = useDevicesOfTrack(track.id);
  // Every section the track can hold is shown, empty or not (MIDI effects and the instrument
  // on MIDI tracks only).
  const categories = CHAIN_SECTIONS.map((s) => s.id).filter((c) => canHost(c, track));
  return (
    <div className={clsx("eth-devices", stack && "eth-devices--stack")} data-feature="devices">
      {!stack && (
        <div className="eth-devices__toolbar">
          {picker && <TrackPicker track={track} />}
          <AddDevice track={track} />
        </div>
      )}
      <ChainSections
        devices={devices}
        categories={categories}
        layout={layout}
        label={t("devices.chain.label", { name: track.name })}
        onAdd={(payload) => void insertFromLibrary(transport, track, payload)}
        onMove={(id, before) => void send(cmd("Device", { type: "Move", id, track: track.id, before }))}
      />
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
  const t = useT();
  if (!track)
    return (
      <div className="eth-devices eth-devices--empty" data-feature="devices">
        {t("devices.chain.no_project")}
      </div>
    );
  return <Chain key={track.id} track={track} layout={layout} picker={!trackId} />;
}
