import clsx from "clsx";
import { useState, type DragEvent } from "react";
import type { Device } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, resolveDroppedMedia } from "@/features/browser/dragPayload";
import { Button } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, nextGestureId, useTransport } from "@/transport";

/**
 * The sample of a Sampler device: shows the loaded file and takes a new one dropped from
 * the sample browser (import + `Device::SetSample` as one undo step). Renders nothing for
 * other devices.
 */
export function SampleSlot({ device }: { device: Device }) {
  const transport = useTransport();
  const kind = device.kind;
  const sample = kind.type === "Builtin" && kind.device.type === "Sampler" ? kind.device.sample : undefined;
  const media = useProjectStore((s) => (sample ? s.project?.media[sample] : undefined));
  const [over, setOver] = useState(false);
  const [error, setError] = useState<string | null>(null);
  if (sample === undefined) return null;

  const onDragOver = (e: DragEvent) => {
    if (!hasBrowserDrag(e.dataTransfer)) return;
    e.preventDefault();
    e.stopPropagation();
    e.dataTransfer.dropEffect = "copy";
    setOver(true);
  };

  const onDrop = async (e: DragEvent) => {
    setOver(false);
    const payload = readBrowserDrag(e.dataTransfer);
    if (!payload) return;
    e.preventDefault();
    e.stopPropagation();
    setError(null);
    const gesture = nextGestureId();
    try {
      const m = await resolveDroppedMedia(transport, payload, { gesture });
      await transport.send(cmd("Device", { type: "SetSample", device: device.id, media: m.id }), { gesture });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
    }
  };

  const clear = () => void transport.send(cmd("Device", { type: "SetSample", device: device.id, media: null })).catch(() => {});

  return (
    <div
      className={clsx("eth-sample-slot", over && "eth-sample-slot--over", !sample && "eth-sample-slot--empty")}
      data-testid="sample-slot"
      onDragOver={onDragOver}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => void onDrop(e)}
    >
      <span className="eth-sample-slot__name" title={media?.name}>
        {error ? `Load failed: ${error}` : sample ? (media?.name ?? "Sample") : "Drop a sample here from the Browser"}
      </span>
      {sample && (
        <Button size="sm" tone="ghost" aria-label="Remove sample" title="Remove sample" onClick={clear}>
          ✕
        </Button>
      )}
    </div>
  );
}
