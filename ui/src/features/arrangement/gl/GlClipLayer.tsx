/**
 * The arranger's clip layer: one canvas drawing every clip of the lanes (WebGL2, or the
 * Canvas2D fallback: see backendChoice.ts), between the grid (below) and the lane DOM
 * (above: drop hints, live recording, …), plus the pointer handling of the clips.
 *
 * Pointer events land on the lanes (the canvas takes none) and are hit-tested against the
 * scene in the capture phase, before the lane's own handlers: a clip's title band and edge
 * handles start the clip gesture (`onClipPointerDown`, with the part hit), its body
 * lets the press through to the lane (locate, marquee, MIDI insert), as before, except with
 * Ctrl+Shift (Cmd+Shift) held: that slips the clip's content (`onClipSlipPointerDown`).
 * DOM overlays in the lanes that take the pointer themselves keep it.
 */

import { contributions } from "@/ext";
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { Clip, ClipId } from "@/generated";
import { openContextMenu } from "@/kit";
import { clipDisplayName, isReadOnly, useEditorStore, useProjectStore } from "@/state";
import { itemSelection, subscribeSettledZoom, TempoMap } from "@/timeline";
import { subscribeLaneFrames } from "@/features/automation";
import { fullClipMenu } from "../clipContextMenu";
import { AudioToMidiClipStatus, useAudioToMidi } from "@/features/audio-to-midi";
import { onClipPointerDown } from "../clipDrag";
import { onClipSlipPointerDown } from "../clipSlip";
import { isSlipModifier, SLIP_CURSOR } from "../slip";
import { renameClipInPlace, renameClipsCommand, useClipRename } from "../clipRename";
import { sendEdit, type ArrangementContextValue } from "../context";
import { arrangementView, useArrangementUi } from "../uiStore";
import { cursorFor, type Hit } from "./hitTest";
import { readPalette, type Palette } from "./palette";
import { ArrangerRenderer } from "./renderer";
import { reportWebglFailure, reportWebglLoss, WEBGL_RESTORE_TIMEOUT_MS, type BackendKind } from "./backendChoice";
import type { Hover, SceneInput } from "./scene";
import { setGlRenderer } from "./active";
import { ClipSummary } from "./ClipSummary";
import { mountOverlayLayers } from "./layers/overlays";
import { size, space } from "@/theme";
import { t as tr, useT } from "@/i18n";

/** The audio-to-MIDI stop / failure button (`--eth-size-icon-sm`) and the title's gap. */
const A2M_BUTTON_PX = parseFloat(size.iconSm);
const TITLE_GAP_PX = parseFloat(space.xs);
import { mountClipEditLayers } from "./layers/index";
import { mountLaneLayers } from "./layers/lanes";

export interface GlClipLayerProps {
  ctx: ArrangementContextValue;
  scrollRef: React.RefObject<HTMLDivElement | null>;
  /**
   * What draws. Fixed for the layer's life: a WebGL failure (`reportWebglFailure`) remounts
   * it (keyed by the backend and the attempt).
   */
  backend: BackendKind;
}

/** How often the extensions watching the clips in view are told about them. */
const VISIBLE_CLIPS_MS = 250;

