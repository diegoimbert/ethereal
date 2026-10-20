import { useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { emit } from "@/ext";
import type { Event, ExportDownload, ExportResult } from "@/generated";
import { Button, Dialog, NumberField, Select, TextInput, Toggle } from "@/kit";
import { useProjectStore } from "@/state";
import { clipSpan } from "@/features/time-edits/commands";
import { rangeOf, useTimeSelection } from "@/features/time-edits/store";
import { useSelectedItems } from "@/timeline";
import { cmd, commandErrorText, isCommandFailed, newId, TransportContext, type EngineTransport } from "@/transport";
import { fetchDownload, saveFile } from "./download";
import { useExportAudio } from "./store";
import { formatBytes, useT, type MessageKey } from "@/i18n";
import {
  BIT_DEPTH_OPTIONS,
  buildRequest,
  CONTAINER_OPTIONS,
  DEFAULT_FORM,
  defaultStemTracks,
  depthAllowed,
  hasBitDepth,
  rateAllowed,
  estimateBytes,
  LARGE_EXPORT_BYTES,
  STEMS_HELP,
  formProblem,
  MAX_TAIL_SECONDS,
  RANGE_OPTIONS,
  RATE_OPTIONS,
  stemCandidates,
  type ExportForm,
} from "./request";

type Status =
  | { state: "idle" }
  | { state: "rendering"; job: string; progress: number }
  | { state: "done"; result: ExportResult }
  | { state: "failed"; message: string }
  | { state: "failed"; key: MessageKey };

/** A render whose engine went away (switched engines). */
const INTERRUPTED: MessageKey = "export_audio.interrupted";

function message(e: unknown): string {
  if (isCommandFailed(e)) return commandErrorText(e.error);
  return e instanceof Error ? e.message : String(e);
}

/**
 * Export audio: the project menu's "Export audio…" opens a dialog (range, format, bit depth,
 * sample rate, normalize, tail, stems). The render runs engine-side while the dialog shows
 * progress (cancelable); closed mid-render, the top bar shows the progress (a click
 * reopens it). Native hosts save into the project's `exports/` folder; the web
 * build downloads the files through the browser.
 */
export function ExportDialog() {
  const ctx = useContext(TransportContext);
  if (!ctx) return null;
  return <ExportPanel transport={ctx.transport} />;
}

/** `onEvent` subscription that always calls the latest `listener`. */
function useEvents(transport: EngineTransport, listener: (e: Event) => void): void {
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  });
  useEffect(() => transport.onEvent((e) => ref.current(e)), [transport]);
}

