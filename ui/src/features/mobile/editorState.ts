/** Whether the mobile clip editor panel is open, and opening a clip in it. */
import { create } from "zustand";
import type { Clip } from "@/generated";
import { useEditorStore } from "@/state";
import { useShellStore } from "@/state/shell";

/** `lanes`: the piano roll's velocity / expression lanes are shown (phones hide them by default: the grid needs the height). */
export const useMobileEditor = create<{ open: boolean; lanes: boolean }>()(() => ({ open: false, lanes: false }));

/** Open `clip` in its editor (the piano roll for MIDI, the audio editor for audio). */
export function openClipEditor(clip: Clip): void {
  useEditorStore.getState().openClip(clip.id);
  // The drawer's tab only (not its desktop open state, which is remembered).
  const tab = clip.content.type === "Midi" ? "piano-roll" : "warp";
  useShellStore.setState((s) => ({ bottom: { ...s.bottom, tab } }));
  useMobileEditor.setState({ open: true });
}

export const closeClipEditor = (): void => useMobileEditor.setState({ open: false });
