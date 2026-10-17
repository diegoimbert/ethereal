/**
 * The chains of an instrument / audio effect / MIDI effect rack (Ableton-style), in the rack's
 * device card and its window: one collapsible row per chain. A collapsed chain is a line: the
 * chevron and its name, then the track header's controls (mute, solo, volume fader, pan knob),
 * its zones (keys, velocity, chain select) behind a settings button, and delete. Expanded, it
 * shows its devices in the inspector's sectioned chain (`ChainSections`: MIDI effects, the
 * instrument, audio effects; the sections a rack of this type can hold). Drop a device card on
 * a chain's row to move it there. A rack on a chain draws its own chains the same way, one level
 * in (up to `MAX_RACK_DEPTH` levels: the deepest chains offer no racks). Drag a chain's row to
 * reorder it or onto another rack's chains (cmd/ctrl: a copy); its context menu copies, cuts,
 * pastes and duplicates it (`chainClipboard.ts`).
 */
import clsx from "clsx";
import { useCallback, useEffect, useMemo, useRef, useState, type DragEvent } from "react";
import { AppWindow, ChevronRight, Headphones, Plus, SlidersHorizontal, Volume2, VolumeX, X } from "lucide-react";
import type { BuiltinDeviceType, Device, DeviceCategory, DeviceId, RackChain, RackChainId, Zone } from "@/generated";
import { useBuiltinTypes } from "@/features/devices/descriptors";
import { ChainSections } from "@/features/devices/DeviceChain";
import { DeviceLibrary } from "@/features/devices/DeviceLibrary";
import { DeviceWindowButton } from "@/features/devices/window/DeviceWindowButton";
import { setDeviceWindow, useDeviceWindow } from "@/features/devices/window/useDeviceWindow";
import { waitForDevice } from "@/features/devices/userInsert";
import { deviceCategory } from "@/features/devices/chainUtils";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { VolumeFader } from "@/features/arrangement/VolumeFader";
import { deviceSpec, type LibraryDevicePayload } from "@/features/devices/library";
import { recordDeviceUse } from "@/features/devices/libraryStats";
import { DEVICE_DRAG_TYPE } from "@/features/devices/chainUtils";
import { chainSoloCommand } from "@/domain/soloCommands";
import { additiveClick } from "@/domain/soloPlan";
import { useGestureSender, useSend } from "@/features/devices/gesture";
import { firstMatch, shortcutLabel } from "@/features/keymap";
import { useT, type MessageKey } from "@/i18n";
import { IconButton, Knob, NumberField, openContextMenu, Popover } from "@/kit";
import { duration } from "@/theme/tokens";
import { MOTION } from "@/timeline/motion";
import { defaultChainName, isDefaultChainName, useProjectStore } from "@/state";
import { cmd, newId, nextGestureId, useTransport } from "@/transport";
import { fitsChain, MAX_RACK_DEPTH, rackLevel, rackTypeOf } from "./model";
import {
  CHAIN_DRAG_TYPE,
  autoscroll,
  canPlaceChain,
  dropIndex,
  duplicateChain,
  endChainDrag,
  pasteAfter,
  pasteChain,
  placeChain,
  startChainDrag,
  useChainClipboard,
  useChainClipboardKeys,
  useChainDrag,
} from "./chainClipboard";
import "./racks.css";

const byOrder = <T extends { order: string; id: string }>(a: T, b: T) => (a.order < b.order ? -1 : a.order > b.order ? 1 : a.id < b.id ? -1 : 1);

export interface RackPanelProps {
  rack: Device;
}

/** The chain sections a rack type holds (an instrument rack: all three). */
const SECTIONS: Partial<Record<BuiltinDeviceType, ReadonlyArray<DeviceCategory>>> = {
  InstrumentRack: ["NoteEffect", "Instrument", "AudioEffect"],
  AudioEffectRack: ["AudioEffect"],
  MidiEffectRack: ["NoteEffect"],
};

