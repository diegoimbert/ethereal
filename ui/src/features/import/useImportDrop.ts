/** OS-file drop target that only adds the files to the project (the sample browser). */
import { useContext, type DragEvent } from "react";
import { TransportContext } from "@/transport";
import { importAudio, type ImportOutcome } from "./importAudio";
import { droppedFiles, hasOsFiles, noteHover } from "./osDrop";
import type { ImportSource } from "./sources";

export interface ImportDropProps {
  onDragOver?: (e: DragEvent<HTMLElement>) => void;
  onDrop?: (e: DragEvent<HTMLElement>) => void;
}

const NONE: ImportDropProps = {};

/**
 * Drop handlers (spread them on the drop zone): dropped audio files are imported into the
 * open project (uploaded, or by path on the desktop). `onImport` runs when a drop starts.
 */
export function useImportDrop(onImport?: (done: Promise<ImportOutcome[]>) => void): ImportDropProps {
  const transport = useContext(TransportContext)?.transport;
  if (!transport) return NONE;
  const run = (sources: ImportSource[]) => {
    if (sources.length === 0) return;
    const done = importAudio(transport, sources);
    onImport?.(done);
  };
  return {
    onDragOver: (e) => {
      if (!hasOsFiles(e.dataTransfer)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
      noteHover(e, (sources) => run(sources));
    },
    onDrop: (e) => {
      if (!hasOsFiles(e.dataTransfer)) return;
      e.preventDefault();
      run(droppedFiles(e.dataTransfer));
    },
  };
}
