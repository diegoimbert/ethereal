import { useEffect, useState, type DragEvent, type PointerEvent } from "react";
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
import { CHAIN_SECTIONS, currentChainDrag, deviceCategory, droppedChainDrag, isChainDrag, type ChainDrag } from "./chainUtils";
import { canHost, type LibraryDevicePayload } from "./library";
import { insertFromLibrary } from "./userInsert";
import { DevicePicker } from "./DevicePicker";
import { DeviceLibrary } from "./DeviceLibrary";
import { DeviceView } from "./DeviceView";
import { useSend } from "./gesture";
import { useSelectedTrack } from "./selectedTrack";
import { pasteChainAttr, useDeviceClipboard, useDeviceClipboardKeys, type PasteChain } from "./deviceClipboard";
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
 * before the instrument, audio effects last). A row can also be dragged onto the chain, to a
 * place in its section (before a device, or at the section's end).
 */
function AddSectionDevice({
  category,
  card = false,
  accept,
  onAdd,
  autoOpen = false,
  drop,
}: {
  category: DeviceDescriptor["category"];
  card?: boolean;
  /**
   * The card as a drop target for a dragged device: it lands at the end of the section (right
   * after its last device), and the insert line shows in the gap above the card, between the
   * last device and the card, like the line before any other card.
   */
  drop?: { active: boolean; onDragOver(e: DragEvent): void; onDragLeave(): void; onDrop(e: DragEvent): void };
  accept?(payload: LibraryDevicePayload): boolean;
  onAdd(payload: LibraryDevicePayload): void;
  /** Open the library on mount (after the surrounding fold's opening animation). */
  autoOpen?: boolean;
}) {
  const [open, setOpen] = useState(false);
  /** A row is dragged out onto the chain: the popover steps aside (still open) until it ends. */
  const [dragging, setDragging] = useState(false);
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
      className={clsx(dragging && "eth-devlib-popover--dragging")}
      trigger={(p) =>
        card ? (
          // The section's last "device": a placeholder card that adds one.
          <button
            type="button"
            className="eth-devices__add-card"
            aria-label={addLabel}
            onDragOver={drop?.onDragOver}
            onDragLeave={drop?.onDragLeave}
            onDrop={drop?.onDrop}
            {...p}
          >
            {drop?.active && <span className="eth-device__insert" aria-hidden data-testid="device-insert" />}
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
          // Dragged onto the chain: dropped in place (the chain's drop targets), or back to the
          // list when the drag is cancelled.
          drag={{
            // After the drag has started (hiding its source at once cancels it in WebKit).
            onStart: () => void setTimeout(() => setDragging(true)),
            onEnd: (dropped) => {
              setDragging(false);
              if (dropped) close();
            },
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
  /**
   * Add a device picked in a section's library: at `before` when given (dropped from the
   * library at a place in its section; `null`: the end), else at its section's usual place.
   */
  onAdd(payload: LibraryDevicePayload, category: DeviceCategory, before?: DeviceId | null): void;
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
  /**
   * The chain, for ⌘V with no device selected (deviceClipboard.ts): a click on its empty space
   * clears the selection and focuses it, and a paste then appends at its end.
   */
  pasteChain?: PasteChain;
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
  pasteChain,
}: ChainSectionsProps) {
  const stack = layout === "stack";
  const t = useT();
  const builtins = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const [endDrop, setEndDrop] = useState(false);
  /** The section whose add card a dragged device is over (it lands at the section's end). */
  const [cardDrop, setCardDrop] = useState<DeviceCategory | null>(null);
  const readOnly = useReadOnly();
  const categoryOf = (d: Device) => deviceCategory(d, builtins, plugins);
  const transport = useTransport();
  // Bound here too, so an empty chain takes a paste.
  useDeviceClipboardKeys(transport, categoryOf);
  // A click on the chain's own space (not a card, not a control): no device selected and the
  // chain focused, so ⌘V appends at its end.
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    const el = e.target as HTMLElement;
    if (!pasteChain || e.button !== 0 || el.closest("[data-paste-chain]") !== e.currentTarget) return;
    if (el.closest(".eth-device, button, input, select, textarea, a, [role='button'], [contenteditable='true']")) return;
    useDeviceClipboard.getState().select(null);
    e.currentTarget.focus({ preventScroll: true });
  };
  /** Whether `category` may land at `before`: before a device of its section, or at its end. */
  const inSection = (category: DeviceCategory, before: DeviceId | null) => {
    const target = before === null ? null : devices.find((d) => d.id === before);
    return before === sectionEnd(category) || (!!target && categoryOf(target) === category);
  };
  // Moves stay inside the device's section (the engine's chain rule): before a device of the
  // same section, or right after the section's last one. A device of another chain goes
  // anywhere. A library device goes in its own section, if the chain shows it (and takes it);
  // an instrument only in an empty slot.
  const canDrop = (drag: ChainDrag | null, before: DeviceId | null) => {
    if (readOnly) return false;
    if (drag?.type === "library") {
      const { category } = drag.payload;
      if (!categories.includes(category) || (accept && !accept(drag.payload))) return false;
      if (category === "Instrument" && devices.some((d) => categoryOf(d) === "Instrument")) return false;
      return inSection(category, before);
    }
    const id = drag?.id ?? null;
    const dragged = id === null ? undefined : devices.find((d) => d.id === id);
    if (!dragged) return true;
    // Before itself or right after itself: no move.
    if (before === id || (before !== null && devices[devices.indexOf(dragged) + 1]?.id === before)) return false;
    if (before === null && devices.at(-1)?.id === id) return false;
    return inSection(categoryOf(dragged), before);
  };
  const dropAt = (drag: ChainDrag, before: DeviceId | null) => {
    if (!canDrop(drag, before)) return;
    if (drag.type === "library") onAdd(drag.payload, drag.payload.category, before);
    else onMove(drag.id, before);
  };
  const sections = CHAIN_SECTIONS.filter((s) => categories.includes(s.id)).map((s) => ({
    ...s,
    label: t(s.label),
    devices: devices.filter((d) => categoryOf(d) === s.id),
  }));

  // Stacked, a section ends with its add card (none for the instrument while there is one, none
  // for a viewer): the end of the section is between its last device and that card.
  const hasCard = (s: (typeof sections)[number]) => stack && !readOnly && (s.id !== "Instrument" || s.devices.length === 0);
  /** Where a device dropped at a section's end lands: before the next section's first device. */
  const sectionEnd = (category: DeviceCategory): DeviceId | null => {
    const later = CHAIN_SECTIONS.slice(CHAIN_SECTIONS.findIndex((c) => c.id === category) + 1).map((c) => c.id);
    return devices.find((d) => later.includes(categoryOf(d)))?.id ?? null;
  };
  // The chain's end zone (below everything) is the last section's end: with an add card there,
  // its line shows above that card, not below it.
  const last = sections.at(-1);
  const lastCard = last && hasCard(last) ? last.id : null;
  const cardDropProps = (category: DeviceCategory) => ({
    active: cardDrop === category || (endDrop && category === lastCard),
    onDragOver: (e: DragEvent) => {
      if (!isChainDrag(e.dataTransfer.types) || !canDrop(currentChainDrag(), sectionEnd(category))) return;
      e.preventDefault();
      e.stopPropagation();
      setCardDrop(category);
    },
    onDragLeave: () => setCardDrop(null),
    onDrop: (e: DragEvent) => {
      setCardDrop(null);
      const drag = droppedChainDrag(e.dataTransfer);
      if (!drag) return;
      e.preventDefault();
      e.stopPropagation();
      dropAt(drag, sectionEnd(category));
    },
  });

  const onEndDragOver = (e: DragEvent) => {
    if (!isChainDrag(e.dataTransfer.types) || !canDrop(currentChainDrag(), null)) return;
    e.preventDefault();
    e.stopPropagation();
    setEndDrop(true);
  };
  const onEndDrop = (e: DragEvent) => {
    setEndDrop(false);
    const drag = droppedChainDrag(e.dataTransfer);
    if (!drag) return;
    e.preventDefault();
    e.stopPropagation();
    dropAt(drag, null);
  };

  return (
    <div
      className={clsx("eth-devices__chain", stack && "eth-devices__chain--stack")}
      role="list"
      aria-label={label}
      data-paste-chain={pasteChain && pasteChainAttr(pasteChain)}
      tabIndex={pasteChain ? -1 : undefined}
      onPointerDown={onPointerDown}
    >
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
                  onDropBefore={(dragged) => dropAt(dragged, d.id)}
                  acceptsBefore={(dragged) => canDrop(dragged, d.id)}
                  onMove={(before) => dropAt({ type: "device", id: d.id }, before)}
                  onRemove={onRemove && (() => onRemove(d))}
                  layout={layout}
                />
              </div>
            );
          })}
          {hasCard(section) && (
            <AddSectionDevice
              category={section.id}
              accept={accept}
              onAdd={(p) => onAdd(p, section.id)}
              card
              autoOpen={autoOpenAdd === section.id}
              drop={cardDropProps(section.id)}
            />
          )}
        </div>
      ))}
      <div
        className={endDrop && !lastCard ? "eth-devices__end eth-devices__end--drop" : "eth-devices__end"}
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
        onAdd={(payload, _category, before) => void insertFromLibrary(transport, track, payload, before)}
        onMove={(id, before) => void send(cmd("Device", { type: "Move", id, track: track.id, before }))}
        pasteChain={{ type: "Track", track: track.id }}
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