export function GlClipLayer({ ctx, scrollRef, backend }: GlClipLayerProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [renderer, setRenderer] = useState<ArrangerRenderer | null>(null);
  const headerWidth = useArrangementUi((s) => s.headerWidth);

  // The renderer, its inputs and its triggers.
  useLayoutEffect(() => {
    const canvas = canvasRef.current;
    // Ancestors: their refs aren't attached yet when a child's layout effect first runs.
    const scroller = scrollRef.current ?? canvas?.closest<HTMLDivElement>(".eth-arr__scroll") ?? null;
    const content = ctx.contentRef.current ?? canvas?.closest<HTMLDivElement>(".eth-arr__content") ?? null;
    if (!canvas || !scroller) return;
    const r = new ArrangerRenderer({ canvas, view: arrangementView, peaks: ctx.peaks, rows: () => ctx.rowsRef.current, backend });
    if (!r.ok) {
      // The WebGL context couldn't be created: retry on a new canvas, then fall back.
      r.dispose();
      reportWebglFailure();
      return;
    }
    setRenderer(r);
    setGlRenderer(r);
    const unmountLayers = mountClipEditLayers(r, ctx.transport);

    // Phase-2c overlays: collaborators' presence, new-track ghost lanes (layers/).
    const overlays = mountOverlayLayers(r, () => ctx.rowsRef.current);
    let palette: Palette = readPalette();
    let hover: Hover | null = null;
    let tempo: { key: [unknown, unknown]; map: TempoMap } | null = null;
    const tempoOf = (p: NonNullable<SceneInput["project"]>) => {
      if (!tempo || tempo.key[0] !== p.tempo_points || tempo.key[1] !== p.time_signatures) {
        tempo = { key: [p.tempo_points, p.time_signatures], map: TempoMap.fromProject(p) };
      }
      return tempo.map;
    };
    const update = () => {
      const project = useProjectStore.getState().project;
      if (!project) return;
      r.setInput({
        project,
        preview: useArrangementUi.getState().preview,
        selected: itemSelection.getState().selected.clip,
        hover,
        renaming: useClipRename.getState().clip,
        palette,
        tempo: tempoOf(project),
        transportKind: ctx.transport.kind,
      });
    };
    const viewport = () => {
      const hw = useArrangementUi.getState().headerWidth;
      r.setViewport({
        width: Math.max(0, (content?.clientWidth ?? scroller.clientWidth) - hw),
        height: scroller.clientHeight,
        scrollTop: scroller.scrollTop,
        headerWidth: hw,
        bleed: bleedOf(backend, hw),
      });
    };
    update();
    viewport();
    r.render();

    // A lost WebGL context is restored by the renderer; one that doesn't come back, or is
    // lost again and again, falls back to Canvas2D.
    let restoreTimer: ReturnType<typeof setTimeout> | undefined;
    const offLoss = r.onContextLost(() => {
      reportWebglLoss();
      clearTimeout(restoreTimer);
      restoreTimer = setTimeout(() => {
        if (!r.backendImpl.ready) reportWebglFailure();
      }, WEBGL_RESTORE_TIMEOUT_MS);
    });

    const offs: Array<() => void> = [
      offLoss,
      () => clearTimeout(restoreTimer),
      // Automation, comping and group summary lanes (gl/layers/).
      mountLaneLayers(r),
      useProjectStore.subscribe(update),
      itemSelection.subscribe(update),
      useClipRename.subscribe(update),
      useArrangementUi.subscribe((s, prev) => {
        if (s.preview !== prev.preview) update();
        if (s.headerWidth !== prev.headerWidth) viewport();
      }),
      onThemeChange(() => {
        palette = readPalette();
        overlays.onTheme();
        update();
      }),
      () => overlays.dispose(),
      arrangementView.subscribe(() => r.requestRender(true)),
      subscribeSettledZoom(arrangementView, () => r.requestRender()),
      subscribeLaneFrames(() => r.requestRender(true)),
    ];
    const onScroll = () => viewport();
    scroller.addEventListener("scroll", onScroll, { passive: true });
    offs.push(() => scroller.removeEventListener("scroll", onScroll));
    const ro = new ResizeObserver(viewport);
    ro.observe(scroller);
    if (content) ro.observe(content);
    offs.push(() => ro.disconnect());
    // Text is rasterized in the UI font: redo it once the font has loaded.
    const fonts = typeof document !== "undefined" ? document.fonts : undefined;
    if (fonts) {
      const reset = () => r.resetText();
      void fonts.ready.then(reset);
      fonts.addEventListener("loadingdone", reset);
      offs.push(() => fonts.removeEventListener("loadingdone", reset));
    }

    // Audio to MIDI's status button (`ClipStatusOverlay`) sits after the clip's name.
    const statusClip = (s: ReturnType<typeof useAudioToMidi.getState>) => s.job?.clip ?? s.error?.clip ?? null;
    offs.push(
      r.addTitleReserve({
        id: "audio-to-midi-status",
        side: "right",
        width: (clip) => (statusClip(useAudioToMidi.getState()) === clip.id ? A2M_BUTTON_PX + TITLE_GAP_PX : 0),
      }),
      useAudioToMidi.subscribe((s, prev) => {
        if (statusClip(s) !== statusClip(prev)) r.invalidateTitles();
      }),
    );

    // Pointer: hit-tested in the capture phase (see the module doc).
    const local = (e: MouseEvent) => {
      const box = content!.getBoundingClientRect();
      return { x: e.clientX - box.left, y: e.clientY - box.top };
    };
    /** Presses on the lanes themselves (not on DOM overlays that take the pointer). */
    const isLaneEl = (t: unknown) => t instanceof HTMLElement && (t === content || t.classList.contains("eth-arr-lane"));
    const onLanes = (e: Event) => isLaneEl(e.target);
    const hitAt = (e: MouseEvent): Hit | null => {
      if (!onLanes(e)) return null;
      const p = local(e);
      return r.hitTest(p.x, p.y);
    };
    /**
     * A double-click's hit. Its target can be the track's row rather than its lane: the first
     * press selects the track, and a selected row changes `content-visibility`, so until the
     * next rendering update the browser hit-tests the row without its content. The row's own
     * structure (not a control, nor the header column) counts as the lane.
     */
    const hitForDoubleClick = (e: MouseEvent): Hit | null => {
      const t = e.target;
      const structural = t instanceof HTMLElement && (isLaneEl(t) || t.classList.contains("eth-arr-row") || t.classList.contains("eth-arr-row__main"));
      if (!structural) return null;
      const p = local(e);
      if (p.x < useArrangementUi.getState().headerWidth) return null;
      return r.hitTest(p.x, p.y);
    };
    const clipOf = (id: ClipId): Clip | undefined => useProjectStore.getState().project?.clips[id];
    const isClipPart = (h: Hit | null): h is Hit => !!h && (h.part === "header" || h.part === "small" || h.part === "resize-start" || h.part === "resize-end");

    const onPointerDown = (e: PointerEvent) => {
      if (e.button !== 0) return;
      const hit = hitAt(e);
      // A shared project's viewer: clip presses select; fades, gain lines and slips don't start.
      if (isReadOnly() && (hit?.part === "layer" || hit?.part === "body")) return;
      if (hit?.part === "layer") {
        r.layers.byId(hit.layer!)?.onPointerDown?.(hit, e);
        return;
      }
      if (hit?.part === "body" && isSlipModifier(e)) {
        const clip = clipOf(hit.clip);
        if (clip) onClipSlipPointerDown(e, clip, ctx);
        return;
      }
      if (!isClipPart(hit)) return;
      const clip = clipOf(hit.clip);
      if (!clip) return;
      onClipPointerDown(e, clip, ctx, { mode: hit.part === "resize-start" || hit.part === "resize-end" ? hit.part : "move" });
    };
    const setHover = (next: Hover | null) => {
      if (next?.clip === hover?.clip && next?.part === hover?.part && next?.data === hover?.data) return;
      hover = next;
      update();
    };
    let cursor = "";
    let lastHit: Hit | null = null;
    const showCursor = (hit: Hit | null, mods: { shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }) => {
      const next =
        hit?.part === "layer"
          ? (r.layers.byId(hit.layer!)?.cursor?.(hit) ?? "")
          : hit?.part === "body" && isSlipModifier(mods)
            ? SLIP_CURSOR
            : cursorFor(hit?.part ?? null);
      if (next !== cursor && content) {
        cursor = next;
        content.style.cursor = next;
      }
    };
    // Pressing or releasing the slip modifiers over a clip body changes the cursor at once.
    const onKey = (e: KeyboardEvent) => {
      if (["Shift", "Control", "Meta"].includes(e.key)) showCursor(lastHit, e);
    };
    window.addEventListener("keydown", onKey);
    window.addEventListener("keyup", onKey);
    offs.push(() => {
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", onKey);
    });
    const onPointerMove = (e: PointerEvent) => {
      if (e.buttons !== 0) return;
      const hit = hitAt(e);
      lastHit = hit;
      showCursor(hit, e);
      // Every part, the body and layer hits included: layers show things while a clip is
      // hovered (e.g. its fade handles).
      setHover(hit ? { clip: hit.clip, part: hit.part, data: typeof hit.data === "string" ? hit.data : undefined } : null);
    };
    const onLeave = () => {
      lastHit = null;
      setHover(null);
    };
    const onDoubleClick = (e: MouseEvent) => {
      const hit = hitForDoubleClick(e);
      if (hit?.part === "layer") {
        if (isReadOnly()) return;
        r.layers.byId(hit.layer!)?.onDoubleClick?.(hit, e);
        return;
      }
      // The body too (an audio clip's: a MIDI lane's double press opens its clips, clipInsert.ts).
      if (!hit || (hit.part !== "body" && !isClipPart(hit))) return;
      e.stopPropagation();
      // A double-click opens the clip and expands the collapsed drawer and inspector.
      useEditorStore.getState().revealClip(hit.clip);
    };
    const onContextMenu = (e: MouseEvent) => {
      const hit = hitAt(e);
      if (hit?.part === "layer") {
        r.layers.byId(hit.layer!)?.onContextMenu?.(hit, e);
        return;
      }
      if (!isClipPart(hit)) return;
      const clip = clipOf(hit.clip);
      if (clip) openContextMenu(e, fullClipMenu(ctx.transport, clip));
    };
    if (content) {
      content.addEventListener("pointerdown", onPointerDown, true);
      content.addEventListener("pointermove", onPointerMove, { passive: true });
      content.addEventListener("pointerleave", onLeave);
      content.addEventListener("dblclick", onDoubleClick, true);
      content.addEventListener("contextmenu", onContextMenu, true);
      offs.push(() => {
        content.removeEventListener("pointerdown", onPointerDown, true);
        content.removeEventListener("pointermove", onPointerMove);
        content.removeEventListener("pointerleave", onLeave);
        content.removeEventListener("dblclick", onDoubleClick, true);
        content.removeEventListener("contextmenu", onContextMenu, true);
        content.style.cursor = "";
      });
    }

    // Extensions that follow what the arranger shows (`@/ext` "arrangement.visibleClips"):
    // the clips in view, the edited one first, four times a second.
    const watchers = contributions("arrangement.visibleClips");
    if (watchers.length > 0) {
      const timer = window.setInterval(() => {
        const edited = useEditorStore.getState().clip;
        const ids = [...(edited ? [edited] : []), ...r.visibleClipIds()].filter((id, i, all) => all.indexOf(id) === i);
        for (const watch of watchers) watch(ids, ctx.transport);
      }, VISIBLE_CLIPS_MS);
      offs.push(() => window.clearInterval(timer));
    }

    return () => {
      for (const off of offs) off();
      unmountLayers();
      setGlRenderer(null);
      setRenderer(null);
      r.dispose();
    };
    // The renderer lives as long as the view: its inputs are read from the stores.
  }, [ctx, scrollRef, backend]);

  return (
    <div className="eth-arr-gl" style={{ left: headerWidth - bleedOf(backend, headerWidth) }} data-testid="arranger-gl" data-backend={backend}>
      {/* Pixels only: the clips are in ClipSummary, as text. */}
      <canvas ref={canvasRef} className="eth-arr-gl__canvas" aria-hidden />
      {renderer && <RenameOverlay renderer={renderer} ctx={ctx} />}
      {renderer && <ClipStatusOverlay renderer={renderer} ctx={ctx} />}
      {renderer && <ClipSummary />}
      {renderer && <SelectionAnnouncer />}
    </div>
  );
}

