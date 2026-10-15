import clsx from "clsx";
import type { DragEvent, KeyboardEvent, MouseEvent, ReactNode } from "react";
import { AudioLines, FolderKanban, Music, SlidersHorizontal, Star, Volume2 } from "lucide-react";
import type { LibraryItem, LibraryItemKind } from "@/generated";
import { Badge, IconButton } from "@/kit";
import { useT } from "@/i18n";
import { libraryItemPayload, writeBrowserDrag } from "../dragPayload";
import { formatDuration, isFactoryRoot, itemDetail, shortKey } from "./model";

const ITEM_ICON: Record<LibraryItemKind, ReactNode> = {
  Audio: <AudioLines />,
  Midi: <Music />,
  Preset: <SlidersHorizontal />,
  Project: <FolderKanban />,
};

export interface ItemRowProps {
  item: LibraryItem;
  previewing: boolean;
  /** The row the search field's arrow keys are on. */
  active?: boolean;
  /** Click / Enter: preview audio (toggle), load a preset, open a project. */
  onActivate(item: LibraryItem): void;
  /** Double-click (presets, projects). */
  onOpen(item: LibraryItem): void;
  onFavourite(item: LibraryItem, favourite: boolean): void;
  onTag(tag: string): void;
  onContextMenu(e: MouseEvent, item: LibraryItem): void;
}

/** One index result: kind icon, name, folder / device, bpm · key · duration, tags, star. */
export function ItemRow({ item, previewing, active, onActivate, onOpen, onFavourite, onTag, onContextMenu }: ItemRowProps) {
  const t = useT();
  const audio = item.kind === "Audio";
  const draggable = (audio || item.kind === "Midi") && !!item.source;
  const detail = itemDetail(item);
  // Factory content (presets, bundled samples) is app text; the user's own items keep their case.
  const user = !isFactoryRoot(item.root);
  const { bpm, key, duration_seconds: duration } = item.meta;

  const onDragStart = (e: DragEvent<HTMLDivElement>) => {
    const payload = libraryItemPayload(item);
    if (payload) writeBrowserDrag(e.dataTransfer, payload);
  };
  const title = audio
    ? t("browser.row.preview_hint", { name: item.name })
    : item.kind === "Midi"
      ? t("browser.row.drag_hint", { name: item.name })
      : item.kind === "Preset"
        ? t("browser.row.preset_hint", { name: item.name })
        : t("browser.row.open_hint", { name: item.name });

  return (
    <li className="eth-browser-v2__item" onContextMenu={(e) => onContextMenu(e, item)}>
      <div
        className={clsx(
          "eth-browser__row",
          `eth-browser__row--${item.kind.toLowerCase()}`,
          previewing && "eth-browser__row--previewing",
          active && "eth-browser__row--active",
        )}
        role="button"
        tabIndex={0}
        aria-label={item.name}
        aria-pressed={audio ? previewing : undefined}
        // Its label and title are the user's file name.
        data-i18n-exempt={user || undefined}
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
        <span className={clsx("eth-browser__name", user && "eth-user-text")}>{item.name}</span>
        {detail && <span className={clsx("eth-browser__detail", user && item.kind !== "Project" && item.kind !== "Preset" && "eth-user-text")}>{detail}</span>}
        {bpm !== null && bpm !== undefined && <Badge className="eth-browser-v2__badge">{Math.round(bpm)} bpm</Badge>}
        {key && <Badge className="eth-browser-v2__badge">{shortKey(key)}</Badge>}
        {duration !== null && duration !== undefined && <span className="eth-browser__size">{formatDuration(duration)}</span>}
      </div>
      {item.tags.length > 0 && (
        <span className="eth-browser-v2__tags">
          {item.tags.map((tag) => (
            <button
              key={tag}
              type="button"
              className={clsx("eth-browser-v2__tag", user && "eth-user-text")}
              title={t("browser.menu.filter_by", { tag })}
              onClick={() => onTag(tag)}
            >
              {tag}
            </button>
          ))}
        </span>
      )}
      <IconButton
        size="sm"
        tone="ghost"
        className={clsx("eth-browser-v2__star", item.favourite && "eth-browser-v2__star--on")}
        label={item.favourite ? t("browser.item.unfavourite", { name: item.name }) : t("browser.item.favourite", { name: item.name })}
        active={item.favourite}
        icon={<Star />}
        tabIndex={-1}
        onClick={() => onFavourite(item, !item.favourite)}
      />
    </li>
  );
}
