/** Mock of `Device::SetSidechain`. Owned by `sidechain`. */

import type { TrackId } from "@/generated";
import { fail, type ReducerContext } from "../documentReducer";

export function setSidechain(ctx: ReducerContext, deviceId: string, source: TrackId | null): void {
  const d = ctx.tx.get("Device", deviceId) ?? fail("NotFound", `device ${deviceId}`);
  if (source !== null) {
    if (!ctx.tx.get("Track", source)) fail("NotFound", `track ${source}`);
    if (source === d.track) fail("InvalidArgument", "a device cannot sidechain its own track");
  }
  ctx.tx.upsert("Device", { ...d, sidechain: source });
}