/**
 * Css px the clip layer draws under the header column (whose translucent background shows the
 * clips passing under it): all of it on WebGL; none on the Canvas2D fallback (cut at the lanes).
 */
function bleedOf(backend: string, headerWidth: number): number {
  return backend === "webgl" ? headerWidth : 0;
}

/** Calls `fn` when the theme (`<html data-theme>`, `data-theme-id`, `data-theme-rev`) or the glow setting (`data-glow`) changes. */
function onThemeChange(fn: () => void): () => void {
  const mo = new MutationObserver(fn);
  mo.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme", "data-theme-id", "data-theme-rev", "data-glow"] });
  return () => mo.disconnect();
}

/**
 * Renaming a clip (its menu's "Rename"): the name field over the clip's title band, as in
 * the DOM. Enter or a click away renames, Escape cancels.
 */
function RenameOverlay({ renderer, ctx }: { renderer: ArrangerRenderer; ctx: ArrangementContextValue }): ReactNode {
  const id = useClipRename((s) => s.clip);
  const clip = useProjectStore((s) => (id ? s.project?.clips[id] : undefined));
  const rect = useTitleRect(renderer, id);
  if (!id || !clip || !rect) return null;
  return (
    <div className="eth-arr-gl__rename" style={{ left: rect.x, top: rect.y, width: rect.width }}>
      {clip.looping.enabled && <span className="eth-arr-gl__rename-icon" />}
      <ClipNameField key={clip.id} clip={clip} ctx={ctx} />
    </div>
  );
}

