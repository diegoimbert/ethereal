// "change color…" of the clip menu: which clips the palette colors and where it opens.
import { create } from "zustand";
import type { ClipId } from "@/generated";
import { lastContextMenuPoint } from "@/kit";
import { cmd, type EngineTransport } from "@/transport";
import { asOneStep } from "./editMath";
import { sendEdit } from "./context";

interface ClipColorState {
  /** The clips being colored and where the palette opens (viewport px); null: closed. */
  open: { clips: ClipId[]; x: number; y: number } | null;
}

export const useClipColorPicker = create<ClipColorState>()(() => ({ open: null }));

/** Open the palette for `clips` where the last context menu was. */
export function openClipColorPicker(clips: ClipId[]): void {
  const at = lastContextMenuPoint() ?? { x: window.innerWidth / 2, y: window.innerHeight / 2 };
  useClipColorPicker.setState({ open: { clips, ...at } });
}

export function closeClipColorPicker(): void {
  useClipColorPicker.setState({ open: null });
}

/** Set every clip of `clips` to `color` (null: back to its track's color), one undo step. */
export function setClipsColor(transport: EngineTransport, clips: ClipId[], color: number | null): Promise<void> {
  const commands = clips.map((id) => cmd("Clip", { type: "SetColor", id, color }));
  return sendEdit(transport, asOneStep("set clip color", commands, { id: "undo.clips.color", args: {} }));
}
