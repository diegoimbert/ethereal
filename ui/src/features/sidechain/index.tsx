// `SidechainSelector` is mounted in every device header by `features/devices/DeviceView`
// (next to `PluginDeviceControls`): keep its name and props.
import clsx from "clsx";
import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, ChevronRight } from "lucide-react";
import type { Device, MidiSidechain, MidiTap, Project, SidechainTap, TrackId } from "@/generated";
import { useDescriptor } from "@/features/devices/descriptors";
import { useSend } from "@/features/devices/gesture";
import { foldForSearch, useT } from "@/i18n";
import { disabledProps, menuItems, menuKeyDown, Popover, SearchField } from "@/kit";
import { clampToViewport } from "@/kit/placement";
import { useProject } from "@/state";
import { cmd } from "@/transport";
import {
  defaultTap,
  hasNoteInput,
  MIDI_TAPS,
  midiSidechainSources,
  POINT_LABEL,
  sidechainSources,
  TAP_ERROR_TEXT,
  tapError,
  tapKey,
  tapOf,
  tapPoints,
  type TapOption,
} from "./routing";

export interface SidechainSelectorProps {
  device: Device;
}

/** A source track as the picker lists it. */
interface SourceRow {
  track: TrackId;
  /** "Drums", "Keys (this track)", "Bass (would loop)". */
  label: string;
  userText: boolean;
  /** It can be picked (some tap point of it keys the device without a cycle). */
  usable: boolean;
  /** Its tap points (those matching the search when the track's name doesn't). */
  taps: Array<TapOption & { refused: string | null }>;
}

/**
 * Sidechain source picker (`Device::SetSidechain`, one undo step per change), for devices at
 * any depth (track chain, drum pads, rack chains). Renders nothing for devices without a
 * sidechain input (`DeviceDescriptor.sidechain_inputs == 0`). One dropdown: a search field,
 * then the source tracks (every track but master; its own track for its pre FX signal and
 * earlier racks' chains), each with its tap points in a submenu (pre FX / post FX / post
 * mixer of the track or of any of its rack chains and drum pads). A click on a track takes its
 * default point. Choices the engine would reject (routing cycles, points produced after the
 * device) are disabled, with the reason.
 */
export function SidechainSelector({ device }: SidechainSelectorProps) {
  const t = useT();
  const { descriptor } = useDescriptor(device);
  const project = useProject();
  const send = useSend();
  const [open, setOpen] = useState(false);
  // Each opening starts afresh (no search left from the last one, even while it fades out).
  const [opening, setOpening] = useState(0);
  if (!descriptor || descriptor.sidechain_inputs === 0 || !project) return null;

  const set = (source: TrackId | null, tap: SidechainTap | null) =>
    void send(
      cmd("Device", {
        type: "SetSidechain",
        device: device.id,
        source,
        tap: tap ?? undefined,
      }),
    );

  const source = device.sidechain;
  const sourceTrack = source === null ? null : project.tracks[source];
  const tapLabel = source === null ? null : tapPoints(project, source).find((o) => tapKey(o.tap) === tapKey(tapOf(device)))?.label;
  const label =
    source === null
      ? t("sidechain.none")
      : `${source === device.track ? t("sidechain.source_own", { name: sourceTrack?.name ?? "" }) : (sourceTrack?.name ?? "")}${tapLabel ? ` · ${tapLabel}` : ""}`;
  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        if (o) setOpening((n) => n + 1);
        setOpen(o);
      }}
      placement="bottom-start"
      className="eth-popover--menu eth-sidechain-picker"
      aria-label={t("sidechain.source_label", { name: device.name })}
      trigger={(p) => (
        <button
          type="button"
          className={clsx("eth-select", "eth-select--sm", "eth-sidechain", open && "eth-select--open")}
          aria-label={t("sidechain.source_label", { name: device.name })}
          title={t("sidechain.source_title")}
          {...p}
        >
          <span className={clsx("eth-select__value", source !== null && source !== device.track && "eth-user-text")}>{label}</span>
          <ChevronDown className="eth-select__chevron" aria-hidden />
        </button>
      )}
    >
      {(close) => (
        <SidechainMenu
          key={opening}
          project={project}
          device={device}
          onPick={(src, tap) => {
            close();
            set(src, tap);
          }}
        />
      )}
    </Popover>
  );
}

