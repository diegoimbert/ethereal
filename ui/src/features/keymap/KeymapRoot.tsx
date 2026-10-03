import { useEffect } from "react";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { CheatSheetPrinter } from "./CheatSheet";
import { installDispatcher } from "./dispatcher";
import { KeymapEditor } from "./KeymapEditor";
import { loadKeymap, useKeymapStore } from "./store";
import "./keymap.css";

/**
 * Mounted once by the app shell: loads the user's keymap from the engine (and follows
 * `KeymapEvent::Changed` from other windows / remote UIs), runs the palette-command
 * dispatcher, and hosts the editor dialog and the cheat sheet printer.
 */
export function KeymapRoot() {
  const transport = useOptionalTransport();
  useEffect(() => {
    if (!transport) return;
    void loadKeymap(transport);
    return transport.onEvent((e) => {
      if (e.type === "Keymap" && e.event.type === "Changed") useKeymapStore.setState({ keymap: e.event.keymap, stored: true });
    });
  }, [transport]);
  useEffect(() => installDispatcher(), []);
  return (
    <>
      <KeymapEditor />
      <CheatSheetPrinter />
    </>
  );
}
