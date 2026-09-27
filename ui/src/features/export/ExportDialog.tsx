import {
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type { Event, ExportDownload, ExportResult } from "@/generated";
import { Button, Dialog, NumberField, Select, TextInput, Toggle } from "@/kit";
import { useProjectStore } from "@/state";
import { useTimeRangeSelection } from "@/timeline/selection";
import {
  cmd,
  isCommandFailed,
  newId,
  TransportContext,
  type EngineTransport,
} from "@/transport";
import { fetchDownload, saveFile } from "./download";
import {
  BIT_DEPTH_OPTIONS,
  buildRequest,
  CONTAINER_OPTIONS,
  DEFAULT_FORM,
  defaultStemTracks,
  depthAllowed,
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
  | { state: "failed"; message: string };

function message(e: unknown): string {
  if (isCommandFailed(e)) return e.error.message || e.error.code;
  return e instanceof Error ? e.message : String(e);
}

/**
 * Export audio: an "Export" button in the top bar opens a dialog (range, format, bit depth,
 * sample rate, normalize, tail, stems). The render runs engine-side while the dialog shows
 * progress (cancelable). Native hosts save into the project's `exports/` folder; the web
 * build downloads the files through the browser.
 */
export function ExportDialog() {
  const ctx = useContext(TransportContext);
  if (!ctx) return null;
  return (
    <ExportPanel
      transport={ctx.transport}
      connected={ctx.connection.status === "connected"}
    />
  );
}

/** `onEvent` subscription that always calls the latest `listener`. */
function useEvents(
  transport: EngineTransport,
  listener: (e: Event) => void,
): void {
  const ref = useRef(listener);
  useEffect(() => {
    ref.current = listener;
  });
  useEffect(() => transport.onEvent((e) => ref.current(e)), [transport]);
}

function ExportPanel({
  transport,
  connected,
}: {
  transport: EngineTransport;
  connected: boolean;
}) {
  const project = useProjectStore((s) => s.project);
  const selection = useTimeRangeSelection();
  const [open, setOpen] = useState(false);
  const [form, setForm] = useState<ExportForm>(DEFAULT_FORM);
  const [status, setStatus] = useState<Status>({ state: "idle" });
  const [downloaded, setDownloaded] = useState<ReadonlySet<string>>(new Set());
  const jobRef = useRef<string | null>(null);
  const projectId = project?.id ?? null;

  const candidates = useMemo(
    () => (project ? stemCandidates(project) : []),
    [project],
  );

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

  const update = (patch: Partial<ExportForm>) =>
    setForm((f) => ({ ...f, ...patch }));

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
        if (inflight.current.get(d.token) === p)
          inflight.current.delete(d.token);
      });
      return p;
    },
    [transport],
  );

  useEvents(transport, (e: Event) => {
    if (e.type !== "Export" || e.event.job !== jobRef.current) return;
    const ev = e.event;
    switch (ev.type) {
      case "Progress":
        setStatus({ state: "rendering", job: ev.job, progress: ev.progress });
        break;
      case "Done":
        jobRef.current = null;
        setStatus({ state: "done", result: ev.result });
        if (ev.result.type === "Download")
          for (const d of ev.result.downloads) void download(d);
        break;
      case "Failed":
        jobRef.current = null;
        setStatus({ state: "failed", message: ev.message });
        break;
      case "Cancelled":
        jobRef.current = null;
        setStatus({ state: "idle" });
        break;
    }
  });

  const release = useCallback(
    (result: ExportResult) => {
      if (result.type !== "Download") return;
      for (const d of result.downloads) {
        const pending = inflight.current.get(d.token) ?? Promise.resolve();
        void pending
          .then(() =>
            transport.send(cmd("Export", { type: "Release", token: d.token })),
          )
          .catch(() => undefined);
      }
    },
    [transport],
  );

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

  const cancel = () => {
    if (status.state !== "rendering") return;
    void transport
      .send(cmd("Export", { type: "Cancel", job: status.job }))
      .catch(() => undefined);
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
  const estimate =
    project && transport.kind !== "tauri"
      ? estimateBytes(form, project, selection)
      : 0;
  const pct = rendering ? Math.round(status.progress * 100) : 0;

  return (
    <div className="eth-export" data-feature="export">
      <Button
        size="sm"
        aria-label="Export audio"
        title="Export the arrangement to an audio file (mix or stems)"
        disabled={!connected || !project}
        active={open}
        onClick={() => setOpen(true)}
      >
        {rendering ? `Export ${pct}%` : "Export"}
      </Button>
      <Dialog
        open={open}
        onClose={close}
        title="Export audio"
        className="eth-export__dialog"
        footer={
          rendering ? (
            <Button tone="danger" onClick={cancel}>
              Cancel export
            </Button>
          ) : (
            <>
              <Button tone="ghost" onClick={close}>
                Close
              </Button>
              <Button
                tone="accent"
                disabled={problem !== null || pulling > 0}
                title={
                  pulling > 0
                    ? "Wait for the downloads to finish"
                    : (problem ?? undefined)
                }
                onClick={() => void start()}
              >
                Export
              </Button>
            </>
          )
        }
      >
        <div className="eth-export__form">
          <label className="eth-export__row">
            <span>Range</span>
            <Select
              size="sm"
              aria-label="Range"
              disabled={rendering}
              options={RANGE_OPTIONS.map((o) => ({
                ...o,
                disabled: o.value === "selection" && !selection,
              }))}
              value={form.range}
              onChange={(range) => update({ range })}
            />
          </label>
          <label className="eth-export__row">
            <span>Format</span>
            <Select
              size="sm"
              aria-label="Format"
              disabled={rendering}
              options={CONTAINER_OPTIONS}
              value={form.container}
              onChange={(container) =>
                update({
                  container,
                  bitDepth: depthAllowed(container, form.bitDepth)
                    ? form.bitDepth
                    : "Int24",
                })
              }
            />
          </label>
          <label className="eth-export__row">
            <span>Bit depth</span>
            <Select
              size="sm"
              aria-label="Bit depth"
              disabled={rendering}
              options={BIT_DEPTH_OPTIONS.map((o) => ({
                ...o,
                disabled: !depthAllowed(form.container, o.value),
              }))}
              value={form.bitDepth}
              onChange={(bitDepth) => update({ bitDepth })}
            />
          </label>
          <label className="eth-export__row">
            <span>Sample rate</span>
            <Select
              size="sm"
              aria-label="Sample rate"
              disabled={rendering}
              options={RATE_OPTIONS}
              value={form.rate}
              onChange={(rate) => update({ rate })}
            />
          </label>
          <label className="eth-export__row">
            <span>Tail</span>
            <NumberField
              size="sm"
              aria-label="Tail"
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
            <span>File name</span>
            <TextInput
              size="sm"
              aria-label="File name"
              disabled={rendering}
              placeholder={project?.settings.name ?? ""}
              value={form.name}
              onChange={(e) => update({ name: e.target.value })}
            />
          </label>
          <div className="eth-export__row">
            <Toggle
              size="sm"
              label="Normalize"
              checked={form.normalize}
              disabled={rendering}
              onChange={(normalize) => update({ normalize })}
            />
            <Toggle
              size="sm"
              label="Stems"
              checked={form.stems}
              disabled={rendering}
              onChange={(stems) => update({ stems })}
            />
          </div>
          {form.stems && <p className="eth-export__help">{STEMS_HELP}</p>}
          {form.stems && (
            <ul className="eth-export__tracks" aria-label="Stem tracks">
              {candidates.map((t) => (
                <li key={t.id}>
                  <Toggle
                    size="sm"
                    label={t.name}
                    disabled={rendering}
                    checked={form.tracks.includes(t.id)}
                    onChange={(on) =>
                      update({
                        tracks: on
                          ? [...form.tracks, t.id]
                          : form.tracks.filter((id) => id !== t.id),
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
            Large export (about {Math.round(estimate / 1e6)} MB): the browser
            keeps the files in memory until they are downloaded.
          </p>
        )}
        <ExportStatus
          status={status}
          downloaded={downloaded}
          onDownload={(d) => void download(d)}
        />
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
  switch (status.state) {
    case "idle":
      return null;
    case "rendering":
      return (
        <div className="eth-export__status" role="status">
          <progress
            className="eth-export__progress"
            aria-label="Export progress"
            max={1}
            value={status.progress}
          />
          <span>{Math.round(status.progress * 100)}%</span>
        </div>
      );
    case "failed":
      return (
        <p className="eth-export__error" role="alert">
          {status.message}
        </p>
      );
    case "done":
      return status.result.type === "Files" ? (
        <div
          className="eth-export__status"
          role="status"
          data-testid="export-done"
        >
          <span>Saved in the project folder:</span>
          <ul className="eth-export__files">
            {status.result.files.map((f) => (
              <li key={f}>{f}</li>
            ))}
          </ul>
        </div>
      ) : (
        <div
          className="eth-export__status"
          role="status"
          data-testid="export-done"
        >
          <ul className="eth-export__files">
            {status.result.downloads.map((d) => (
              <li key={d.token}>
                <span>{d.name}</span>
                <Button size="sm" tone="ghost" onClick={() => onDownload(d)}>
                  {downloaded.has(d.token) ? "Download again" : "Download"}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      );
  }
}
