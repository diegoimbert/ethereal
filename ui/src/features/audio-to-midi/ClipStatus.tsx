import { TriangleAlert, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { Clip } from "@/generated";
import type { EngineTransport } from "@/transport";
import { ConvertDialog } from "./ConvertDialog";
import { cancelConversion, openConvertDialog, useAudioToMidi } from "./store";

/** Longer than the kit dialog's exit animation. */
const DIALOG_LINGER_MS = 300;

const stop = (e: { stopPropagation(): void }) => e.stopPropagation();

/**
 * Audio to MIDI on an audio clip (mounted in its clip view): while the clip converts, a
 * progress bar along its bottom edge and a stop button in its title; after a failure, a
 * warning (click: details in the dialog). Hosts the clip's Convert dialog.
 */
export function AudioToMidiClipStatus({ clip, transport }: { clip: Clip; transport: EngineTransport }) {
  const progress = useAudioToMidi((s) => (s.job?.clip === clip.id ? s.job.progress : null));
  const failed = useAudioToMidi((s) => (s.error?.clip === clip.id ? s.error.message : null));
  const dialog = useAudioToMidi((s) => s.dialog === clip.id);
  const name = clip.name || "clip";
  // The dialog stays mounted a moment after closing (its exit animation).
  const [mounted, setMounted] = useState(dialog);
  if (dialog && !mounted) setMounted(true);
  useEffect(() => {
    if (dialog || !mounted) return;
    const t = setTimeout(() => setMounted(false), DIALOG_LINGER_MS);
    return () => clearTimeout(t);
  }, [dialog, mounted]);
  return (
    <>
      {progress !== null && (
        <>
          <button
            type="button"
            className="eth-a2m__clip-stop"
            aria-label={`Stop converting ${name} to MIDI`}
            title={`Converting to MIDI: ${Math.round(progress * 100)}% (click to stop)`}
            onPointerDown={stop}
            onDoubleClick={stop}
            onClick={(e) => {
              e.stopPropagation();
              void cancelConversion(transport);
            }}
          >
            <X />
          </button>
          <span
            className="eth-a2m__clip-progress"
            role="progressbar"
            aria-label={`Converting ${name} to MIDI`}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(progress * 100)}
          >
            <span className="eth-a2m__bar-fill" style={{ transform: `scaleX(${progress})` }} />
          </span>
        </>
      )}
      {failed && progress === null && (
        <button
          type="button"
          className="eth-a2m__clip-error"
          aria-label={`Convert to MIDI failed: ${failed}`}
          title={`${failed} (click for details)`}
          onPointerDown={stop}
          onDoubleClick={stop}
          onClick={(e) => {
            e.stopPropagation();
            openConvertDialog(clip.id);
          }}
        >
          <TriangleAlert />
        </button>
      )}
      {mounted && <ConvertDialog clip={clip} transport={transport} />}
    </>
  );
}