export function RackPanel({ rack }: RackPanelProps) {
  const t = useT();
  const send = useSend();
  const chainsTable = useProjectStore((s) => s.project?.rack_chains);
  const chains = useMemo(
    () =>
      Object.values(chainsTable ?? {})
        .filter((c) => c.rack === rack.id)
        .sort(byOrder),
    [chainsTable, rack.id],
  );
  /** Expanded chains (collapsed by default, a new one too). */
  const [open, setOpen] = useState<ReadonlySet<RackChainId>>(() => new Set());
  const toggle = (id: RackChainId) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const level = useProjectStore((s) => (s.project ? rackLevel(s.project, rack.id) : 1));
  const transport = useTransport();
  useChainClipboardKeys(transport);
  const types = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const over = useChainDrag((s) => (s.over?.rack === rack.id ? s.over.index : null));
  const rackType = rackTypeOf(rack);
  if (!rackType) return null;

  /**
   * A dragged chain over this list (the innermost one: a nested rack's list keeps it): the drop
   * line between rows, or the not-allowed cursor where the chain can't go. `null`: no chain drag.
   */
  const dropAt = (e: DragEvent): { before: RackChainId | null; copy: boolean } | null => {
    const id = useChainDrag.getState().dragging;
    if (!id || !e.dataTransfer.types.includes(CHAIN_DRAG_TYPE)) return null;
    e.stopPropagation();
    const project = useProjectStore.getState().project;
    if (!project || !canPlaceChain(project, id, rack.id, (d) => deviceCategory(d, types, plugins))) {
      e.dataTransfer.dropEffect = "none";
      useChainDrag.getState().setOver(null);
      return null;
    }
    e.preventDefault();
    const copy = e.ctrlKey || e.metaKey;
    e.dataTransfer.dropEffect = copy ? "copy" : "move";
    const rows = Array.from(e.currentTarget.querySelectorAll(":scope > .eth-rack__chain > .eth-rack__row"), (r) => r.getBoundingClientRect());
    const index = dropIndex(rows, e.clientY);
    // In its own rack, right before or after itself is where it already is: no line.
    const same = !copy && (chains[index]?.id === id || chains[index - 1]?.id === id);
    useChainDrag.getState().setOver(same ? null : { rack: rack.id, index });
    return { before: chains[index]?.id ?? null, copy };
  };
  const onListDragOver = (e: DragEvent<HTMLDivElement>) => {
    if (dropAt(e)) autoscroll(e.currentTarget, e.clientY);
  };
  const onListDrop = (e: DragEvent<HTMLDivElement>) => {
    const id = useChainDrag.getState().dragging;
    const at = dropAt(e);
    endChainDrag();
    if (!id || !at) return;
    void placeChain(transport, id, rack.id, at.before, at.copy).then((placed) => placed && useChainClipboard.getState().select(placed));
  };
  const onListDragLeave = (e: DragEvent<HTMLDivElement>) => {
    if (e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget)) return;
    if (useChainDrag.getState().over?.rack === rack.id) useChainDrag.getState().setOver(null);
  };

  // An effect rack's "+ Add chain": a new, empty chain, opened to add its effects.
  const addChain = () => {
    const id = newId();
    setOpen((prev) => new Set(prev).add(id));
    void send(cmd("Rack", { type: "AddChain", id, rack: rack.id, name: null, before: null }));
  };

  return (
    <div
      className={clsx("eth-rack", level > 1 && "eth-rack--nested")}
      data-rack={rack.id}
      data-level={level}
      aria-label={t("racks.chains.label", { name: rack.name })}
    >
      <div
        className="eth-rack__chains"
        role="list"
        aria-label={t("racks.chains.list")}
        onDragOver={onListDragOver}
        onDragLeave={onListDragLeave}
        onDrop={onListDrop}
        // Paste a copied chain at the end (a row's own menu pastes after it).
        onContextMenu={(e) => {
          if ((e.target as HTMLElement).closest(".eth-rack__chains") !== e.currentTarget || (e.target as HTMLElement).closest(".eth-rack__chain")) return;
          openContextMenu(e, [
            {
              label: t("racks.chains.paste"),
              shortcut: shortcutLabel("edit.paste"),
              disabled: !useChainClipboard.getState().copied,
              onSelect: () => void pasteChain(transport, rack.id, null),
            },
          ]);
        }}
      >
        {chains.map((c, i) => (
          <ChainItem
            key={c.id}
            chain={c}
            rackType={rackType}
            nestRacks={level < MAX_RACK_DEPTH}
            expanded={open.has(c.id)}
            onToggle={() => toggle(c.id)}
            dropBefore={over === i}
          />
        ))}
        {over === chains.length && chains.length > 0 && <div className="eth-rack__insert eth-rack__insert--end" aria-hidden data-testid="chain-insert" />}
        {chains.length === 0 && <div className={clsx("eth-rack__empty", over === 0 && "eth-rack__empty--drop")}>{t("racks.chains.empty")}</div>}
        {/* The same dashed card as the sections' "+ Add audio effect". An instrument rack adds an
            instrument (a pick makes a collapsed chain with it); an effect rack an empty chain. */}
        {rackType === "InstrumentRack" ? (
          <AddChainWith rack={rack} rackType={rackType} nestRacks={level < MAX_RACK_DEPTH} />
        ) : (
          <button type="button" className="eth-devices__add-card eth-rack__add-chain" onClick={addChain}>
            <Plus aria-hidden />
            {t("racks.chains.add")}
          </button>
        )}
      </div>
    </div>
  );
}