function SidechainMenu({ project, device, onPick }: { project: Project; device: Device; onPick(source: TrackId | null, tap: SidechainTap | null): void }) {
  const t = useT();
  const [query, setQuery] = useState("");
  // The track whose tap points show, with its item (the submenu's anchor).
  const [sub, setSub] = useState<{ track: TrackId; item: HTMLElement } | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const current = tapOf(device);
  const q = foldForSearch(query.trim());

  const rows: SourceRow[] = [];
  for (const { track, cycle } of sidechainSources(project, device)) {
    const own = track.id === device.track;
    const isSource = track.id === device.sidechain;
    const taps = tapPoints(project, track.id).map((o) => {
      const why = isSource && tapKey(o.tap) === tapKey(current) ? null : tapError(project, device, track.id, o.tap);
      return { ...o, refused: why ? t(TAP_ERROR_TEXT[why]) : null };
    });
    const nameHit = q === "" || foldForSearch(track.name).includes(q);
    const shown = nameHit ? taps : taps.filter((o) => foldForSearch(o.label).includes(q));
    if (shown.length === 0) continue;
    rows.push({
      track: track.id,
      label: cycle ? t("sidechain.source_loop", { name: track.name }) : own ? t("sidechain.source_own", { name: track.name }) : track.name,
      userText: !cycle && !own,
      usable: isSource || (own ? defaultTap(project, device, track.id) !== null : !cycle),
      taps: shown,
    });
  }
  const pickTrack = (row: SourceRow) => {
    if (!row.usable) return;
    const tap = row.track === device.sidechain ? current : defaultTap(project, device, row.track);
    if (tap) onPick(row.track, tap);
  };
  const openSub = (row: SourceRow, item: HTMLElement, focus: boolean) => {
    setSub(row.usable ? { track: row.track, item } : null);
    if (focus && row.usable) requestAnimationFrame(() => menuItems(document.getElementById(subId(row.track)))[0]?.focus());
  };
  const subRow = rows.find((r) => r.track === sub?.track) ?? null;

  return (
    <div className="eth-sidechain-picker__body">
      <div className="eth-sidechain-picker__search">
        <SearchField
          label={t("sidechain.search")}
          value={query}
          onChange={(v) => {
            setQuery(v);
            setSub(null);
          }}
          autoFocus
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              menuItems(list.current)[0]?.focus();
            } else if (e.key === "Enter") {
              // Enter takes the first matching track.
              const first = rows.find((r) => r.usable);
              if (first && q !== "") pickTrack(first);
            }
          }}
        />
      </div>
      {/* eslint-disable-next-line jsx-a11y/interactive-supports-focus -- a menu: focus is on its items (roving, menuKeyDown) */}
      <div
        ref={list}
        className="eth-menu eth-sidechain-picker__list"
        role="menu"
        aria-label={t("sidechain.sources")}
        onKeyDown={(e) => menuKeyDown(e, list.current)}
      >
        {q === "" && (
          <button type="button" role="menuitem" className="eth-menu__item" onMouseEnter={() => setSub(null)} onClick={() => onPick(null, null)}>
            <span className="eth-select-list__check" aria-hidden>
              {device.sidechain === null && <Check />}
            </span>
            <span className="eth-menu__label">{t("sidechain.none")}</span>
          </button>
        )}
        {rows.map((row) => (
          <button
            key={row.track}
            type="button"
            role="menuitem"
            aria-haspopup={row.usable ? "menu" : undefined}
            aria-expanded={row.usable ? sub?.track === row.track : undefined}
            className={clsx("eth-menu__item", sub?.track === row.track && "eth-menu__item--open")}
            {...disabledProps(!row.usable, row.label)}
            onMouseEnter={(e) => openSub(row, e.currentTarget, false)}
            onClick={() => pickTrack(row)}
            onKeyDown={(e) => {
              if (e.key !== "ArrowRight") return;
              e.preventDefault();
              openSub(row, e.currentTarget, true);
            }}
          >
            <span className="eth-select-list__check" aria-hidden>
              {device.sidechain === row.track && <Check />}
            </span>
            <span className={clsx("eth-menu__label", row.userText && "eth-user-text")}>{row.label}</span>
            {row.usable && <ChevronRight className="eth-sidechain-picker__chevron" aria-hidden />}
          </button>
        ))}
        {rows.length === 0 && <span className="eth-sidechain-picker__empty">{t("sidechain.no_match")}</span>}
      </div>
      {subRow && sub && (
        <TapSubmenu
          id={subId(subRow.track)}
          anchor={sub.item}
          label={subRow.label}
          taps={subRow.taps}
          current={device.sidechain === subRow.track ? tapKey(current) : null}
          onPick={(key) => {
            const o = subRow.taps.find((x) => x.key === key);
            if (o) onPick(subRow.track, o.tap);
          }}
          onBack={() => {
            sub.item.focus();
            setSub(null);
          }}
        />
      )}
    </div>
  );
}

