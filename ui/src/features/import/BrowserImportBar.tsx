import "./import.css";
import { FilePlus2 } from "lucide-react";
import { Button } from "@/kit";
import type { EngineTransport } from "@/transport";
import { importAudio, type ImportOutcome } from "./importAudio";
import { pickSources } from "./picker";

export interface BrowserImportBarProps {
  transport: EngineTransport;
  /** Runs when files were picked (the browser then shows the project's media). */
  onImport?: (done: Promise<ImportOutcome[]>) => void;
}

/**
 * Footer of the sample browser: "Import audio…" adds the picked files to the project (no
 * clips; ⌘I places them in the arrangement), and the drop hint.
 */
export function BrowserImportBar({ transport, onImport }: BrowserImportBarProps) {
  const pick = async () => {
    const sources = await pickSources(transport);
    if (sources.length > 0) onImport?.(importAudio(transport, sources));
  };
  return (
    <div className="eth-import-bar">
      <Button size="sm" tone="ghost" title="Add audio files to the project" onClick={() => void pick()}>
        <FilePlus2 aria-hidden />
        Import audio…
      </Button>
      <span className="eth-import-bar__hint">or drop files here</span>
    </div>
  );
}
