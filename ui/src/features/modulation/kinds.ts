/** Modulator kinds (`Modulation::ListModulatorKinds`, cached per transport) and menus. */

import { useEffect, useMemo, useState } from "react";
import type { Command, Device, ModulatorDescriptor, ModulatorKind } from "@/generated";
import { localizeModulator, t, useLocale, type MessageKey } from "@/i18n";
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

/** Modulator kind descriptors (`Modulation::ListModulatorKinds`, cached per transport), in the current language. */
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
  const locale = useLocale();
  // eslint-disable-next-line react-hooks/exhaustive-deps -- `locale` only invalidates
  return useMemo(() => kinds.map(localizeModulator), [kinds, locale]);
}

/** "add …" of each modulator kind (literal keys; an unknown kind shows its engine name). */
const ADD_KIND: Record<ModulatorKind, MessageKey> = {
  Lfo: "modulation.add.lfo",
  Envelope: "modulation.add.envelope",
  EnvelopeFollower: "modulation.add.follower",
  Steps: "modulation.add.steps",
  Random: "modulation.add.random",
  Keytrack: "modulation.add.keytrack",
  Velocity: "modulation.add.velocity",
};

/** Context-menu entries adding a modulator of each kind to `device`. */
export function addModulatorEntries(kinds: ReadonlyArray<ModulatorDescriptor>, device: Device, send: (c: Command) => unknown): ContextMenuEntry[] {
  return kinds.map((k) => ({
    label: k.kind in ADD_KIND ? t(ADD_KIND[k.kind]) : t("modulation.add.other", { name: k.name }),
    onSelect: () => void send(cmd("Modulation", { type: "AddModulator", id: newId(), device: device.id, kind: k.kind, name: null })),
  }));
}
