import "./folders.css";
import clsx from "clsx";
import type { CSSProperties } from "react";
import { X } from "lucide-react";
import { IconButton } from "@/kit";
import { t as tr, useT } from "@/i18n";
import { useFolderImports, type FolderJob } from "./importFolder";
import { formatBytes, skippedText } from "./plan";

/** The job's folder: “Drums”, or "the folder" before its name is known. */
const folderName = (job: FolderJob) => (job.name ? tr("browser.import.quoted", { name: job.name }) : tr("browser.import.the_folder"));

function text(job: FolderJob, copies: boolean): string {
  const name = folderName(job);
  const extra = [job.skipped > 0 && skippedText(job.skipped), job.failed > 0 && tr("browser.import.failed_count", { count: job.failed })]
    .filter(Boolean)
    .join(" · ");
  const tail = extra ? ` · ${extra}` : "";
  switch (job.state) {
    case "reading":
      return tr("browser.import.reading", { name });
    case "copying":
      return tr("browser.import.copying", { name, done: job.done, total: job.total, sent: formatBytes(job.sent), bytes: formatBytes(job.bytes) }) + tail;
    case "done":
      return (copies ? tr("browser.import.done_copied", { name, count: job.done }) : tr("browser.import.done", { name, count: job.done })) + tail;
    case "cancelled":
      return tr("browser.import.cancelled", { name, done: job.done, total: job.total }) + tail;
    case "error":
      return job.error ?? tr("browser.import.failed");
  }
}

/**
 * The browser's folder-import rows (`base-136`): progress with Cancel while a folder is read
 * or copied, then the outcome (skipped files, errors) until dismissed.
 */
export function FolderImportStatus({ copies }: { copies: boolean }) {
  const t = useT();
  const jobs = useFolderImports((s) => s.jobs);
  const { cancel, dismiss } = useFolderImports.getState();
  if (jobs.length === 0) return null;
  return (
    <ul className="eth-folder-import" aria-label={t("browser.import.list")}>
      {jobs.map((job) => {
        const running = job.state === "reading" || job.state === "copying";
        const progress = job.bytes > 0 ? Math.min(1, job.sent / job.bytes) : 0;
        return (
          <li
            key={job.id}
            className={clsx("eth-folder-import__row", job.state === "error" && "eth-folder-import__row--error")}
            data-state={job.state}
            role={job.state === "error" ? "alert" : "status"}
          >
            <span className="eth-folder-import__text" title={text(job, copies)}>
              {text(job, copies)}
            </span>
            {running && (
              <span
                className={clsx("eth-folder-import__bar", job.state === "reading" && "eth-folder-import__bar--busy")}
                role="progressbar"
                aria-label={t("browser.import.progress", { name: job.name ?? t("browser.import.the_folder") })}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(progress * 100)}
                style={{ "--eth-folder-import-progress": progress } as CSSProperties}
              >
                <span className="eth-folder-import__fill" />
              </span>
            )}
            {running ? (
              <IconButton
                size="sm"
                tone="ghost"
                label={t("browser.import.cancel", { name: job.name ?? t("browser.import.the_folder") })}
                icon={<X />}
                onClick={() => cancel(job.id)}
              />
            ) : (
              <IconButton size="sm" tone="ghost" label={t("common.dismiss")} icon={<X />} onClick={() => dismiss(job.id)} />
            )}
          </li>
        );
      })}
    </ul>
  );
}
