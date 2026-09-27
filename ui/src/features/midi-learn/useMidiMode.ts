// MIDI mode: while on, every mappable control in the app is highlighted (mapped ones and the
// one being learned are marked), a click on one starts learning it instead of editing it
// (clicking it again cancels), right-click offers "Learn MIDI" / "Remove MIDI mapping", and
// Escape cancels a learn. Works on the whole document (controls belong to other features),
// through capture-phase listeners that only swallow events on mappable controls.
import { useEffect } from "react";
import type { MidiMapTarget } from "@/generated";
import { openContextMenu } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, type EngineTransport } from "@/transport";
import { useMidiLearnStore } from "./store";
import { MAPPABLE_SELECTOR, mappingOf, resolveControl, sameTarget, targetKey } from "./targets";

export const MIDI_MODE_CLASS = "eth-midi-mode";
export const MAPPABLE_ATTR = "data-midi-mappable";
export const MAPPED_ATTR = "data-midi-mapped";
export const LEARNING_ATTR = "data-midi-learning";

/** Mark mappable controls under `root` (idempotent). */
export function markControls(root: ParentNode): void {
  const project = useProjectStore.getState().project;
  const { learning } = useMidiLearnStore.getState();
  const learningKey = learning ? targetKey(learning) : null;
  const seen = new Set<Element>();
  for (const el of root.querySelectorAll(MAPPABLE_SELECTOR)) {
    const r = resolveControl(el, project);
    // Only the element a control resolves to is marked (not its inner parts).
    if (!r || r.element !== el) continue;
    seen.add(el);
    el.setAttribute(MAPPABLE_ATTR, "");
    toggleAttr(el, MAPPED_ATTR, !!mappingOf(project?.midi_mappings, r.target));
    toggleAttr(el, LEARNING_ATTR, targetKey(r.target) === learningKey);
  }
  for (const el of root.querySelectorAll(`[${MAPPABLE_ATTR}]`)) {
    if (!seen.has(el)) clearMarks(el);
  }
}

export function clearControls(root: ParentNode): void {
  for (const el of root.querySelectorAll(`[${MAPPABLE_ATTR}]`)) clearMarks(el);
}

function clearMarks(el: Element) {
  el.removeAttribute(MAPPABLE_ATTR);
  el.removeAttribute(MAPPED_ATTR);
  el.removeAttribute(LEARNING_ATTR);
}

function toggleAttr(el: Element, name: string, on: boolean) {
  if (on) {
    if (!el.hasAttribute(name)) el.setAttribute(name, "");
  } else if (el.hasAttribute(name)) {
    el.removeAttribute(name);
  }
}

/** Start (or, for the target already being learned, cancel) learning `target`. */
export function learn(transport: EngineTransport, target: MidiMapTarget | null): Promise<unknown> {
  const current = useMidiLearnStore.getState().learning;
  const next = target && sameTarget(current, target) ? null : target;
  return transport.send(cmd("MidiMap", { type: "Learn", target: next })).catch((e: unknown) => console.warn("MIDI learn failed", e));
}

function unmap(transport: EngineTransport, target: MidiMapTarget) {
  const m = mappingOf(useProjectStore.getState().project?.midi_mappings, target);
  if (m) void transport.send(cmd("MidiMap", { type: "Unmap", ids: [m.id] })).catch((e: unknown) => console.warn("unmap failed", e));
}

/**
 * Install MIDI mode on the document while `active` (and learning is possible). Turning it
 * off (or unmounting) removes every mark and cancels a learn in progress.
 */
export function useMidiMode(transport: EngineTransport | null, active: boolean): void {
  useEffect(() => {
    if (!transport || !active || typeof document === "undefined") return;
    const root = document.body;
    root.classList.add(MIDI_MODE_CLASS);

    let frame = 0;
    const schedule = () => {
      if (frame) return;
      frame = requestAnimationFrame(() => {
        frame = 0;
        markControls(document);
      });
    };
    markControls(document);
    const observer = new MutationObserver(schedule);
    observer.observe(root, { childList: true, subtree: true });
    const offProject = useProjectStore.subscribe(schedule);
    const offLearn = useMidiLearnStore.subscribe(schedule);

    const resolve = (e: globalThis.Event) => {
      const t = e.target;
      if (!(t instanceof Element) || t.closest('[data-feature="midi-learn"]')) return null;
      return resolveControl(t, useProjectStore.getState().project);
    };
    const onPointerDown = (e: PointerEvent | MouseEvent) => {
      const r = resolve(e);
      if (!r) return;
      e.preventDefault();
      e.stopPropagation();
      // Learn on the primary button's pointerdown (the matching mousedown/click are swallowed).
      if (e.type === "pointerdown" && (e.button ?? 0) === 0) void learn(transport, r.target);
    };
    const swallow = (e: globalThis.Event) => {
      if (!resolve(e)) return;
      e.preventDefault();
      e.stopPropagation();
    };
    const onContextMenu = (e: MouseEvent) => {
      const r = resolve(e);
      if (!r) return;
      const mapped = !!mappingOf(useProjectStore.getState().project?.midi_mappings, r.target);
      const learning = sameTarget(useMidiLearnStore.getState().learning, r.target);
      openContextMenu(e, [
        { label: learning ? "Cancel MIDI learn" : "Learn MIDI", onSelect: () => void learn(transport, r.target) },
        { label: "Remove MIDI mapping", danger: true, disabled: !mapped, onSelect: () => unmap(transport, r.target) },
      ]);
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && useMidiLearnStore.getState().learning) void learn(transport, null);
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("mousedown", onPointerDown, true);
    document.addEventListener("click", swallow, true);
    document.addEventListener("dblclick", swallow, true);
    document.addEventListener("contextmenu", onContextMenu, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("mousedown", onPointerDown, true);
      document.removeEventListener("click", swallow, true);
      document.removeEventListener("dblclick", swallow, true);
      document.removeEventListener("contextmenu", onContextMenu, true);
      document.removeEventListener("keydown", onKeyDown);
      observer.disconnect();
      offProject();
      offLearn();
      if (frame) cancelAnimationFrame(frame);
      root.classList.remove(MIDI_MODE_CLASS);
      clearControls(document);
      if (useMidiLearnStore.getState().learning) void learn(transport, null);
    };
  }, [transport, active]);
}
