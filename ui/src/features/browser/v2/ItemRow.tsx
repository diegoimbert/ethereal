import clsx from "clsx";
import type { DragEvent, KeyboardEvent, MouseEvent, ReactNode } from "react";
import { AudioLines, FolderKanban, Music, SlidersHorizontal, Star, Volume2 } from "lucide-react";
import type { LibraryItem, LibraryItemKind } from "@/generated";
import { Badge, IconButton } from "@/kit";
import { writeBrowserDrag, type BrowserDragPayload } from "../dragPayload";
import { formatDuration, itemDetail, shortKey } from "./model";

const ITEM_ICON: Record<LibraryItemKind, ReactNode> = {
  Audio: <AudioLines />,
  Midi: <Music />,
  Preset: <SlidersHorizontal />,
  Project: <FolderKanban />,
};

export interface ItemRowProps {
  item: LibraryItem;
  previewing: boolean;
  /** Click / Enter: preview audio (toggle), load a preset, open a project. */
  onActivate(item: LibraryItem): void;
  /** Double-click (presets, projects). */
  onOpen(item: LibraryItem): void;
  onFavourite(item: LibraryItem, favourite: boolean): void;
  onTag(tag: string): void;
  onContextMenu(e: MouseEvent, item: LibraryItem): void;
}

/** One index result: kind icon, name, folder / device, bpm · key · duration, tags, star. */
export function ItemRow({ item, previewing, onActivate, onOpen, onFavourite, onTag, onContextMenu }: ItemRowProps) {
  const audio = item.kind === "Audio";
  const draggable = (audio || item.kind === "Midi") && !!item.source;
  const detail = itemDetail(item);
  const { bpm, key, duration_seconds: duration } = item.meta;

  const onDragStart = (e: DragEvent<HTMLDivElement>) => {
    if (!item.source) return;
    const payload: BrowserDragPayload = { version: 1, kind: "media", source: item.source, name: item.name, file_kind: audio ? "Audio" : "Midi" };
    writeBrowserDrag(e.dataTransfer, payload);
  };
  const title = audio
    ? `${item.name}: click to preview, drag to a track`
    : item.kind === "Midi"
      ? `${item.name}: drag to a track`
      : item.kind === "Preset"
        ? `${item.name}: double-click to load on the selected track's device`
        : `${item.name}: double-click to open`;

  return (
    <li className="eth-browser-v2__item" onContextMenu={(e) => onContextMenu(e, item)}>
      <div
        className={clsx(
          "eth-browser__row",
          `eth-browser__row--${item.kind.toLowerCase()}`,
          previewing && "eth-browser__row--previewing",
        )}
        role="button"
        tabIndex={0}
        aria-label={item.name}
        aria-pressed={audio ? previewing : undefined}
        data-row=""
        data-path={item.id}
        title={title}
        draggable={draggable}
        onDragStart={draggable ? onDragStart : undefined}
        onClick={(e) => {
          e.currentTarget.focus();
          if (audio) onActivate(item);
        }}
        onDoubleClick={() => !audio && onOpen(item)}
        onKeyDown={(e: KeyboardEvent) => {
          if (e.key === "Enter" && e.target === e.currentTarget) (audio ? onActivate : onOpen)(item);
        }}
      >
        <span className="eth-browser__icon" aria-hidden>
          {previewing ? <Volume2 /> : ITEM_ICON[item.kind]}
        </span>
        <span className="eth-browser__name">{item.name}</span>
        {detail && <span className="eth-browser__detail">{detail}</span>}
        {bpm !== null && bpm !== undefined && <Badge className="eth-browser-v2__badge">{Math.round(bpm)} BPM</Badge>}
        {key && <Badge className="eth-browser-v2__badge">{shortKey(key)}</Badge>}
        {duration !== null && duration !== undefined && <span className="eth-browser__size">{formatDuration(duration)}</span>}
      </div>
      {item.tags.length > 0 && (
        <span className="eth-browser-v2__tags">
          {item.tags.map((t) => (
            <button key={t} type="button" className="eth-browser-v2__tag" title={`Filter by “${t}”`} onClick={() => onTag(t)}>
              {t}
            </button>
          ))}
        </span>
      )}
      <IconButton
        size="sm"
        tone="ghost"
        className={clsx("eth-browser-v2__star", item.favourite && "eth-browser-v2__star--on")}
        label={item.favourite ? `Unfavourite ${item.name}` : `Favourite ${item.name}`}
        active={item.favourite}
        icon={<Star />}
        tabIndex={-1}
        onClick={() => onFavourite(item, !item.favourite)}
      />
    </li>
  );
}
