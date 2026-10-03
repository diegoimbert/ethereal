import "./folders.css";
import clsx from "clsx";
import type { CSSProperties } from "react";
import { X } from "lucide-react";
import { IconButton } from "@/kit";
import { useFolderImports, type FolderJob } from "./importFolder";
import { formatBytes, skippedText } from "./plan";

const files = (n: number) => (n === 1 ? "1 file" : `${n} files`);

function text(job: FolderJob, copies: boolean): string {
  const name = job.name ? `“${job.name}”` : "folder";
  const extra = [job.skipped > 0 && skippedText(job.skipped), job.failed > 0 && `${job.failed} failed`].filter(Boolean).join(" · ");
  const tail = extra ? ` · ${extra}` : "";
  switch (job.state) {
    case "reading":
      return `Reading ${name}…`;
    case "copying":
      return `Importing ${name}: ${job.done} of ${files(job.total)} (${formatBytes(job.sent)} of ${formatBytes(job.bytes)})${tail}`;
    case "done":
      return `Imported ${files(job.done)} into ${name}${copies ? " (copied into this browser’s storage)" : ""}${tail}`;
    case "cancelled":
      return `Import of ${name} cancelled: ${job.done} of ${files(job.total)} copied${tail}`;
    case "error":
      return job.error ?? "Import failed";
  }
}

/**
 * The browser's folder-import rows (`base-136`): progress with Cancel while a folder is read
 * or copied, then the outcome (skipped files, errors) until dismissed.
 */
export function FolderImportStatus({ copies }: { copies: boolean }) {
  const jobs = useFolderImports((s) => s.jobs);
  const { cancel, dismiss } = useFolderImports.getState();
  if (jobs.length === 0) return null;
  return (
    <ul className="eth-folder-import" aria-label="Folder imports">
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
                aria-label={`Importing ${job.name ?? "folder"}`}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={Math.round(progress * 100)}
                style={{ "--eth-folder-import-progress": progress } as CSSProperties}
              >
                <span className="eth-folder-import__fill" />
              </span>
            )}
            {running ? (
              <IconButton size="sm" tone="ghost" label={`Cancel importing ${job.name ?? "folder"}`} icon={<X />} onClick={() => cancel(job.id)} />
            ) : (
              <IconButton size="sm" tone="ghost" label="Dismiss" icon={<X />} onClick={() => dismiss(job.id)} />
            )}
          </li>
        );
      })}
    </ul>
  );
}