function ChainItem({
  chain: c,
  rackType,
  nestRacks,
  expanded,
  onToggle,
  dropBefore,
}: {
  chain: RackChain;
  rackType: BuiltinDeviceType;
  nestRacks: boolean;
  expanded: boolean;
  onToggle(): void;
  /** A dragged chain would land right before this one: the drop line above its row. */
  dropBefore: boolean;
}) {
  const t = useT();
  const send = useSend();
  const transport = useTransport();
  const selected = useChainClipboard((s) => s.selected === c.id);
  const cut = useChainClipboard((s) => s.copied?.id === c.id && s.copied.cut);
  const dragging = useChainDrag((s) => s.dragging === c.id);
  /** Where the press that may start a drag began: the row's controls and fields don't drag it. */
  const pressed = useRef<EventTarget | null>(null);
  const sender = useGestureSender();
  const [drop, setDrop] = useState(false);
  const mix = (m: { volume?: number; pan?: number; mute?: boolean; solo?: boolean }) =>
    cmd("Rack", { type: "SetChainMix", id: c.id, volume: m.volume ?? null, pan: m.pan ?? null, mute: m.mute ?? null, solo: m.solo ?? null });
  const soloChain = (additive: boolean) => void send(chainSoloCommand(Object.values(useProjectStore.getState().project?.rack_chains ?? {}), c.id, additive));
  const remove = () => void send(cmd("Rack", { type: "RemoveChain", id: c.id }));
  // Renaming in place (window.prompt does nothing in the desktop app's web view).
  const [renaming, setRenaming] = useState(false);
  const rename = () => setRenaming(true);
  const commitName = (value: string) => {
    setRenaming(false);
    // Empty: back to its default name ("chain N": shown as its devices' names again).
    const project = useProjectStore.getState().project;
    const name = value.trim() || (project ? defaultChainName(project, c) : "");
    if (name && name !== c.name) void send(cmd("Rack", { type: "RenameChain", id: c.id, name }));
  };
  const moveInto = (id: DeviceId, before: DeviceId | null) => void send(cmd("Rack", { type: "MoveDevice", id, chain: c.id, before }));
  const stop = (e: { stopPropagation(): void }) => e.stopPropagation();
  const { mounted, shown } = useFold(expanded);
  // The chain's instrument (an effect rack's chain: its first effect): its window opens from the
  // row; an unnamed chain (its default "chain N") shows its name.
  // An effect rack's chain: its first device stands for it (window button).
  const instrument = useProjectStore((s) => {
    if (!s.project) return null;
    const own = Object.values(s.project.devices)
      .filter((d) => d.chain === c.id)
      .sort(byOrder);
    if (rackType !== "InstrumentRack") return own[0] ?? null;
    return own.find((d) => (d.kind.type === "Builtin" ? INSTRUMENTS.has(d.kind.device.type) : false)) ?? own.find((d) => d.kind.type === "Plugin") ?? null;
  });
  // An effect rack's unnamed chain lists its effects ("Delay, Reverb", ellipsized by its CSS).
  const effectNames = useProjectStore((s) =>
    rackType === "InstrumentRack" || !s.project
      ? ""
      : Object.values(s.project.devices)
          .filter((d) => d.chain === c.id)
          .sort(byOrder)
          .map((d) => d.name)
          .join(", "),
  );
  const fallback = rackType === "InstrumentRack" ? (instrument?.name ?? "") : effectNames;
  const shownName = fallback && isDefaultChainName(c.name) ? fallback : c.name;

  return (
    <div role="listitem" className={clsx("eth-rack__chain", expanded && "eth-rack__chain--open", dragging && "eth-rack__chain--dragging")} aria-label={c.name}>
      {dropBefore && <div className="eth-rack__insert" aria-hidden data-testid="chain-insert" />}
      <div
        className={clsx(
          "eth-rack__row",
          c.mute && "eth-rack__row--muted",
          drop && "eth-rack__row--drop",
          selected && "eth-rack__row--selected",
          cut && "eth-rack__row--cut",
        )}
        role="button"
        tabIndex={0}
        data-chain={c.id}
        aria-expanded={expanded}
        aria-label={t("racks.chain.label", { name: c.name })}
        title={t("racks.chain.drag_hint")}
        onClick={onToggle}
        onPointerDown={(e) => {
          pressed.current = e.target;
          if (e.button === 0 || e.button === 2) useChainClipboard.getState().select(c.id);
        }}
        draggable={!renaming}
        onDragStart={(e) => {
          // A fader, knob, button or the name field under the press: their own drag, not the row's.
          const from = pressed.current instanceof Element ? pressed.current : null;
          if (from?.closest(".eth-rack__controls, .eth-rack__window, input")) {
            e.preventDefault();
            return;
          }
          e.stopPropagation();
          e.dataTransfer.setData(CHAIN_DRAG_TYPE, c.id);
          e.dataTransfer.effectAllowed = "copyMove";
          startChainDrag(c.id);
        }}
        onDragEnd={endChainDrag}
        onKeyDown={(e) => {
          if (e.target !== e.currentTarget) return;
          // ⌘D duplicates (⌘C / ⌘X / ⌘V come as the document's copy / cut / paste events).
          if (firstMatch(["edit.duplicate"], e) === "edit.duplicate") {
            e.preventDefault();
            e.stopPropagation();
            void duplicateChain(transport, c.id);
            return;
          }
          if (e.key !== "Enter" && e.key !== " ") return;
          e.preventDefault();
          onToggle();
        }}
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
          moveInto(id, null);
        }}
        onContextMenu={(e) =>
          openContextMenu(e, [
            { label: t("racks.chain.copy"), shortcut: shortcutLabel("edit.copy"), onSelect: () => useChainClipboard.getState().copy(c.id) },
            { label: t("racks.chain.cut"), shortcut: shortcutLabel("edit.cut"), onSelect: () => useChainClipboard.getState().copy(c.id, true) },
            {
              label: t("racks.chain.paste_after"),
              shortcut: shortcutLabel("edit.paste"),
              disabled: !useChainClipboard.getState().copied,
              onSelect: () => void pasteAfter(transport, c.id),
            },
            { label: t("racks.chain.duplicate"), shortcut: shortcutLabel("edit.duplicate"), onSelect: () => void duplicateChain(transport, c.id) },
            "separator",
            { label: t("racks.chain.rename"), onSelect: rename },
            { label: c.solo ? t("racks.chain.unsolo") : t("racks.chain.solo"), onSelect: () => soloChain(false) },
            "separator",
            { label: t("racks.chain.delete"), danger: true, onSelect: remove },
          ])
        }
      >
        <ChevronRight className="eth-rack__chevron" aria-hidden />
        <span className="eth-rack__window" onClick={stop} onDoubleClick={stop} onKeyDown={stop}>
          {instrument ? (
            <InstrumentWindowButton device={instrument} />
          ) : (
            <IconButton
              size="sm"
              tone="ghost"
              label={rackType === "InstrumentRack" ? t("racks.chain.open_instrument") : t("racks.chain.open_device")}
              title={rackType === "InstrumentRack" ? t("racks.chain.no_instrument") : t("racks.chain.no_device")}
              icon={<AppWindow />}
              disabled
            />
          )}
        </span>
        {renaming ? (
          <input
            className="eth-rack__rename eth-user-text"
            aria-label={t("racks.chain.name")}
            defaultValue={shownName}
            autoFocus
            onFocus={(e) => e.currentTarget.select()}
            onClick={stop}
            onDoubleClick={stop}
            onBlur={(e) => commitName(e.currentTarget.value)}
            onKeyDown={(e) => {
              e.stopPropagation();
              if (e.key === "Enter") commitName(e.currentTarget.value);
              else if (e.key === "Escape") setRenaming(false);
            }}
          />
        ) : (
          <span className="eth-rack__name eth-user-text" onDoubleClick={(e) => (stop(e), rename())} title={t("racks.chain.rename_hint")}>
            {shownName}
          </span>
        )}
        {/* The track header's controls (same buttons, fader and knob). */}
        <span className="eth-rack__controls" onClick={stop} onKeyDown={stop} onDoubleClick={stop}>
          <button
            type="button"
            className="eth-arr-header__toggle eth-arr-header__mute eth-rack__toggle"
            aria-pressed={c.mute}
            aria-label={t("racks.chain.mute_label", { name: c.name })}
            title={c.mute ? t("racks.chain.unmute") : t("racks.chain.mute")}
            onClick={() => void send(mix({ mute: !c.mute }))}
          >
            {c.mute ? <VolumeX /> : <Volume2 />}
          </button>
          <button
            type="button"
            className="eth-arr-header__toggle eth-arr-header__solo eth-rack__toggle"
            aria-pressed={c.solo}
            aria-label={t("racks.chain.solo_label", { name: c.name })}
            title={t("racks.chain.solo_title")}
            onClick={(e) => soloChain(additiveClick(e))}
          >
            <Headphones />
          </button>
          <VolumeFader
            db={c.volume}
            label={t("racks.chain.volume", { name: c.name })}
            onBegin={sender.begin}
            onEnd={sender.end}
            onChange={(db) => {
              if (db !== c.volume) void sender.send(mix({ volume: db }));
            }}
          />
          <span className="eth-arr-pan">
            <Knob
              size="sm"
              bipolar
              hideLabel
              label={t("racks.chain.pan", { name: c.name })}
              value={(c.pan + 1) / 2}
              valueText={formatPan(c.pan)}
              onChange={(v) => void sender.send(mix({ pan: Math.round((v * 2 - 1) * 100) / 100 }))}
              onChangeStart={sender.begin}
              onChangeEnd={sender.end}
            />
          </span>
          <Popover
            aria-label={t("racks.chain.zones", { name: c.name })}
            placement="bottom-end"
            trigger={(p) => (
              <IconButton
                size="sm"
                tone="ghost"
                label={t("racks.chain.zones", { name: c.name })}
                title={t("racks.chain.zones_title")}
                icon={<SlidersHorizontal />}
                {...p}
              />
            )}
          >
            {() => <ZoneEditor chain={c} />}
          </Popover>
          <IconButton
            size="sm"
            tone="ghost"
            label={t("racks.chain.delete_label", { name: c.name })}
            title={t("racks.chain.delete_title")}
            icon={<X />}
            onClick={remove}
          />
        </span>
      </div>
      {/* Opens and closes animated (its height eases; the devices stay mounted while it closes). */}
      {mounted && (
        <div className={clsx("eth-rack__fold", shown && "eth-rack__fold--open")}>
          <div className="eth-rack__fold-inner">
            <ChainDevices chain={c} rackType={rackType} nestRacks={nestRacks} />
          </div>
        </div>
      )}
    </div>
  );
}

