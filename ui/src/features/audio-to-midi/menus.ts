/**
 * The clip context menu entry: "Convert to MIDI…" on audio clips (opens the dialog), or
 * "Stop Converting to MIDI" on the clip being converted. Inserted before the menu's last
 * group (the destructive "Delete" entries), like the freeze and clip-editing entries.
 */

import type { Clip } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import type { EngineTransport } from "@/transport";
import { t } from "@/i18n";
import { cancelConversion, isConvertible, openConvertDialog, useAudioToMidi } from "./store";

export function audioToMidiClipEntries(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  if (!isConvertible(clip)) return [];
  const job = useAudioToMidi.getState().job;
  if (job?.clip === clip.id) {
    return [{ label: t("audio_to_midi.menu.stop"), onSelect: () => void cancelConversion(transport) }];
  }
  return [{ label: t("audio_to_midi.menu.convert"), disabled: !!job, onSelect: () => openConvertDialog(clip.id) }];
}

export function withAudioToMidiEntries(entries: ReadonlyArray<ContextMenuEntry>, transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  const extra = audioToMidiClipEntries(transport, clip);
  if (extra.length === 0) return [...entries];
  const lastSep = entries.lastIndexOf("separator");
  if (lastSep < 0) return entries.length ? [...entries, "separator", ...extra] : extra;
  return [...entries.slice(0, lastSep), "separator", ...extra, ...entries.slice(lastSep)];
}
