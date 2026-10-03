/** The factory IR list of the Convolution Reverb (`Device::ListFactoryIrs`). */

import { useEffect, useState } from "react";
import type { FactoryIr } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

let factoryCache: Promise<FactoryIr[]> | null = null;

/** The factory IR list (`Device::ListFactoryIrs`), fetched once per session. */
export function useFactoryIrs(transport: EngineTransport): ReadonlyArray<FactoryIr> {
  const [irs, setIrs] = useState<ReadonlyArray<FactoryIr>>([]);
  useEffect(() => {
    let live = true;
    factoryCache ??= transport
      .send(cmd("Device", { type: "ListFactoryIrs" }))
      .then((r) => (r.type === "FactoryIrs" ? r.irs : []))
      .catch(() => {
        factoryCache = null;
        return [];
      });
    void factoryCache.then((list) => {
      if (live) setIrs(list);
    });
    return () => {
      live = false;
    };
  }, [transport]);
  return irs;
}

/** Test hook: forget the cached factory list. */
export function resetFactoryIrCache(): void {
  factoryCache = null;
}
