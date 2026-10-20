/**
 * A device card's inline parameter list (the header's "Parameters" toggle): the
 * generic UI for every device, factory or plugin (name + value control per visible param).
 * The area is capped at `--eth-size-device-params-max-height` and scrolls inside, so a
 * plugin with hundreds of params doesn't take over the chain; only the rows in view are
 * mounted. A search field shows for longer lists. Pinned params (per plugin, `pins.ts`)
 * come first.
 */
import { useDeferredValue, useMemo, useState } from "react";
import type { DeviceDescriptor, DeviceId, ParamInfo } from "@/generated";
import { TextInput, VirtualList } from "@/kit";
import { size } from "@/theme/tokens";
import type { GestureSender } from "../gesture";
import { ParamRow } from "./AllParams";
import { filterParams } from "./filter";
import { usePins } from "./pins";
import "./params.css";
import { useT } from "@/i18n";

const ROW_PX = parseFloat(size.paramListRowHeight);
/** Lists longer than this get a search field. */
export const PARAMS_SEARCH_FROM = 8;

export interface DeviceParamsProps {
  device: DeviceId;
  deviceName: string;
  descriptor: DeviceDescriptor;
  sender: GestureSender;
  /** Pin key of the device (`pinKey`): rows get a pin toggle, pinned params come first. */
  pins: string;
  /** Id of the area (the header toggle's `aria-controls`). */
  id?: string;
}

/** `params` with the pinned ones first, in pin order. */
function pinnedFirst(params: ParamInfo[], pinned: ReadonlyArray<number>): ParamInfo[] {
  if (pinned.length === 0) return params;
  const set = new Set(pinned);
  const byId = new Map(params.filter((p) => set.has(p.id)).map((p) => [p.id, p]));
  const first = pinned.flatMap((id) => byId.get(id) ?? []);
  return [...first, ...params.filter((p) => !set.has(p.id))];
}

export function DeviceParams({ device, deviceName, descriptor, sender, pins, id }: DeviceParamsProps) {
  const t = useT();
  const [query, setQuery] = useState("");
  const deferred = useDeferredValue(query);
  const pinned = usePins(pins);
  const pinnedSet = useMemo(() => new Set(pinned), [pinned]);
  const visible = useMemo(() => filterParams(descriptor.params, ""), [descriptor.params]);
  const list = useMemo(
    () => pinnedFirst(deferred ? filterParams(descriptor.params, deferred) : visible, pinned),
    [descriptor.params, deferred, visible, pinned],
  );
  return (
    <div className="eth-params eth-device-params" id={id} data-testid="device-params">
      {visible.length > PARAMS_SEARCH_FROM && (
        <div className="eth-params__bar">
          <TextInput
            size="sm"
            type="search"
            className="eth-params__search"
            placeholder={t("devices.params.search_placeholder")}
            aria-label={t("devices.params.search", { name: deviceName })}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <span className="eth-params__count" aria-live="polite">
            {list.length === visible.length ? `${visible.length}` : t("devices.params.count_filtered", { shown: list.length, total: visible.length })}
          </span>
        </div>
      )}
      <VirtualList
        key={deferred}
        className="eth-params__list eth-device-params__list"
        role="list"
        aria-label={t("devices.params.list", { name: deviceName })}
        count={list.length}
        rowHeight={ROW_PX}
        rowKey={(i) => list[i]!.id}
        renderRow={(i) => <ParamRow device={device} info={list[i]!} sender={sender} pinKey={pins} pinned={pinnedSet.has(list[i]!.id)} />}
        empty={<div className="eth-params__empty">{visible.length === 0 ? t("devices.params.none") : t("devices.params.no_match", { query })}</div>}
      />
    </div>
  );
}
