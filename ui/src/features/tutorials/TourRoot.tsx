// The tour overlay: dims the screen except the current step's highlighted parts (holes with
// a ring, animated from step to step, following them as they move), shows the bubble beside
// them and guards input (`guard.ts`). The app shell mounts it once.
import { useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { Button, IconButton } from "@/kit";
import { pauseDispatch } from "@/features/keymap";
import { useT } from "@/i18n";
import { dimPath, holes, placeBubble, rectOf, sameRects, union, type Rect, type Side } from "./geometry";
import { installGuard } from "./guard";
import { askExit, backStep, currentStep, exitTour, finishTour, nextStep, nudgeTour, tickTour, useTour, visibleSteps } from "./store";
import { findTargets } from "./targets";
import { textOf } from "./types";
import "./tutorials.css";

/** Open floating layers (menus, select lists, popovers), which the bubble must not cover. */
const PICKERS = '.eth-popover:not(.eth-popover--closing), [role="listbox"]';

interface Layout {
  viewport: { w: number; h: number };
  rects: Rect[];
  bubble: { x: number; y: number; side: Side } | null;
}

export function TourRoot() {
  // Starting a tour needs an overlay to show it.
  useEffect(() => {
    useTour.setState((s) => ({ hosts: s.hosts + 1 }));
    return () => useTour.setState((s) => ({ hosts: s.hosts - 1 }));
  }, []);
  const active = useTour((s) => s.tour !== null);
  return active ? <ActiveTour /> : null;
}

function ActiveTour() {
  const t = useT();
  const tour = useTour((s) => s.tour)!;
  const ctx = useTour((s) => s.ctx)!;
  const index = useTour((s) => s.index);
  const missing = useTour((s) => s.missing);
  const satisfied = useTour((s) => s.satisfied);
  const confirmExit = useTour((s) => s.confirmExit);
  const nudge = useTour((s) => s.nudge);
  const step = tour.steps[index]!;
  const titleId = useId();
  const maskId = `eth-tour-mask-${useId().replace(/[^\w-]/g, "")}`;

  const bubbleRef = useRef<HTMLDivElement>(null);
  const targetsRef = useRef<Element[]>([]);
  const [layout, setLayout] = useState<Layout>({ viewport: { w: window.innerWidth, h: window.innerHeight }, rects: [], bubble: null });
  const layoutRef = useRef(layout);
  useLayoutEffect(() => {
    layoutRef.current = layout;
  });

  const interactive = step.interactive ?? step.until !== undefined;
  const transportKeys = step.transportKeys === true;
  const centered = missing || !step.targets;

  // Follow the targets every frame (resizes, scrolling, animations, dialogs opening), and
  // let the engine check the step's condition.
  useEffect(() => {
    let raf = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const frame = () => {
      const s = useTour.getState();
      const cur = currentStep(s);
      const els = findTargets(cur, s.ctx);
      targetsRef.current = els;
      const viewport = { w: window.innerWidth, h: window.innerHeight };
      const rects = s.missing ? [] : (els.map(rectOf).filter(Boolean) as Rect[]);
      const box = bubbleRef.current;
      const size = box ? { w: box.offsetWidth, h: box.offsetHeight } : null;
      // The bubble also keeps clear of a picker opened from the highlighted part (a menu, a list).
      const pickers = cur?.targets && !s.missing ? [...document.querySelectorAll(PICKERS)].map(rectOf).filter((r): r is Rect => r !== null) : [];
      const area = s.missing || !cur?.targets ? null : union([...holes(rects, viewport), ...pickers]);
      const bubble = size ? placeBubble(area, size, viewport, s.missing ? "center" : cur?.placement) : null;
      const prev = layoutRef.current;
      if (
        prev.viewport.w !== viewport.w ||
        prev.viewport.h !== viewport.h ||
        !sameRects(prev.rects, rects) ||
        prev.bubble?.x !== bubble?.x ||
        prev.bubble?.y !== bubble?.y ||
        prev.bubble?.side !== bubble?.side
      )
        setLayout({ viewport, rects, bubble });
      tickTour(performance.now(), cur?.targets ? els.length : null);
      schedule();
    };
    const schedule = () => {
      if (typeof requestAnimationFrame === "function") raf = requestAnimationFrame(frame);
      else timer = setTimeout(frame, 50);
    };
    frame();
    return () => {
      if (raf) cancelAnimationFrame(raf);
      clearTimeout(timer);
    };
  }, []);

  // Input: only the bubble and (interactive steps) the highlighted parts.
  const regions = useRef({ interactive, transportKeys });
  useLayoutEffect(() => {
    regions.current = { interactive, transportKeys };
  });
  useEffect(
    () =>
      installGuard({
        bubble: () => bubbleRef.current,
        targets: () => findTargets(currentStep(), useTour.getState().ctx),
        interactive: () => regions.current.interactive && !useTour.getState().missing,
        transportKeys: () => regions.current.transportKeys,
        onEscape: () => askExit(!useTour.getState().confirmExit),
        onBlocked: nudgeTour,
      }),
    [],
  );
  // The app's shortcuts (keymap dispatcher, transport keys) rest, unless the step wants space.
  useEffect(() => (transportKeys ? undefined : pauseDispatch()), [transportKeys]);
  // A press in the bubble is nobody else's "outside click" (an open popover stays open).
  useEffect(() => {
    const el = bubbleRef.current;
    if (!el) return;
    const stop = (e: Event) => e.stopPropagation();
    el.addEventListener("pointerdown", stop);
    el.addEventListener("mousedown", stop);
    return () => {
      el.removeEventListener("pointerdown", stop);
      el.removeEventListener("mousedown", stop);
    };
  }, []);

  // A new step: its parts scrolled into view, the bubble focused (screen readers read it).
  useEffect(() => {
    const t = setTimeout(() => {
      targetsRef.current[0]?.scrollIntoView?.({ block: "nearest", inline: "nearest" });
      const box = bubbleRef.current;
      if (box && !box.contains(document.activeElement)) box.focus({ preventScroll: true });
    }, 0);
    return () => clearTimeout(t);
  }, [index]);

  // A blocked click: the bubble pulses (unless motion is reduced).
  useEffect(() => {
    const el = bubbleRef.current;
    if (nudge === 0 || !el?.animate || window.matchMedia?.("(prefers-reduced-motion: reduce)").matches) return;
    el.animate([{ transform: "scale(1)" }, { transform: "scale(1.03)" }, { transform: "scale(1)" }], { duration: 400, easing: "ease-out" });
  }, [nudge]);

  const shown = visibleSteps(tour, ctx);
  const position = shown.indexOf(step);
  const first = position <= 0;
  const last = position === shown.length - 1;
  const advance = !step.until || satisfied || missing;
  const waiting = !!step.until && !satisfied && !missing;
  const hs = centered ? [] : holes(layout.rects, layout.viewport);
  const bubbleStyle: CSSProperties = layout.bubble ? { left: layout.bubble.x, top: layout.bubble.y } : { visibility: "hidden" };

  // Three layers, each its own stacking context (tutorials.css): the dim under popovers, the
  // rings and the bubble above them (a menu opened during a step doesn't hide the highlight).
  return createPortal(
    <>
      {/* data-*: the step's state, for the tour walkthrough (apps/web/e2e/tours.walkthrough.spec.ts). */}
      <div
        className="eth-tour"
        data-testid="tour"
        data-tour-id={tour.id}
        data-step={step.id}
        data-targets={step.targets ? (Array.isArray(step.targets) ? step.targets.join(" ") : "") : undefined}
        data-missing={missing || undefined}
        data-waiting={waiting || undefined}
      >
        <svg className="eth-tour__dim" width={layout.viewport.w} height={layout.viewport.h} aria-hidden>
          <defs>
            <mask id={maskId}>
              <rect x={0} y={0} width={layout.viewport.w} height={layout.viewport.h} className="eth-tour__mask-fill" />
              {hs.map((h, i) => (
                <rect key={i} className="eth-tour__hole" x={h.x} y={h.y} width={h.w} height={h.h} style={{ x: h.x, y: h.y, width: h.w, height: h.h }} />
              ))}
            </mask>
          </defs>
          <rect className="eth-tour__shade" x={0} y={0} width={layout.viewport.w} height={layout.viewport.h} mask={`url(#${maskId})`} />
          {/* Catches presses on the dimmed area; the holes let them through (even-odd). */}
          <path className="eth-tour__catch" d={dimPath(layout.viewport, hs)} fillRule="evenodd" />
        </svg>
      </div>
      <div className="eth-tour__rings" aria-hidden>
        {hs.map((h, i) => (
          <div
            key={i}
            className="eth-tour__ring"
            data-testid="tour-ring"
            data-interactive={interactive || undefined}
            style={{ left: h.x, top: h.y, width: h.w, height: h.h }}
          />
        ))}
      </div>
      <div
        ref={bubbleRef}
        className="eth-tour__bubble"
        data-side={layout.bubble?.side}
        style={bubbleStyle}
        role="dialog"
        aria-modal="false"
        aria-labelledby={titleId}
        tabIndex={-1}
        data-testid="tour-bubble"
      >
        <header className="eth-tour__head">
          <span className="eth-tour__tour">{t(tour.title)}</span>
          <span className="eth-tour__count">
            {position + 1} / {shown.length}
          </span>
          <IconButton size="sm" tone="ghost" label={t("tutorials.shell.exit")} icon={<X />} onClick={() => askExit(true)} />
        </header>
        <div className="eth-tour__content" aria-live="polite">
          {step.illustration && !missing && <div className="eth-tour__illustration">{step.illustration}</div>}
          <h2 className="eth-tour__title" id={titleId}>
            {t(step.title)}
          </h2>
          <div className="eth-tour__body">{missing && step.fallback !== undefined ? textOf(step.fallback, ctx) : textOf(step.body, ctx)}</div>
          {missing && step.fallback === undefined && <p className="eth-tour__note">{t("tutorials.shell.missing")}</p>}
          {waiting && step.hint && (
            <p className="eth-tour__hint" role="status">
              <span className="eth-tour__hint-dot" aria-hidden />
              {t(step.hint)}
            </p>
          )}
        </div>
        {confirmExit ? (
          <footer className="eth-tour__foot eth-tour__foot--confirm" role="alertdialog" aria-label={t("tutorials.shell.exit_confirm_label")}>
            <span className="eth-tour__confirm">{t("tutorials.shell.exit_confirm")}</span>
            <Button size="sm" onClick={() => askExit(false)} autoFocus>
              {t("tutorials.shell.keep_going")}
            </Button>
            <Button size="sm" tone="danger" onClick={exitTour}>
              {t("tutorials.shell.exit_button")}
            </Button>
          </footer>
        ) : (
          <footer className="eth-tour__foot">
            {!first && (
              <Button size="sm" tone="ghost" onClick={backStep}>
                {t("tutorials.shell.back")}
              </Button>
            )}
            <span className="eth-tour__spacer" />
            {step.actions?.map((a) => (
              <Button key={a.id} size="sm" onClick={() => finishTour(a)}>
                {t(a.label)}
              </Button>
            ))}
            {advance && (
              <Button size="sm" tone="accent" onClick={() => (last ? finishTour() : nextStep())}>
                {last ? t("tutorials.shell.finish") : t("tutorials.shell.next")}
              </Button>
            )}
          </footer>
        )}
      </div>
    </>,
    document.body,
  );
}
