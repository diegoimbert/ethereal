// A clip's context menu with every feature's entries: the arranger opens it from its hit
// test.
import type { Clip } from "@/generated";
import type { ContextMenuEntry } from "@/kit";
import type { EngineTransport } from "@/transport";
import { withFreezeClipEntries } from "@/features/freeze";
import { withAudioToMidiEntries } from "@/features/audio-to-midi";
import { withSliceClipEntries } from "@/features/drum-rack";
import { withMediaRefClipEntries } from "@/features/media-refs";
import { withResolveClipEntries } from "@/features/import-resolver";
import { withClipEditingEntries } from "@/features/clip-editing/clipEditing";
import { clipMenu } from "./actions";

export function fullClipMenu(transport: EngineTransport, clip: Clip): ContextMenuEntry[] {
  return withResolveClipEntries(
    withMediaRefClipEntries(
      withSliceClipEntries(
        withAudioToMidiEntries(withFreezeClipEntries(withClipEditingEntries(clipMenu(transport, clip), transport, clip), transport, clip), transport, clip),
        transport,
        clip,
      ),
      clip,
    ),
    clip,
  );
}
