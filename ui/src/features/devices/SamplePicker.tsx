/**
 * The Sampler's sample picker: the library index's audio samples (`Browser::Query`, as the
 * browser's search) in the device library's popover chrome. The search field has focus; the
 * first result is active right after typing (Enter picks it, ↑ ↓ move and preview it, like
 * the browser's rows); a click on a row picks it, its speaker icon previews it. Picking
 * hands the row's browser drag payload to `onPick`, so it loads exactly like a drop.
 */
import "./deviceLibrary.css";
import clsx from "clsx";
import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AudioLines, Search, Volume2, X } from "lucide-react";
import type { LibraryItem } from "@/generated";
import { libraryItemPayload, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { ALL_ROOTS, buildQuery, formatDuration, isFactoryRoot, itemDetail, loadSync } from "@/features/browser/v2/model";
import { useItemPreview, type PreviewTarget } from "@/features/browser/v2/preview";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { TextInput } from "@/kit";
import { cmd } from "@/transport";
import { useT } from "@/i18n";

interface Results {
  /** The search text these results are for. */
  text: string;
  items: LibraryItem[];
  total: number;
}

const target = (item: LibraryItem): PreviewTarget | null => (item.source ? { key: item.id, source: item.source, item: item.id } : null);

export function SamplePicker({ onPick }: { onPick(payload: BrowserDragPayload): void }) {
  const t = useT();
  const { transport, send } = useEngineCommands();
  const [text, setText] = useState("");
  const [results, setResults] = useState<Results | null>(null);
  const [active, setActive] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Enter typed before the results of the current text arrived: picks the first one then.
  const pickOnArrival = useRef(false);
  const onPickRef = useRef(onPick);
  useEffect(() => {
    onPickRef.current = onPick;
  });
  const [sync] = useState(loadSync);
  const preview = useItemPreview(transport, send, sync, setError);
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    let alive = true;
    const query = buildQuery({ root: ALL_ROOTS, folder: "" }, { text, kind: "Audio", tags: [], favouritesOnly: false, sort: "Name" }, []);
    void send(cmd("Browser", { type: "Query", query })).then((r) => {
      if (!alive || r?.type !== "BrowserPage") return;
      const items = r.page.items.filter((i) => i.kind === "Audio" && i.source);
      setResults({ text, items, total: r.page.total });
      const first = items[0] && libraryItemPayload(items[0]);
      if (pickOnArrival.current && first) onPickRef.current(first);
      pickOnArrival.current = false;
    });
    return () => {
      alive = false;
    };
  }, [send, text]);

  // A preview still playing stops with the picker.
  const previewing = useRef(false);
  useEffect(() => {
    previewing.current = preview.previewing !== null;
  });
  useEffect(() => () => void (previewing.current && send(cmd("Media", { type: "StopPreview" }))), [send]);

  // The last results stay shown while the new ones load (no flash of an empty list).
  const current = results?.text === text ? results : null;
  const items = results?.items ?? [];
  const activeId = items.some((i) => i.id === active) ? active : (items[0]?.id ?? null);

  const pick = (item: LibraryItem) => {
    const payload = libraryItemPayload(item);
    if (payload) onPick(payload);
  };

  const onSearchKey = (ev: KeyboardEvent<HTMLInputElement>) => {
    pickOnArrival.current = false;
    if (ev.key === "Escape" && text) {
      ev.stopPropagation();
      setText("");
      return;
    }
    if (ev.key === "Enter") {
      ev.preventDefault();
      const item = items.find((i) => i.id === activeId);
      if (!current) pickOnArrival.current = true;
      else if (item) pick(item);
      return;
    }
    if (ev.key !== "ArrowDown" && ev.key !== "ArrowUp") return;
    ev.preventDefault();
    const i = items.findIndex((x) => x.id === activeId);
    const to = ev.key === "ArrowDown" ? Math.min(items.length - 1, i + 1) : i < 0 ? items.length - 1 : Math.max(0, i - 1);
    const next = items[to];
    if (!next || next.id === activeId) return;
    setActive(next.id);
    Array.from(list.current?.querySelectorAll<HTMLElement>("[data-key]") ?? [])
      .find((el) => el.dataset.key === next.id)
      ?.scrollIntoView?.({ block: "nearest" });
    const tgt = target(next);
    if (tgt) void preview.play(tgt);
  };

  return (
    <div className="eth-devlib eth-sample-picker" data-feature="sample-picker">
      <div className="eth-devlib__search">
        <span className="eth-devlib__field">
          <Search className="eth-devlib__search-icon" aria-hidden />
          <TextInput
            size="sm"
            type="search"
            className="eth-devlib__search-input"
            aria-label={t("devices.samples.search")}
            placeholder={t("devices.samples.search")}
            value={text}
            onChange={(e) => setText(e.target.value)}
            onKeyDown={onSearchKey}
            autoFocus
          />
          {text && (
            <button type="button" className="eth-devlib__search-clear" aria-label={t("devices.library.clear_search")} onClick={() => setText("")}>
              <X aria-hidden />
            </button>
          )}
        </span>
      </div>
      <ul ref={list} className="eth-devlib__list" aria-label={t("devices.samples.list")} aria-busy={!current}>
        {items.map((item) => {
          const user = !isFactoryRoot(item.root);
          const detail = itemDetail(item);
          const duration = item.meta.duration_seconds;
          const on = preview.previewing === item.id;
          const tgt = target(item);
          return (
            <li key={item.id}>
              <div
                className={clsx("eth-devlib__row", activeId === item.id && "eth-devlib__row--active")}
                data-key={item.id}
                aria-label={item.name}
                role="button"
                title={t("devices.samples.load", { name: item.name })}
                onClick={() => pick(item)}
              >
                <button
                  type="button"
                  className="eth-devlib__icon eth-sample-picker__preview"
                  aria-label={on ? t("devices.samples.stop_preview", { name: item.name }) : t("devices.samples.preview", { name: item.name })}
                  aria-pressed={on}
                  tabIndex={-1}
                  onClick={(e) => {
                    e.stopPropagation();
                    if (tgt) void preview.toggle(tgt);
                  }}
                >
                  {on ? <Volume2 aria-hidden /> : <AudioLines aria-hidden />}
                </button>
                <span className={clsx("eth-devlib__name", user && "eth-user-text")}>{item.name}</span>
                {detail && <span className={clsx("eth-devlib__vendor", user && "eth-user-text")}>{detail}</span>}
                {duration !== null && duration !== undefined && <span className="eth-devlib__vendor">{formatDuration(duration)}</span>}
              </div>
            </li>
          );
        })}
        {current && items.length === 0 && <li className="eth-devlib__empty">{text.trim() ? t("devices.library.no_match") : t("devices.samples.empty")}</li>}
      </ul>
      <div className="eth-devlib__footer">
        <span className="eth-devlib__status" role="status">
          {error ?? (current ? t("devices.samples.count", { count: current.total }) : null)}
        </span>
      </div>
    </div>
  );
}