/** What a rack's add card adds (a chain's first device): an instrument rack's instrument. */
const FIRST_DEVICE: Partial<Record<BuiltinDeviceType, { category: DeviceCategory; label: MessageKey }>> = {
  InstrumentRack: { category: "Instrument", label: "racks.chains.add_instrument" },
};

/**
 * An instrument rack's "+ Add instrument": the instrument library under the card; a pick adds a
 * chain with that instrument (collapsed), one undo step, and opens the instrument's window.
 */
function AddChainWith({ rack, rackType, nestRacks }: { rack: Device; rackType: BuiltinDeviceType; nestRacks: boolean }) {
  const t = useT();
  const transport = useTransport();
  const accept = useChainAccept(rackType, nestRacks);
  const first = FIRST_DEVICE[rackType];
  if (!first) return null;
  const add = async (p: LibraryDevicePayload) => {
    recordDeviceUse(p);
    const gesture = nextGestureId();
    const chain = newId();
    const device = newId();
    try {
      await transport.send(cmd("Rack", { type: "AddChain", id: chain, rack: rack.id, name: null, before: null }), { gesture });
      await transport.send(cmd("Rack", { type: "InsertDevice", id: device, chain, device: deviceSpec(p), before: null }), { gesture });
    } finally {
      transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
    }
    // Like any device added from the library, the new device's window opens.
    const added = await waitForDevice(device);
    if (added) setDeviceWindow(transport, added, true);
  };
  return (
    <Popover
      aria-label={t(first.label)}
      placement="bottom-start"
      trigger={(p) => (
        <button type="button" className="eth-devices__add-card eth-rack__add-chain" {...p}>
          <Plus aria-hidden />
          {t(first.label)}
        </button>
      )}
    >
      {(close) => (
        <DeviceLibrary
          category={first.category}
          accept={accept}
          onClose={close}
          onPick={(payload) => {
            close();
            void add(payload).catch((e: unknown) => console.warn("add chain failed", e));
          }}
        />
      )}
    </Popover>
  );
}

