import { isEtherFileName } from "@/features/project/actions";
import { contributions } from "@/ext";
import "./import.css";
import { useContext, useEffect } from "react";
import { TransportContext, type EngineTransport } from "@/transport";
import { importMidiFile, isMidiName } from "@/features/midi-file/importMidi";
import { importAudio } from "./importAudio";
import { deliverPathDrop, isPathDropHost } from "./osDrop";
import { isImportShortcut, openImportDialog } from "./picker";
import { pathSource } from "./sources";

function isTextEntry(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName);
}

function useImportCommands(transport: EngineTransport) {
  // ⌘I / Ctrl+I anywhere but in text fields.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !isImportShortcut(e) || isTextEntry(e.target)) return;
      e.preventDefault();
      void openImportDialog(transport);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [transport]);

  // Desktop: OS paths dropped on the window land in the zone hovered last; anywhere else
  // they are only added to the project.
  useEffect(() => {
    if (!isPathDropHost(transport)) return;
    return transport.onPathDrop((all) => {
      // An Ethereal project file is opened, not imported as audio, and so are the project
      // files an extension imports (`@/ext` "import.isProjectFile").
      const projectFile = (p: string) => isEtherFileName(p) || contributions("import.isProjectFile").some((is) => is(p));
      const paths = all.filter((p) => !projectFile(p));
      if (paths.length === 0) return;
      if (deliverPathDrop(paths)) return;
      // Nowhere in particular: MIDI files become new MIDI tracks at the start, audio
      // files are added to the project.
      const midi = paths.filter(isMidiName);
      for (const p of midi) importMidiFile(transport, pathSource(p), { track: null, at: 0 }).catch((e: unknown) => console.warn("MIDI file import failed", e));
      const audio = paths.filter((p) => !isMidiName(p));
      if (audio.length > 0) void importAudio(transport, audio.map(pathSource));
    });
  }, [transport]);
}

function Commands({ transport }: { transport: EngineTransport }) {
  useImportCommands(transport);
  return null;
}

/**
 * Mounted once by the app shell: the ⌘I command and desktop path drops (the status bar shows
 * the imports, `ImportStatus`). Renders nothing without an engine connection.
 */
export function ImportRoot() {
  const transport = useContext(TransportContext)?.transport;
  return transport ? <Commands transport={transport} /> : null;
}
