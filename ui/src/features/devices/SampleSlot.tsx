import clsx from "clsx";
import { createContext, useContext, useState, type DragEvent } from "react";
import type { Device, DeviceId, MediaId } from "@/generated";
import { hasBrowserDrag, readBrowserDrag, resolveDroppedMedia, type BrowserDragPayload } from "@/features/browser/dragPayload";
import { Button, Popover } from "@/kit";
import { useProjectStore } from "@/state";
import { cmd, nextGestureId, useTransport, type EngineTransport } from "@/transport";
import { SamplePicker } from "./SamplePicker";
import { useT } from "@/i18n";

/**
 * Loads a sample (a browser drag payload: dropped, or picked in the sample picker) into a
 * Sampler: import if needed + `Device::SetSample`, one undo step.
 */
export async function loadSample(transport: EngineTransport, device: DeviceId, payload: BrowserDragPayload): Promise<void> {
  const gesture = nextGestureId();
  try {
    const m = await resolveDroppedMedia(transport, payload, { gesture });
    await transport.send(cmd("Device", { type: "SetSample", device, media: m.id }), { gesture });
  } finally {
    transport.send(cmd("Edit", { type: "EndGesture", gesture })).catch(() => {});
  }
}

/**
 * The sample picker of a Sampler's window, shared by whichever slot the window shows (the
 * layout's waveform chip, or the bare slot while the descriptor loads): open when the window
 * opens on a Sampler without a sample. Slots of other devices (a slice editor's sampler in a
 * Drum Rack window) keep their own, closed, picker.
 */
export const SamplePickerWindow = createContext<{ device: DeviceId; open: boolean; setOpen(open: boolean): void } | null>(null);

/**
 * Dropping a sample from the browser on a Sampler (import + `Device::SetSample` as one undo
 * step): drag handlers for any drop target (the slot, the waveform), whether a drag is over
 * it, and the last load error.
 */
export function useSampleDrop(device: Device) {
  const transport = useTransport();
  const [over, setOver] = useState(false);
  const [error, setError] = useState<string | null>(null);

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
    await load(payload);
  };

  const load = async (payload: BrowserDragPayload) => {
    setError(null);
    try {
      await loadSample(transport, device.id, payload);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return {
    over,
    error,
    load,
    handlers: {
      onDragOver,
      onDragLeave: () => setOver(false),
      onDrop: (e: DragEvent) => void onDrop(e),
    },
  };
}

/** The sample of a Sampler (`null`: none loaded), `undefined` for other devices. */
export function samplerSample(device: Device): MediaId | null | undefined {
  const kind = device.kind;
  return kind.type === "Builtin" && kind.device.type === "Sampler" ? kind.device.sample : undefined;
}

/**
 * The sample of a Sampler device: shows the loaded file and takes a new one dropped from
 * the sample browser; a click opens the sample picker (a pick replaces the sample). Renders
 * nothing for other devices.
 */
export function SampleSlot({ device }: { device: Device }) {
  const transport = useTransport();
  const sample = samplerSample(device);
  const media = useProjectStore((s) => (sample ? s.project?.media[sample] : undefined));
  const { over, error, load, handlers } = useSampleDrop(device);
  const shared = useContext(SamplePickerWindow);
  const [ownOpen, setOwnOpen] = useState(false);
  const win = shared?.device === device.id ? shared : null;
  const open = win ? win.open : ownOpen;
  const setOpen = win ? win.setOpen : setOwnOpen;
  const t = useT();
  if (sample === undefined) return null;

  const clear = () => void transport.send(cmd("Device", { type: "SetSample", device: device.id, media: null })).catch(() => {});

  return (
    <div className={clsx("eth-sample-slot", over && "eth-sample-slot--over", !sample && "eth-sample-slot--empty")} data-testid="sample-slot" {...handlers}>
      <Popover
        aria-label={t("devices.samples.select")}
        placement="bottom-start"
        open={open}
        onOpenChange={setOpen}
        trigger={(p) => (
          <button
            type="button"
            className="eth-sample-slot__pick"
            title={sample ? t("devices.samples.replace", { name: media?.name ?? t("devices.samples.sample") }) : t("devices.samples.select")}
            {...p}
          >
            <span className="eth-sample-slot__name">
              {error ? (
                t("devices.samples.load_failed", { error })
              ) : sample ? (
                media ? (
                  <span className="eth-user-text">{media.name}</span>
                ) : (
                  t("devices.samples.sample")
                )
              ) : (
                t("devices.samples.drop_or_select")
              )}
            </span>
          </button>
        )}
      >
        {(close) => (
          <SamplePicker
            onPick={(payload) => {
              close();
              void load(payload);
            }}
          />
        )}
      </Popover>
      {sample && (
        <Button size="sm" tone="ghost" aria-label={t("devices.samples.remove")} title={t("devices.samples.remove")} onClick={clear}>
          ✕
        </Button>
      )}
    </div>
  );
}
