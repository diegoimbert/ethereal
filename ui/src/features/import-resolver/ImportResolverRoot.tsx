import { useContext, useEffect } from "react";
import { TransportContext, type EngineTransport } from "@/transport";
import { ReplaceDialog } from "./ReplaceDialog";
import { ResolverDialog } from "./ResolverDialog";
import { setReplaceTransport } from "./replace";
import { applyResolveEvent, armAutoShow, autoShow, disarmAutoShow, noteMissing, proposeResolution, setResolverTransport, useResolver } from "./store";

function Sync({ transport }: { transport: EngineTransport }) {
  useEffect(() => {
    setResolverTransport(transport);
    setReplaceTransport(transport);
    useResolver.setState({ autoShow: autoShow() });
    // An import opens its project before its report: the report comes first, then (when
    // it closes) the missing items (`ProjectImportDialogs`).
    let importing = false;
    const off = transport.onEvent((e) => {
      applyResolveEvent(e);
      if (e.type === "Import") {
        importing = e.event.type === "Progress";
        if (importing) disarmAutoShow();
        // After an import: count what is missing (the report's "Resolve…" button).
        if (e.event.type === "Done" && !useResolver.getState().open) {
          setTimeout(() => void proposeResolution(transport), 0);
        }
      }
      // Owner request: opening a project lists everything missing (plugins, devices, audio
      // files) in one dialog, with "Resolve later". Missing audio files are found as the
      // engine loads them, just after.
      if (e.type === "ProjectLoaded" && !importing) armAutoShow(transport);
      if (e.type === "Media" && e.event.type === "Missing") noteMissing(transport);
    });
    return () => {
      off();
      setResolverTransport(null);
      setReplaceTransport(null);
    };
  }, [transport]);
  return null;
}

/**
 * Mounted once by the app shell: the "Resolve missing items" and "Replace plugin…" dialogs
 * and their engine events.
 * Renders nothing without an engine connection.
 */
export function ImportResolverRoot() {
  const transport = useContext(TransportContext)?.transport;
  if (!transport) return null;
  return (
    <>
      <Sync transport={transport} />
      <ResolverDialog transport={transport} />
      <ReplaceDialog transport={transport} />
    </>
  );
}