const subId = (track: TrackId) => `eth-sidechain-taps-${track}`;

/** A choice of a submenu ([`TapSubmenu`]). */
interface SubOption {
  key: string;
  label: string;
  refused: string | null;
}

/**
 * MIDI input picker (`Device::SetMidiSidechain`, one undo step per change), Live's "MIDI From"
 * on an effect: the MIDI track whose notes the device's note input follows (e.g. Shifter's
 * MIDI pitch mode), pre or post its MIDI effects. Renders nothing for devices that aren't
 * audio effects with a note input (`DeviceDescriptor.midi_input`). One dropdown: "no midi
 * input", then the MIDI tracks (its own included), each with its points in a submenu; a click
 * on a track takes post fx. Tracks that would close a routing loop are disabled.
 */
export function MidiSidechainSelector({ device }: SidechainSelectorProps) {
  const t = useT();
  const { descriptor } = useDescriptor(device);
  const project = useProject();
  const send = useSend();
  const [open, setOpen] = useState(false);
  if (!descriptor || !hasNoteInput(descriptor) || !project) return null;
  const set = (source: MidiSidechain | null) => void send(cmd("Device", { type: "SetMidiSidechain", device: device.id, source }));
  const source = device.midi_sidechain ?? null;
  const sourceTrack = source === null ? null : project.tracks[source.track];
  const label =
    source === null
      ? t("midi_sidechain.none")
      : `${source.track === device.track ? t("sidechain.source_own", { name: sourceTrack?.name ?? "" }) : (sourceTrack?.name ?? "")} · ${t(POINT_LABEL[source.tap])}`;
  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      placement="bottom-start"
      className="eth-popover--menu eth-sidechain-picker"
      aria-label={t("midi_sidechain.source_label", { name: device.name })}
      trigger={(p) => (
        <button
          type="button"
          className={clsx("eth-select", "eth-select--sm", "eth-sidechain", open && "eth-select--open")}
          aria-label={t("midi_sidechain.source_label", { name: device.name })}
          title={t("midi_sidechain.source_title")}
          {...p}
        >
          <span className={clsx("eth-select__value", source !== null && source.track !== device.track && "eth-user-text")}>{label}</span>
          <ChevronDown className="eth-select__chevron" aria-hidden />
        </button>
      )}
    >
      {(close) => (
        <MidiSidechainMenu
          project={project}
          device={device}
          onPick={(s) => {
            close();
            set(s);
          }}
        />
      )}
    </Popover>
  );
}

