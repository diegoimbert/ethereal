// The app-wide part of MIDI learn, mounted once by the shell (not by the MIDI tab): it mirrors
// the engine's learn events into the store, connects the hardware MIDI ports, and installs
// MIDI mode on the document. Learning therefore works wherever it is started (a control
// clicked in MIDI mode, its context menu) with the MIDI tab closed.
import { useContext, useEffect } from "react";
import { useProjectStore } from "@/state";
import { cmd, TransportContext } from "@/transport";
import { canLearn } from "./host";
import { useMidiLearnStore } from "./store";
import { useMidiMode } from "./useMidiMode";

export function MidiLearnRoot() {
  // Tolerate a missing `<TransportProvider>` (bare App shell in tests).
  const transport = useContext(TransportContext)?.transport ?? null;
  const hasProject = useProjectStore((s) => !!s.project);
  const enabled = useMidiLearnStore((s) => s.enabled);
  const onEvent = useMidiLearnStore((s) => s.onEvent);
  const supported = !!transport && canLearn(transport);

  useEffect(() => transport?.onEvent(onEvent), [transport, onEvent]);
  useMidiMode(transport, enabled && supported && hasProject);

  // Connect the hardware ports (the host opens MIDI inputs when they are listed).
  useEffect(() => {
    if (transport && supported) void transport.send(cmd("Recording", { type: "ListInputs" })).catch(() => undefined);
  }, [transport, supported]);

  return null;
}