function ExportPanel({ transport }: { transport: EngineTransport }) {
  const t = useT();
  const project = useProjectStore((s) => s.project);
  // The selection: the arrangement's time section (Shift-drag), else the span of the
  // selected clips.
  const selectedClips = useSelectedItems("clip");
  const section = useTimeSelection((st) => rangeOf(st.selection));
  const selection = useMemo(() => {
    if (section) return { start: section.start, end: section.end };
    const span = project ? clipSpan(project, selectedClips) : null;
    return span ? { start: span.start, end: span.end } : null;
  }, [project, selectedClips, section]);
  const open = useExportAudio((s) => s.open);
  const setOpen = (o: boolean) => (o ? useExportAudio.getState().show() : useExportAudio.getState().hide());
  const [form, setForm] = useState<ExportForm>(DEFAULT_FORM);
  const [status, setStatus] = useState<Status>({ state: "idle" });
  const [downloaded, setDownloaded] = useState<ReadonlySet<string>>(new Set());
  const jobRef = useRef<string | null>(null);
  const projectId = project?.id ?? null;

  const candidates = useMemo(() => (project ? stemCandidates(project) : []), [project]);

  // A new project: fresh stem defaults, forget results (state adjusted during render).
  const [seenProject, setSeenProject] = useState<string | null>(null);
  if (projectId !== seenProject) {
    setSeenProject(projectId);
    setForm((f) => ({
      ...f,
      tracks: project ? defaultStemTracks(project) : [],
    }));
    setStatus((s) => (s.state === "rendering" ? s : { state: "idle" }));
  }

  // Another engine (connected to a server, back to the local one): a render on the old one
  // never reports here again; it is over for this dialog.
  const [seenTransport, setSeenTransport] = useState(transport);
  if (transport !== seenTransport) {
    setSeenTransport(transport);
    if (status.state === "rendering") setStatus({ state: "failed", key: INTERRUPTED });
  }
  /** Jobs cancelled here: their late `Done` only releases the files engine-side. */
  const cancelled = useRef(new Set<string>());

  const update = (patch: Partial<ExportForm>) => setForm((f) => ({ ...f, ...patch }));

  // Downloads being pulled from the engine, by token: their bytes are only released
  // engine-side once they finished.
  const inflight = useRef(new Map<string, Promise<void>>());
  // Downloads in flight: a new render would drop their engine-side bytes.
  const [pulling, setPulling] = useState(0);

  const download = useCallback(
    (d: ExportDownload): Promise<void> => {
      const p = (async () => {
        try {
          const bytes = await fetchDownload(transport, d);
          saveFile(d.name, d.mime, bytes);
          setDownloaded((s) => new Set(s).add(d.token));
        } catch (e) {
          setStatus({ state: "failed", message: message(e) });
        }
      })();
      inflight.current.set(d.token, p);
      setPulling((n) => n + 1);
      void p.finally(() => {
        setPulling((n) => n - 1);
        if (inflight.current.get(d.token) === p) inflight.current.delete(d.token);
      });
      return p;
    },
    [transport],
  );

  const release = useCallback(
    (result: ExportResult) => {
      if (result.type !== "Download") return;
      for (const d of result.downloads) {
        const pending = inflight.current.get(d.token) ?? Promise.resolve();
        void pending.then(() => transport.send(cmd("Export", { type: "Release", token: d.token }))).catch(() => undefined);
      }
    },
    [transport],
  );

  useEvents(transport, (e: Event) => {
    if (e.type !== "Export") return;
    if (cancelled.current.has(e.event.job)) {
      if (e.event.type === "Done") release(e.event.result);
      if (e.event.type !== "Progress") cancelled.current.delete(e.event.job);
      return;
    }
    if (e.event.job !== jobRef.current) return;
    const ev = e.event;
    switch (ev.type) {
      case "Progress":
        setStatus({ state: "rendering", job: ev.job, progress: ev.progress });
        break;
      case "Done":
        jobRef.current = null;
        emit("app.event", { type: "export", format: form.container.toLowerCase(), outcome: "ok" });
        setStatus({ state: "done", result: ev.result });
        if (ev.result.type === "Download") for (const d of ev.result.downloads) void download(d);
        break;
      case "Failed":
        jobRef.current = null;
        emit("app.event", { type: "export", format: form.container.toLowerCase(), outcome: "failed" });
        setStatus({ state: "failed", message: ev.message });
        break;
      case "Cancelled":
        jobRef.current = null;
        setStatus({ state: "idle" });
        break;
    }
  });

  const start = async () => {
    if (status.state === "done") release(status.result);
    const job = newId();
    jobRef.current = job;
    setDownloaded(new Set());
    setStatus({ state: "rendering", job, progress: 0 });
    try {
      await transport.send(
        cmd("Export", {
          type: "Render",
          job,
          request: buildRequest(form, selection),
        }),
      );
    } catch (e) {
      if (jobRef.current === job) {
        jobRef.current = null;
        setStatus({ state: "failed", message: message(e) });
      }
    }
  };

  // The dialog is free again at once: an engine that lost the job (it died, or answers an
  // unknown job without an event) can't leave it waiting.
  const cancel = () => {
    if (status.state !== "rendering") return;
    const job = status.job;
    cancelled.current.add(job);
    jobRef.current = null;
    setStatus({ state: "idle" });
    void transport.send(cmd("Export", { type: "Cancel", job })).catch(() => cancelled.current.delete(job));
  };

  const close = () => {
    setOpen(false);
    // Finished downloads are freed engine-side once the dialog is dismissed.
    if (status.state === "done") {
      release(status.result);
      setStatus({ state: "idle" });
    }
  };

  const rendering = status.state === "rendering";
  const problem = formProblem(form, selection);
  // The web/remote builds keep the files in memory until they are downloaded.
  const estimate = project && transport.kind !== "tauri" ? estimateBytes(form, project, selection) : 0;
  const pct = rendering ? Math.round(status.progress * 100) : 0;

  return (
    <div className="eth-export" data-feature="export">
      {rendering && !open && (
        <Button size="sm" aria-label={t("export_audio.title")} title={t("export_audio.show_progress")} onClick={() => setOpen(true)}>
          {t("export_audio.progress_button", { pct })}
        </Button>
      )}
      <Dialog
        open={open}
        onClose={close}
        title={t("export_audio.title")}
        className="eth-export__dialog"
        footer={
          rendering ? (
            <Button tone="danger" onClick={cancel}>
              {t("export_audio.cancel")}
            </Button>
          ) : (
            <>
              <Button tone="ghost" onClick={close}>
                {t("common.close")}
              </Button>
              <Button
                tone="accent"
                disabled={problem !== null || pulling > 0}
                title={pulling > 0 ? t("export_audio.wait_downloads") : problem ? t(problem) : undefined}
                onClick={() => void start()}
              >
                {t("export_audio.export")}
              </Button>
            </>
          )
        }
      >
        <div className="eth-export__form">
          <label className="eth-export__row">
            <span>{t("export_audio.range")}</span>
            <Select
              size="sm"
              aria-label={t("export_audio.range")}
              disabled={rendering}
              options={RANGE_OPTIONS.map((o) => ({
                ...o,
                label: t(o.label),
                disabled: o.value === "selection" && !selection,
              }))}
              value={form.range}
              onChange={(range) => update({ range })}
            />
          </label>
          <label className="eth-export__row">
            <span>{t("export_audio.format")}</span>
            <Select
              size="sm"
              aria-label={t("export_audio.format")}
              disabled={rendering}
              options={CONTAINER_OPTIONS.map((o) => ({ ...o, label: t(o.label) }))}
              value={form.container}
              onChange={(container) =>
                update({
                  container,
                  bitDepth: depthAllowed(container, form.bitDepth) ? form.bitDepth : "Int24",
                  rate: rateAllowed(container, form.rate) ? form.rate : "engine",
                })
              }
            />
          </label>
          <label className="eth-export__row">
            <span>{t("export_audio.bit_depth")}</span>
            <Select
              size="sm"
              aria-label={t("export_audio.bit_depth")}
              disabled={rendering || !hasBitDepth(form.container)}
              options={BIT_DEPTH_OPTIONS.map((o) => ({
                ...o,
                label: t(o.label),
                disabled: !depthAllowed(form.container, o.value),
              }))}
              value={form.bitDepth}
              onChange={(bitDepth) => update({ bitDepth })}
            />
          </label>
          <label className="eth-export__row">
            <span>{t("export_audio.sample_rate")}</span>
            <Select
              size="sm"
              aria-label={t("export_audio.sample_rate")}
              disabled={rendering}
              options={RATE_OPTIONS.map((o) => ({
                ...o,
                label: t(o.label),
                disabled: !rateAllowed(form.container, o.value),
              }))}
              value={form.rate}
              onChange={(rate) => update({ rate })}
            />
          </label>
          <label className="eth-export__row">
            <span>{t("export_audio.tail")}</span>
            <NumberField
              size="sm"
              aria-label={t("export_audio.tail")}
              disabled={rendering}
              value={form.tail}
              min={0}
              max={MAX_TAIL_SECONDS}
              step={0.5}
              precision={1}
              unit="s"
              onChange={(tail) => update({ tail })}
            />
          </label>
          <label className="eth-export__row">
            <span>{t("export_audio.file_name")}</span>
            <TextInput
              size="sm"
              aria-label={t("export_audio.file_name")}
              disabled={rendering}
              // The project's name (user content).
              data-i18n-exempt
              placeholder={project?.settings.name ?? ""}
              value={form.name}
              onChange={(e) => update({ name: e.target.value })}
            />
          </label>
          <div className="eth-export__row">
            <Toggle
              size="sm"
              label={t("export_audio.normalize")}
              checked={form.normalize}
              disabled={rendering}
              onChange={(normalize) => update({ normalize })}
            />
            <Toggle size="sm" label={t("export_audio.stems")} checked={form.stems} disabled={rendering} onChange={(stems) => update({ stems })} />
          </div>
          {form.stems && <p className="eth-export__help">{t(STEMS_HELP)}</p>}
          {form.stems && (
            <ul className="eth-export__tracks" aria-label={t("export_audio.stem_tracks")}>
              {candidates.map((t) => (
                <li key={t.id}>
                  <Toggle
                    size="sm"
                    label={<span className="eth-user-text">{t.name}</span>}
                    disabled={rendering}
                    checked={form.tracks.includes(t.id)}
                    onChange={(on) =>
                      update({
                        tracks: on ? [...form.tracks, t.id] : form.tracks.filter((id) => id !== t.id),
                      })
                    }
                  />
                </li>
              ))}
            </ul>
          )}
        </div>
        {estimate > LARGE_EXPORT_BYTES && !rendering && (
          <p className="eth-export__warning" role="note">
            {t("export_audio.large", { size: formatBytes(estimate, { digits: 0 }) })}
          </p>
        )}
        <ExportStatus status={status} downloaded={downloaded} onDownload={(d) => void download(d)} />
      </Dialog>
    </div>
  );
}