function MidiSidechainMenu({ project, device, onPick }: { project: Project; device: Device; onPick(source: MidiSidechain | null): void }) {
  const t = useT();
  const [sub, setSub] = useState<{ track: TrackId; item: HTMLElement } | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const current = device.midi_sidechain ?? null;
  const rows = midiSidechainSources(project, device).map(({ track, cycle }) => ({
    track: track.id,
    own: track.id === device.track,
    cycle,
    label: cycle ? t("sidechain.source_loop", { name: track.name }) : track.id === device.track ? t("sidechain.source_own", { name: track.name }) : track.name,
  }));
  const options: SubOption[] = MIDI_TAPS.map((tap) => ({ key: tap, label: t(POINT_LABEL[tap]), refused: null }));
  const openSub = (row: (typeof rows)[number], item: HTMLElement, focus: boolean) => {
    setSub(row.cycle ? null : { track: row.track, item });
    if (focus && !row.cycle) requestAnimationFrame(() => menuItems(document.getElementById(midiSubId(row.track)))[0]?.focus());
  };
  const subRow = rows.find((r) => r.track === sub?.track) ?? null;
  return (
    <div className="eth-sidechain-picker__body">
      {/* eslint-disable-next-line jsx-a11y/interactive-supports-focus -- a menu: focus is on its items (roving, menuKeyDown) */}
      <div
        ref={list}
        className="eth-menu eth-sidechain-picker__list"
        role="menu"
        aria-label={t("midi_sidechain.sources")}
        onKeyDown={(e) => menuKeyDown(e, list.current)}
      >
        <button type="button" role="menuitem" className="eth-menu__item" onMouseEnter={() => setSub(null)} onClick={() => onPick(null)}>
          <span className="eth-select-list__check" aria-hidden>
            {current === null && <Check />}
          </span>
          <span className="eth-menu__label">{t("midi_sidechain.none")}</span>
        </button>
        {rows.map((row) => (
          <button
            key={row.track}
            type="button"
            role="menuitem"
            aria-haspopup={row.cycle ? undefined : "menu"}
            aria-expanded={row.cycle ? undefined : sub?.track === row.track}
            className={clsx("eth-menu__item", sub?.track === row.track && "eth-menu__item--open")}
            {...disabledProps(row.cycle, row.label)}
            onMouseEnter={(e) => openSub(row, e.currentTarget, false)}
            onClick={() => !row.cycle && onPick({ track: row.track, tap: current?.track === row.track ? current.tap : "PostFx" })}
            onKeyDown={(e) => {
              if (e.key !== "ArrowRight") return;
              e.preventDefault();
              openSub(row, e.currentTarget, true);
            }}
          >
            <span className="eth-select-list__check" aria-hidden>
              {current?.track === row.track && <Check />}
            </span>
            <span className={clsx("eth-menu__label", !row.cycle && !row.own && "eth-user-text")}>{row.label}</span>
            {!row.cycle && <ChevronRight className="eth-sidechain-picker__chevron" aria-hidden />}
          </button>
        ))}
        {rows.length === 0 && <span className="eth-sidechain-picker__empty">{t("midi_sidechain.no_tracks")}</span>}
      </div>
      {subRow && sub && (
        <TapSubmenu
          id={midiSubId(subRow.track)}
          anchor={sub.item}
          label={subRow.label}
          taps={options}
          current={current?.track === subRow.track ? current.tap : null}
          onPick={(key) => onPick({ track: subRow.track, tap: key as MidiTap })}
          onBack={() => {
            sub.item.focus();
            setSub(null);
          }}
        />
      )}
    </div>
  );
}

const midiSubId = (track: TrackId) => `eth-midi-sidechain-taps-${track}`;

/** A source track's tap points, at the side of its item (in its own portal: the list scrolls). */
function TapSubmenu({
  id,
  anchor,
  label,
  taps,
  current,
  onPick,
  onBack,
}: {
  id: string;
  anchor: HTMLElement;
  label: string;
  taps: SubOption[];
  current: string | null;
  onPick(key: string): void;
  onBack(): void;
}) {
  const t = useT();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = anchor.getBoundingClientRect();
    const w = el.offsetWidth;
    // At the item's right, or its left when there is no room.
    const left = r.right + w <= window.innerWidth ? r.right : r.left - w;
    setPos({ left: clampToViewport(left, w, window.innerWidth), top: clampToViewport(r.top, el.offsetHeight, window.innerHeight) });
  }, [anchor, taps]);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (menuKeyDown(e, ref.current)) return;
    if (e.key !== "ArrowLeft" && e.key !== "Escape") return;
    e.preventDefault();
    e.stopPropagation();
    onBack();
  };
  return createPortal(
    // eslint-disable-next-line jsx-a11y/interactive-supports-focus -- a menu: focus is on its items (roving, menuKeyDown)
    <div
      ref={ref}
      id={id}
      role="menu"
      aria-label={label}
      className="eth-popover eth-popover--menu eth-sidechain-picker__sub"
      style={pos ? { left: pos.left, top: pos.top } : { visibility: "hidden" }}
      onKeyDown={onKeyDown}
    >
      <div className="eth-menu">
        {taps.map((o) => (
          <button
            key={o.key}
            type="button"
            role="menuitem"
            aria-current={o.key === current || undefined}
            className="eth-menu__item"
            {...disabledProps(o.refused !== null, o.refused ?? undefined)}
            onClick={() => o.refused === null && onPick(o.key)}
          >
            <span className="eth-select-list__check" aria-hidden>
              {o.key === current && <Check />}
            </span>
            <span className="eth-menu__label">{o.refused ? t("sidechain.tap_refused", { tap: o.label, reason: o.refused }) : o.label}</span>
          </button>
        ))}
      </div>
    </div>,
    document.body,
  );
}
