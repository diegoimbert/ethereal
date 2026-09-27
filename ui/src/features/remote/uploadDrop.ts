/** OS-file drop target that uploads to a remote engine (used by the sample browser). */
import { useContext, type DragEvent } from "react";
import { TransportContext } from "@/transport";
import { uploadFiles } from "./upload";

export interface UploadDropProps {
  onDragOver?: (e: DragEvent<HTMLElement>) => void;
  onDrop?: (e: DragEvent<HTMLElement>) => void;
}

const NONE: UploadDropProps = {};

/**
 * Drop handlers for OS files (spread on a drop target, e.g. the sample browser): when the
 * UI talks to a remote engine, dropped files are uploaded and imported into the project.
 * Returns no handlers otherwise (a local engine reads files itself, from its library).
 */
export function useUploadDrop(): UploadDropProps {
  const transport = useContext(TransportContext)?.transport;
  if (!transport || transport.kind !== "remote") return NONE;
  return {
    onDragOver: (e) => {
      if (!e.dataTransfer.types.includes("Files")) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    },
    onDrop: (e) => {
      const files = [...e.dataTransfer.files];
      if (files.length === 0) return;
      e.preventDefault();
      void uploadFiles(transport, files);
    },
  };
}