/** What fits a chain of a `rackType` rack: plugins by category, built-ins by the rack's rule. */
function useChainAccept(rackType: BuiltinDeviceType, nestRacks: boolean) {
  const types = useBuiltinTypes();
  return useCallback(
    (p: LibraryDevicePayload) => {
      if (p.device.type === "Plugin")
        return rackType === "AudioEffectRack" ? p.category === "AudioEffect" : rackType === "MidiEffectRack" ? p.category === "NoteEffect" : true;
      const b = p.device.builtin;
      const d = types.find((t) => t.device_type.type === "Builtin" && t.device_type.device === b);
      return !!d && fitsChain(rackType, d, nestRacks);
    },
    [types, rackType, nestRacks],
  );
}

/** The chain instrument's window toggle (the device card's own button). */
function InstrumentWindowButton({ device }: { device: Device }) {
  return <DeviceWindowButton device={device} control={useDeviceWindow(device)} />;
}

/** Built-in instruments (a chain's name falls back to its instrument's). */
const INSTRUMENTS = new Set<BuiltinDeviceType>([
  "Synth",
  "PolySynth",
  "ExternalInstrument",
  "Sampler",
  "MultiSampler",
  "FmSynth",
  "DrumRack",
  "InstrumentRack",
]);

/** How long a fold animates (`--eth-duration-normal`, scaled by the animation setting). */
const FOLD_MS = parseFloat(duration.normal);

