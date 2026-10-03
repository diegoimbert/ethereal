/**
 * "Show all N parameters…": a searchable, windowed list of every visible param of a device
 * (thousands for some plugins). Only the rows in view are mounted, and each row subscribes
 * to its own param value, so value changes of the other params re-render nothing. Each row
 * has a pin toggle (pins pick what the capped card shows, see `pins.ts`).
 */

import clsx from "clsx";
import { memo, useDeferredValue, useMemo, useState } from "react";
import { Pin } from "lucide-react";
import type { DeviceId, ParamId, ParamInfo } from "@/generated";
import { Dialog, IconButton, TextInput, VirtualList } from "@/kit";
import { useProjectStore } from "@/state";
import { size } from "@/theme/tokens";
import type { GestureSender } from "../gesture";
import { ParamControl } from "../ParamControl";
import { setPinned, usePins } from "./pins";
import "./params.css";

const ROW_PX = parseFloat(size.paramListRowHeight);

/** Visible params whose name or group contains `query` (case-insensitive). */
function filterParams(params: ReadonlyArray<ParamInfo>, query: string): ParamInfo[] {
  const q = query.trim().toLowerCase();
  return params.filter((p) => !p.hidden && (!q || p.name.toLowerCase().includes(q) || (p.group?.toLowerCase().includes(q) ?? false)));
}

export interface AllParamsProps {
  device: DeviceId;
  deviceName: string;
  params: ReadonlyArray<ParamInfo>;
  sender: GestureSender;
  /** Pin key of the device (`pinKey`), or `null` for no pin toggles. */
  pins: string | null;
  /** Visible params (the button label). */
  total: number;
}

/** The "Show all N parameters…" button and its dialog. */
export function AllParamsButton(props: AllParamsProps) {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button
        type="button"
        className="eth-device__more eth-device__all"
        aria-haspopup="dialog"
        aria-label={`Show all ${props.total} ${props.deviceName} parameters`}
        onClick={() => setOpen(true)}
      >
        {`Show all ${props.total} parameters…`}
      </button>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        title={`${props.deviceName}: all parameters`}
        className="eth-params-dialog"
      >
        {open && <ParamBrowser {...props} />}
      </Dialog>
    </>
  );
}

/** Search field + windowed list of a device's params (each row: pin, name, control). */
export function ParamBrowser({ device, deviceName, params, sender, pins, total }: AllParamsProps) {
  const [query, setQuery] = useState("");
  // Typing stays responsive while 10k names are filtered.
  const deferred = useDeferredValue(query);
  const list = useMemo(() => filterParams(params, deferred), [params, deferred]);
  const pinned = usePins(pins ?? "");
  const pinnedSet = useMemo(() => new Set(pins ? pinned : []), [pins, pinned]);
  return (
    <div className="eth-params">
      <div className="eth-params__bar">
        <TextInput
          size="sm"
          type="search"
          className="eth-params__search"
          placeholder="Search parameters…"
          aria-label={`Search ${deviceName} parameters`}
          value={query}
          autoFocus
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="eth-params__count" aria-live="polite">
          {list.length === total ? `${total}` : `${list.length} of ${total}`}
        </span>
      </div>
      <VirtualList
        // A new search starts at the top.
        key={deferred}
        className="eth-params__list"
        role="list"
        aria-label={`${deviceName} parameters`}
        count={list.length}
        rowHeight={ROW_PX}
        rowKey={(i) => list[i]!.id}
        renderRow={(i) => {
          const p = list[i]!;
          return (
            <ParamRow
              device={device}
              info={p}
              sender={sender}
              pinKey={pins}
              pinned={pinnedSet.has(p.id)}
            />
          );
        }}
        empty={<div className="eth-params__empty">No parameter matches “{query}”.</div>}
      />
    </div>
  );
}

interface ParamRowProps {
  device: DeviceId;
  info: ParamInfo;
  sender: GestureSender;
  pinKey: string | null;
  pinned: boolean;
}

/** One param row; subscribes to that param's value only. */
const ParamRow = memo(function ParamRow({ device, info, sender, pinKey, pinned }: ParamRowProps) {
  const plain = useParamValue(device, info.id) ?? info.default;
  return (
    <div className={clsx("eth-params__row", pinned && "eth-params__row--pinned")} role="listitem" data-param-row={info.id}>
      {pinKey !== null && (
        <IconButton
          size="sm"
          tone="ghost"
          className="eth-params__pin"
          label={`${pinned ? "Unpin" : "Pin"} ${info.name}`}
          active={pinned}
          icon={<Pin />}
          onClick={() => setPinned(pinKey, info.id, !pinned)}
        />
      )}
      <span className="eth-params__name" title={info.name}>
        <span className="eth-params__label">{info.name}</span>
        {info.group && <span className="eth-params__group">{info.group}</span>}
      </span>
      <span className="eth-params__control">
        <ParamControl device={device} plain={plain} info={info} sender={sender} size="sm" />
      </span>
    </div>
  );
});

/** The document value of one param (re-renders only when that value changes). */
function useParamValue(device: DeviceId, param: ParamId): number | undefined {
  return useProjectStore((s) => s.project?.devices[device]?.params[param]);
}
