import clsx from "clsx";
import { useMemo, useState, type CSSProperties } from "react";
import { ChevronDown } from "lucide-react";
import type { Device, DeviceDescriptor, DeviceLayout, LayoutItem, LayoutSection } from "@/generated";
import type { GestureSender } from "../gesture";
import { LayoutContext, useLayoutContextValue } from "./context";
import { isGenericSection, resolveLayout } from "./model";
import { WidgetView } from "./Widget";
import "./layout.css";

export interface DeviceLayoutViewProps {
  device: Device;
  descriptor: DeviceDescriptor;
  sender: GestureSender;
}

/**
 * The one shared device renderer (CONTRACTS.md §12.4.2): the device's declared layout, or
 * the generic one (params grouped by `ParamInfo.group`); params the layout doesn't show
 * fold under "More controls".
 */
export function DeviceLayoutView({ device, descriptor, sender }: DeviceLayoutViewProps) {
  const ctx = useLayoutContextValue(device, descriptor, sender);
  const resolved = useMemo(() => resolveLayout(descriptor), [descriptor]);
  const [expanded, setExpanded] = useState(false);
  return (
    <LayoutContext.Provider value={ctx}>
      <Sections layout={resolved.main} className={clsx(resolved.declared && "eth-layout--declared")} />
      {resolved.more && (
        <>
          <button
            type="button"
            className="eth-device__more"
            aria-expanded={expanded}
            aria-label={`${expanded ? "Fewer" : "More"} ${device.name} controls`}
            onClick={() => setExpanded((x) => !x)}
          >
            <ChevronDown className={expanded ? "eth-device__more-icon eth-device__more-icon--open" : "eth-device__more-icon"} />
            {expanded ? "Fewer controls" : `More controls (${resolved.moreCount})`}
          </button>
          {expanded && <Sections layout={resolved.more} className="eth-device__body--more" />}
        </>
      )}
    </LayoutContext.Provider>
  );
}

function Sections({ layout, className }: { layout: DeviceLayout; className?: string | undefined }) {
  return (
    <div className={clsx("eth-device__body", "eth-layout", className)}>
      {layout.sections.map((s) => (
        <Section key={s.id} section={s} />
      ))}
    </div>
  );
}

/** Size class of a section's grid: its largest widget (generic: lg main, md folded). */
function gridSize(s: LayoutSection): "lg" | "md" {
  return s.items.some((i) => i.size === "Large") ? "lg" : "md";
}

function Section({ section: s }: { section: LayoutSection }) {
  const generic = isGenericSection(s);
  const style = generic ? undefined : ({ "--section-span": s.span, "--section-columns": s.columns } as CSSProperties);
  return (
    <div
      className={clsx("eth-device__group", "eth-layout__section", generic ? "eth-layout__section--generic" : `eth-layout__section--span-${s.span}`)}
      data-section={s.id}
      style={style}
    >
      {s.title && <div className="eth-device__group-name">{s.title}</div>}
      <div className={clsx("eth-device__params", `eth-device__params--${gridSize(s)}`, !generic && "eth-layout__grid")}>
        {s.items.map((item, i) => (
          <Item key={i} item={item} columns={s.columns} />
        ))}
      </div>
    </div>
  );
}

function Item({ item, columns }: { item: LayoutItem; columns: number }) {
  const span = Math.max(1, Math.min(item.colspan, columns));
  const widget = <WidgetView widget={item.widget} size={item.size} label={item.label} />;
  if (span === 1) return widget;
  return (
    <div
      className={clsx("eth-layout__item", span >= columns && "eth-layout__item--full")}
      style={{ "--item-span": span } as CSSProperties}
      data-colspan={span}
    >
      {widget}
    </div>
  );
}