/**
 * An animated fold: `mounted` while open or closing, `shown` (the open class) a frame after it
 * mounts (so the opening transitions) and off at once when it closes (then unmounted once the
 * transition is over; at once with animations off).
 */
function useFold(open: boolean): { mounted: boolean; shown: boolean } {
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(open);
  // Opening mounts the content (closed); closing drops the open class at once (adjusted while
  // rendering) and unmounts once the transition is over.
  const [prevOpen, setPrevOpen] = useState(open);
  if (open !== prevOpen) {
    setPrevOpen(open);
    if (open) {
      setMounted(true);
    } else {
      setShown(false);
      if (!MOTION.enabled) setMounted(false);
    }
  }
  // With animations off, open content is shown at once.
  if (open && mounted && !shown && !MOTION.enabled) setShown(true);
  useEffect(() => {
    if (open || !MOTION.enabled) return;
    const t = setTimeout(() => setMounted(false), FOLD_MS * MOTION.speed + 50);
    return () => clearTimeout(t);
  }, [open]);
  // Once the closed content is on screen (mounted and laid out: two frames), open it, so the
  // height always transitions (one frame could land before the closed state was painted).
  useEffect(() => {
    if (!open || !mounted || !MOTION.enabled) return;
    let second = 0;
    const first = requestAnimationFrame(() => {
      second = requestAnimationFrame(() => setShown(true));
    });
    return () => {
      cancelAnimationFrame(first);
      cancelAnimationFrame(second);
    };
  }, [open, mounted]);
  return { mounted, shown };
}