interface ExportStatusProps {
  status: Status;
  downloaded: ReadonlySet<string>;
  onDownload(d: ExportDownload): void;
}

function ExportStatus({ status, downloaded, onDownload }: ExportStatusProps) {
  const t = useT();
  switch (status.state) {
    case "idle":
      return null;
    case "rendering":
      return (
        <div className="eth-export__status" role="status">
          <progress className="eth-export__progress" aria-label={t("export_audio.progress")} max={1} value={status.progress} />
          <span>{Math.round(status.progress * 100)}%</span>
        </div>
      );
    case "failed":
      return (
        <p className="eth-export__error" role="alert">
          {"key" in status ? t(status.key) : status.message}
        </p>
      );
    case "done":
      return status.result.type === "Files" ? (
        <div className="eth-export__status" role="status" data-testid="export-done">
          <span>{t("export_audio.saved_in_folder")}</span>
          <ul className="eth-export__files">
            {status.result.files.map((f) => (
              <li key={f} className="eth-user-text">
                {f}
              </li>
            ))}
          </ul>
        </div>
      ) : (
        <div className="eth-export__status" role="status" data-testid="export-done">
          <ul className="eth-export__files">
            {status.result.downloads.map((d) => (
              <li key={d.token}>
                <span className="eth-user-text">{d.name}</span>
                <Button size="sm" tone="ghost" onClick={() => onDownload(d)}>
                  {downloaded.has(d.token) ? t("export_audio.download_again") : t("export_audio.download")}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      );
  }
}