/** A clip's title band in the layer's px (following pan, zoom and layout), or null. */
function useTitleRect(renderer: ArrangerRenderer, id: ClipId | null): { x: number; width: number; y: number } | null {
  const [rect, setRect] = useState<{ x: number; width: number; y: number } | null>(null);
  useLayoutEffect(() => {
    if (!id) return;
    const place = () => {
      const r = renderer.clipRect(id, "header");
      // The layer starts at the canvas's left edge (the header column's, when drawing under it).
      const left = useArrangementUi.getState().headerWidth - (renderer.getViewport().bleed ?? 0);
      setRect((old) => {
        const next = r ? { x: r.x - left, width: r.width, y: r.y } : null;
        return old && next && old.x === next.x && old.y === next.y && old.width === next.width ? old : next;
      });
    };
    place();
    const offs = [arrangementView.subscribe(place), useProjectStore.subscribe(place), useArrangementUi.subscribe(place)];
    return () => offs.forEach((off) => off());
  }, [id, renderer]);
  return id ? rect : null;
}

/**
 * Audio to MIDI's status in a clip's title (its progress and stop button, or the failure
 * button, and its dialog): a DOM overlay at the end of the title band, for the one clip
 * being converted (`AudioToMidiClipStatus`).
 */
function ClipStatusOverlay({ renderer, ctx }: { renderer: ArrangerRenderer; ctx: ArrangementContextValue }): ReactNode {
  const id = useAudioToMidi((s) => s.job?.clip ?? s.error?.clip ?? s.dialog ?? null);
  const clip = useProjectStore((s) => (id ? s.project?.clips[id] : undefined));
  const rect = useTitleRect(renderer, id);
  if (!id || !clip || !rect || clip.content.type !== "Audio") return null;
  return (
    <div className="eth-arr-gl__status" style={{ left: rect.x, top: rect.y, width: rect.width }} data-testid="clip-status-overlay">
      <AudioToMidiClipStatus clip={clip} transport={ctx.transport} />
    </div>
  );
}

