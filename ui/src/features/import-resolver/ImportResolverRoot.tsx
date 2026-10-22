import { useContext, useEffect } from "react";
import { TransportContext, type EngineTransport } from "@/transport";
import { skipNotifications, type NotificationEvent } from "@/features/notifications/store";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { useProjectStore } from "@/state";
import { catalogNow, isMissingPlugin } from "./catalog";
import { ResolverDialog } from "./ResolverDialog";
import { setReplaceTransport } from "./replace";
import { applyResolveEvent, armAutoShow, autoShow, disarmAutoShow, noteMissing, proposeResolution, setResolverTransport, useResolver } from "./store";

/**
 * "could not create device "X"" for a plugin that isn't installed (or a device of another
 * DAW): opening a project with many would toast once per device. The status bar counts them
 * and one toast says some are missing (`openIfMissing`); other failures still toast.
 */
function isMissingDeviceFailure(event: NotificationEvent, transport: EngineTransport): boolean {
  if (event.key?.id !== "notice.engine.device_failed") return false;
  const name = event.key.args.device;
  const project = useProjectStore.getState().project;
  if (typeof name !== "string" || !project) return false;
  const catalog = catalogNow(transport);
  return Object.values(project.devices).some(
    (d) => d.name === name && (d.kind.type === "Missing" || (d.kind.type === "Plugin" && (catalog === null || isMissingPlugin(d, catalog)))),
  );
}

function Sync({ transport }: { transport: EngineTransport }) {
  useEffect(() => skipNotifications((e) => isMissingDeviceFailure(e, transport)), [transport]);
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
    // The engine host's plugin list came (or changed) after the project opened: what is
    // missing is its answer, not this computer's (the dialog or count is brought up to date).
    const offPlugins = usePluginStore.subscribe((s, prev) => {
      if (s.remote !== prev.remote) noteMissing(transport);
    });
    return () => {
      off();
      offPlugins();
      setResolverTransport(null);
      setReplaceTransport(null);
    };
  }, [transport]);
  return null;
}

/**
 * Mounted once by the app shell: the "Resolve missing items" dialog and its engine events
 * ("Replace device…" is a popover on the device card: `ReplacePopover`).
 * Renders nothing without an engine connection.
 */
export function ImportResolverRoot() {
  const transport = useContext(TransportContext)?.transport;
  if (!transport) return null;
  return (
    <>
      <Sync transport={transport} />
      <ResolverDialog transport={transport} />
    </>
  );
}
