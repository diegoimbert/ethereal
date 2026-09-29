/** Modulator kinds (`Modulation::ListModulatorKinds`, cached per transport) and menus. */

import { useEffect, useState } from "react";
import type { Command, Device, ModulatorDescriptor } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import { cmd, newId, useTransport, type EngineTransport } from "@/transport";

const kindsCache = new WeakMap<EngineTransport, Promise<ModulatorDescriptor[]>>();

function fetchKinds(transport: EngineTransport): Promise<ModulatorDescriptor[]> {
  let p = kindsCache.get(transport);
  if (!p) {
    p = transport.send(cmd("Modulation", { type: "ListModulatorKinds" })).then((r) => {
      if (r.type !== "ModulatorKinds") throw new Error(`unexpected reply ${r.type}`);
      return r.kinds;
    });
    p.catch(() => kindsCache.delete(transport));
    kindsCache.set(transport, p);
  }
  return p;
}

/** Modulator kind descriptors (`Modulation::ListModulatorKinds`, cached per transport). */
export function useModulatorKinds(): ModulatorDescriptor[] {
  const transport = useTransport();
  const [kinds, setKinds] = useState<ModulatorDescriptor[]>([]);
  useEffect(() => {
    let live = true;
    fetchKinds(transport).then(
      (k) => live && setKinds(k),
      () => undefined,
    );
    return () => {
      live = false;
    };
  }, [transport]);
  return kinds;
}

/** Context-menu entries adding a modulator of each kind to `device`. */
export function addModulatorEntries(kinds: ReadonlyArray<ModulatorDescriptor>, device: Device, send: (c: Command) => unknown): ContextMenuEntry[] {
  return kinds.map((k) => ({
    label: `Add ${k.name}`,
    onSelect: () => void send(cmd("Modulation", { type: "AddModulator", id: newId(), device: device.id, kind: k.kind, name: null })),
  }));
}