function ClipNameField({ clip, ctx }: { clip: Clip; ctx: ArrangementContextValue }) {
  const t = useT();
  const done = useRef(false);
  // The clips the name goes to (the selection when the menu opened).
  const count = useClipRename((s) => s.ids.length);
  const finish = (name: string | null) => {
    if (done.current) return;
    done.current = true;
    const ids = useClipRename.getState().ids;
    renameClipInPlace(null);
    if (name === null) return;
    // Empty: each back to its default name (an audio clip's sample name; a MIDI clip none).
    const project = useProjectStore.getState().project;
    const edit = project && renameClipsCommand(project, ids.length ? ids : [clip.id], name);
    if (edit) void sendEdit(ctx.transport, edit);
  };
  return (
    <input
      className="eth-clip__rename"
      aria-label={
        count > 1
          ? t("arrangement.clip_menu.rename", { count })
          : clip.name
            ? t("arrangement.clip.rename_named", { name: clip.name })
            : t("arrangement.clip.rename_unnamed")
      }
      placeholder={clip.content.type === "Midi" ? t("arrangement.clip.midi_placeholder") : t("arrangement.clip.audio_placeholder")}
      defaultValue={clip.name}
      autoFocus
      spellCheck={false}
      onFocus={(e) => e.currentTarget.select()}
      onPointerDown={(e) => e.stopPropagation()}
      onDoubleClick={(e) => e.stopPropagation()}
      onBlur={(e) => finish(e.currentTarget.value)}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") finish(e.currentTarget.value);
        else if (e.key === "Escape") finish(null);
      }}
    />
  );
}

/** Screen readers: the canvas has no clip elements, so the selection is announced here. */
function SelectionAnnouncer() {
  const [text, setText] = useState("");
  useEffect(() => {
    const announce = () => {
      const project = useProjectStore.getState().project;
      const ids = [...itemSelection.getState().selected.clip];
      const names = ids.map((id) => project?.clips[id]).filter((c): c is Clip => !!c);
      setText(
        names.length === 0
          ? ""
          : names.length === 1
            ? tr("arrangement.clip.selected_one", {
                name:
                  (project && clipDisplayName(project, names[0]!)) ||
                  tr(names[0]!.content.type === "Midi" ? "arrangement.clip.midi_placeholder" : "arrangement.clip.audio_placeholder"),
              })
            : tr("arrangement.clip.selected_many", { count: names.length }),
      );
    };
    announce();
    return itemSelection.subscribe(announce);
  }, []);
  return (
    <div className="eth-arr-gl__live" aria-live="polite" role="status" data-testid="arranger-selection-live">
      {text}
    </div>
  );
}
