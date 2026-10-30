import clsx from "clsx";
import { FileAudio, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import type { MediaId, MediaRef } from "@/generated";
import { Badge, Button, Dialog, IconButton, SearchField, useConfirm } from "@/kit";
import { formatSeconds } from "@/timeline/format";
import { useProjectStore } from "@/state";
import type { EngineTransport } from "@/transport";
import { t, useT } from "@/i18n";
import { closeMediaManager, openRelink, removeMedia, useMediaRefs } from "./store";
import { locationLabel } from "./sources";
import { isUnused, mediaUsage, type MediaUsage } from "./usage";

/** "3 clips · 1 device", "unused", "freeze render". */
function usageText(u: MediaUsage | undefined): string {
  if (!u || isUnused(u)) return t("media_manager.unused");
  const parts: string[] = [];
  if (u.clips.length) parts.push(t("media_manager.uses.clips", { count: u.clips.length }));
  if (u.devices.length) parts.push(t("media_manager.uses.devices", { count: u.devices.length }));
  if (u.freezes.length) parts.push(t("media_manager.uses.freezes", { count: u.freezes.length }));
  return parts.join(" · ");
}

/** What removing `media` does, for the confirmation. */
function removeBody(media: MediaRef[], usage: Map<MediaId, MediaUsage>): string {
  const clips = media.reduce((n, m) => n + (usage.get(m.id)?.clips.length ?? 0), 0);
  const devices = new Set(media.flatMap((m) => usage.get(m.id)?.devices ?? [])).size;
  const lines = [
    media.length === 1 ? t("media_manager.remove.body_one", { name: media[0]!.name }) : t("media_manager.remove.body_many", { count: media.length }),
  ];
  if (clips) lines.push(t("media_manager.remove.clips", { count: clips }));
  if (devices) lines.push(t("media_manager.remove.devices", { count: devices }));
  lines.push(t("media_manager.remove.undo"));
  return lines.join(" ");
}

function askRemove(transport: EngineTransport, media: MediaRef[], usage: Map<MediaId, MediaUsage>): void {
  useConfirm.getState().ask({
    title: media.length === 1 ? t("media_manager.remove.title_one") : t("media_manager.remove.title_many", { count: media.length }),
    body: removeBody(media, usage),
    action: t("media_manager.remove.action"),
    danger: true,
    run: () => void removeMedia(transport, media.map((m) => m.id)),
  });
}

function Row({ media, usage, transport }: { media: MediaRef; usage: Map<MediaId, MediaUsage>; transport: EngineTransport }) {
  const t = useT();
  const missing = useMediaRefs((s) => s.missing.has(media.id));
  const u = usage.get(media.id);
  const frozen = (u?.freezes.length ?? 0) > 0;
  const seconds = media.sample_rate > 0 ? media.frames / media.sample_rate : 0;
  return (
    <li className={clsx("eth-media-refs__row eth-media-manager__row", isUnused(u) && "eth-media-manager__row--unused")} data-testid="media-manager-row" data-media={media.id}>
      <FileAudio className="eth-media-refs__icon" aria-hidden />
      <div className="eth-media-refs__info">
        <span className="eth-media-refs__name eth-user-text">{media.name}</span>
        <span className={media.location.type === "External" ? "eth-media-refs__path eth-user-text" : "eth-media-refs__path"} title={locationLabel(media)}>
          {locationLabel(media)}
        </span>
        <span className="eth-media-manager__meta">
          {t("media_manager.format", { duration: formatSeconds(seconds, false), rate: media.sample_rate / 1000, channels: media.channels })}
        </span>
      </div>
      <span className="eth-media-manager__usage" data-testid="media-manager-usage">
        {usageText(u)}
      </span>
      {missing ? (
        <Button size="sm" onClick={() => openRelink(media.id)} title={t("media_refs.menu.relink_sample")}>
          <Badge tone="warn">{t("media_refs.missing")}</Badge> {t("media_refs.locate")}
        </Button>
      ) : (
        <span />
      )}
      <IconButton
        size="sm"
        tone="ghost"
        icon={<Trash2 aria-hidden />}
        label={frozen ? t("media_manager.remove.frozen") : t("media_manager.remove.one")}
        disabled={frozen}
        onClick={() => askRemove(transport, [media], usage)}
        data-testid="media-manager-remove"
      />
    </li>
  );
}

/**
 * The media manager: every sample of the project, with where it lives and what uses it
 * (clips, samplers, freeze renders); remove one, or every unused one, after a confirmation
 * that says what it unlinks (one undo step). Missing samples can be relinked from here.
 */
export function MediaManagerDialog({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const open = useMediaRefs((s) => s.manager);
  const error = useMediaRefs((s) => s.error);
  const project = useProjectStore((s) => s.project);
  const [query, setQuery] = useState("");
  const usage = useMemo(() => (project ? mediaUsage(project) : new Map<MediaId, MediaUsage>()), [project]);
  const all = useMemo(() => Object.values(project?.media ?? {}).sort((a, b) => a.name.localeCompare(b.name)), [project?.media]);
  const q = query.trim().toLowerCase();
  const rows = q ? all.filter((m) => m.name.toLowerCase().includes(q) || locationLabel(m).toLowerCase().includes(q)) : all;
  const unused = all.filter((m) => isUnused(usage.get(m.id)));
  return (
    <Dialog
      open={open}
      onClose={closeMediaManager}
      title={t("media_manager.title")}
      className="eth-media-refs eth-media-manager"
      testId="media-manager"
      footer={
        <>
          <Button disabled={unused.length === 0} onClick={() => askRemove(transport, unused, usage)} data-testid="media-manager-remove-unused">
            {t("media_manager.remove_unused", { count: unused.length })}
          </Button>
          <Button tone="accent" onClick={closeMediaManager}>
            {t("common.done")}
          </Button>
        </>
      }
    >
      <p className="eth-media-refs__summary" data-testid="media-manager-summary">
        {all.length === 0 ? t("media_manager.empty") : t("media_manager.summary", { count: all.length, unused: unused.length })}
      </p>
      {all.length > 0 && <SearchField value={query} onChange={setQuery} label={t("media_manager.filter")} className="eth-media-manager__filter" />}
      {rows.length > 0 && (
        <ul className="eth-media-refs__list eth-media-manager__list" aria-label={t("media_refs.samples")}>
          {rows.map((m) => (
            <Row key={m.id} media={m} usage={usage} transport={transport} />
          ))}
        </ul>
      )}
      {error && (
        <p className="eth-media-refs__error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
