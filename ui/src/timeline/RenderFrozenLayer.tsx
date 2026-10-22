import { useLayoutEffect, useRef } from "react";
import type { RenderingState } from "@/generated";
import { useProjectStore } from "@/state/projectStore";
import { useT } from "@/i18n";
import { identityMapping, type PlayheadMapping } from "./playhead";
import { beatsToPx } from "./viewport";
import type { TimelineViewStore } from "./viewStore";
import "./timeline.css";

export interface RenderFrozenLayerProps {
  view: TimelineViewStore;
  /** Maps song beats to the view's axis (clip-relative views). Default identity. */
  mapping?: PlayheadMapping;
  /** Show the "not rendered yet" label at the boundary (the ruler's, not every lane's). */
  label?: boolean;
}

/**
 * While a render runs (`TransportState.rendering`, CONTRACTS.md §13.20), what isn't rendered
 * yet is frozen: a hatched shade over the timeline past the rendered part (`until`, the
 * boundary moving as the render advances) and before the render's start. Playback (the render
 * preview) only plays the part between. Pointer-transparent: editing goes on underneath.
 * Place it in a `position: relative` container spanning the view; positioned outside React
 * on pan and zoom (the boundary re-renders at the render's report rate).
 */
export function RenderFrozenLayer({ view, mapping = identityMapping, label = false }: RenderFrozenLayerProps) {
  const r = useProjectStore((s) => s.transport?.rendering ?? null);
  if (!r) return null;
  return <Frozen view={view} mapping={mapping} label={label} r={r} />;
}

function Frozen({ view, mapping, label, r }: Required<RenderFrozenLayerProps> & { r: RenderingState }) {
  const t = useT();
  const before = useRef<HTMLDivElement>(null);
  const after = useRef<HTMLDivElement>(null);
  const mapRef = useRef(mapping);
  useLayoutEffect(() => {
    mapRef.current = mapping;
  });
  const { start, until } = r;
  useLayoutEffect(() => {
    const apply = () => {
      const s = view.getState();
      const width = s.widthPx > 0 ? s.widthPx : Number.MAX_SAFE_INTEGER;
      const px = (b: number) => {
        const m = mapRef.current(b);
        return m === null ? null : beatsToPx(m, s);
      };
      const b = before.current;
      if (b) {
        const x = px(start);
        const w = x === null ? 0 : Math.max(0, Math.min(x, width));
        b.style.width = `${w}px`;
        b.hidden = w <= 0;
      }
      const a = after.current;
      if (a) {
        const x = px(until);
        a.style.left = `${x === null ? 0 : Math.max(0, x)}px`;
        a.hidden = x !== null && x > width;
        // The boundary's line and label only where it is in view.
        a.toggleAttribute("data-edge", x !== null && x >= 0);
      }
    };
    apply();
    return view.subscribe(apply);
  }, [view, start, until]);
  return (
    <div className="eth-render-frozen" data-testid="render-frozen" aria-hidden>
      <div ref={before} className="eth-render-frozen__shade eth-render-frozen__shade--before" data-testid="render-frozen-before" />
      <div ref={after} className="eth-render-frozen__shade eth-render-frozen__shade--after" data-testid="render-frozen-after" data-until={until}>
        {label && <span className="eth-render-frozen__label">{t("arrangement.render.not_rendered")}</span>}
      </div>
    </div>
  );
}