function formatPan(pan: number): string {
  return Math.abs(pan) < 0.005 ? "C" : `${Math.round(Math.abs(pan) * 100)}${pan < 0 ? "L" : "R"}`;
}

/** An expanded chain: its devices in the inspector's sectioned chain. */
function ChainDevices({ chain, rackType, nestRacks }: { chain: RackChain; rackType: BuiltinDeviceType; nestRacks: boolean }) {
  const t = useT();
  const send = useSend();
  const devicesTable = useProjectStore((s) => s.project?.devices);
  const devices = useMemo(
    () =>
      Object.values(devicesTable ?? {})
        .filter((d) => d.chain === chain.id)
        .sort(byOrder),
    [devicesTable, chain.id],
  );
  const types = useBuiltinTypes();
  const plugins = usePluginStore((s) => s.plugins);
  const categoryOf = (d: Device) => deviceCategory(d, types, plugins);
  // Plugins fit by category; built-ins by the rack's rule (no drum racks, racks below the depth limit).
  const accept = useChainAccept(rackType, nestRacks);
  const emptyInstrumentChain = rackType === "InstrumentRack" && devices.length === 0;
  /** MIDI effects before the instrument, the instrument before the audio effects, audio effects last. */
  const insertBefore = (category: DeviceCategory): DeviceId | null => {
    if (category === "AudioEffect") return null;
    const after: DeviceCategory[] = category === "NoteEffect" ? ["Instrument", "AudioEffect"] : ["AudioEffect"];
    return devices.find((d) => after.includes(categoryOf(d)))?.id ?? null;
  };
  return (
    <div className="eth-devices eth-devices--stack eth-rack__body">
      <ChainSections
        devices={devices}
        // An empty instrument rack chain starts with its instrument: only that section, its
        // library open.
        categories={emptyInstrumentChain ? ["Instrument"] : (SECTIONS[rackType] ?? [])}
        autoOpenAdd={emptyInstrumentChain ? "Instrument" : null}
        // An effect rack's chain holds one kind of device: no section header.
        hideHeads={rackType !== "InstrumentRack"}
        layout="stack"
        label={t("racks.chain.devices", { name: chain.name })}
        accept={accept}
        onAdd={(p, category) => {
          recordDeviceUse(p);
          void send(cmd("Rack", { type: "InsertDevice", id: newId(), chain: chain.id, device: deviceSpec(p), before: insertBefore(category) }));
        }}
        onMove={(id, before) => void send(cmd("Rack", { type: "MoveDevice", id, chain: chain.id, before }))}
        pasteChain={{ type: "Chain", chain: chain.id }}
      />
    </div>
  );
}

/** Key / velocity / chain-selector zones of a chain. */
function ZoneEditor({ chain: c }: { chain: RackChain }) {
  const t = useT();
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
          aria-label={t("racks.zones.low", { name: c.name, zone: label })}
          value={z.lo}
          min={0}
          max={z.hi}
          step={1}
          onChange={(v) => set(field, { lo: Math.round(v), hi: z.hi })}
        />
        <NumberField
          size="sm"
          aria-label={t("racks.zones.high", { name: c.name, zone: label })}
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
    <div className="eth-rack__zones" aria-label={t("racks.chain.zones", { name: c.name })}>
      {row("keys", t("racks.zones.keys"))}
      {row("velocities", t("racks.zones.velocity"))}
      {row("select", t("racks.zones.chain_select"))}
    </div>
  );
}
