import clsx from "clsx";
import { Scissors } from "lucide-react";
import { useState, type DragEvent } from "react";
import type { Device } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { SamplePicker } from "@/features/devices/SamplePicker";
import { notify } from "@/features/notifications";
import { errorMessage } from "@/features/transport-bar/engine";
import { useT } from "@/i18n";
import { Button, Popover } from "@/kit";
import { useProjectStore } from "@/state";
import { useTransport } from "@/transport";
import { firstFreeNote, slicePayloadToPads } from "./sliceActions";

/**
 * "slice to pads" of a drum rack window: a sample picked here, or dropped on the button from
 * the browser, is cut at its transients into one pad per slice (each a sampler playing its
 * slice), from the key above the rack's pads. One undo step. `onSliced` gets the first key.
 */
export function SliceToPads({ rack, onSliced }: { rack: Device; onSliced(note: number): void }) {
  const t = useT();
  const transport = useTransport();
  const [open, setOpen] = useState(false);
  const [over, setOver] = useState(false);
  const [busy, setBusy] = useState(false);
  const free = useProjectStore((s) => (s.project ? firstFreeNote(s.project, rack.id) : null));

  const slice = async (payload: BrowserDragPayload) => {
    const project = useProjectStore.getState().project;
    const note = project ? firstFreeNote(project, rack.id) : null;
    if (!project || note === null) return;
    setBusy(true);
    try {
      await slicePayloadToPads(transport, project, rack.id, payload, note);
      onSliced(note);
    } catch (e) {
      notify("Error", t("drum_rack.slice.failed", { name: payload.name, error: errorMessage(e) }));
    } finally {
      setBusy(false);
    }
  };

  const onDragOver = (e: DragEvent) => {
    if (!hasBrowserDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setOver(true);
  };
  const onDrop = (e: DragEvent) => {
    setOver(false);
    const payload = readBrowserDrag(e.dataTransfer);
    if (!payload || payload.file_kind !== "Audio") return;
    e.preventDefault();
    void slice(payload);
  };

  return (
    <div
      className={clsx("eth-drum-rack__slice-drop", over && "eth-drum-rack__slice-drop--over")}
      onDragOver={onDragOver}
      onDragLeave={() => setOver(false)}
      onDrop={onDrop}
    >
      <Popover
        aria-label={t("drum_rack.slice.pick_sample")}
        placement="bottom-start"
        open={open}
        onOpenChange={setOpen}
        trigger={(p) => (
          <Button size="sm" disabled={free === null || busy} title={t("drum_rack.slice.to_pads_title")} data-testid="slice-to-pads" {...p}>
            <Scissors aria-hidden />
            {busy ? t("drum_rack.slice.slicing") : t("drum_rack.slice.to_pads")}
          </Button>
        )}
      >
        {(close) => (
          <SamplePicker
            onPick={(payload) => {
              close();
              void slice(payload);
            }}
          />
        )}
      </Popover>
    </div>
  );
}
