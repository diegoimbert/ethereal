import clsx from "clsx";
import { AudioWaveform, Music2, Piano } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import "./leftPanels.css";
import type { DeviceDescriptor } from "@/generated";
import { fetchBuiltinTypes } from "@/features/devices/descriptors";
import { useOptionalTransport } from "@/features/transport-bar/engine";
import { useArrangementUi } from "@/features/arrangement/state";
import { useProjectStore, useSelectionStore } from "@/state";
import { canInsert, DEVICE_CATEGORIES, deviceTargetTrack, insertDeviceCommand } from "./deviceInsert";

const CATEGORY_ICON: Record<DeviceDescriptor["category"], ReactNode> = {
  Instrument: <Piano />,
  AudioEffect: <AudioWaveform />,
  NoteEffect: <Music2 />,
};

/**
 * The built-in devices, by category. Clicking one adds it to the selected track
 * (instruments first in its chain, effects last); instruments are disabled on audio tracks.
 */
export function DevicesPanel() {
  const transport = useOptionalTransport();
  const project = useProjectStore((s) => s.project);
  // Subscribed so the target below follows the selection.
  useArrangementUi((s) => s.trackFocus);
  useSelectionStore((s) => s.selectedTrack);
  const track = project ? deviceTargetTrack(project) : undefined;
  const [types, setTypes] = useState<DeviceDescriptor[] | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    if (!transport) return;
    let active = true;
    fetchBuiltinTypes(transport).then(
      (list) => active && setTypes(list),
      () => active && setTypes([]),
    );
    return () => {
      active = false;
    };
  }, [transport]);

  const add = async (d: DeviceDescriptor) => {
    const p = useProjectStore.getState().project;
    const t = p ? deviceTargetTrack(p) : undefined;
    const command = transport && p && t ? await insertDeviceCommand(transport, p, t, d) : null;
    if (!transport || !command || !t) return;
    transport.send(command).then(
      () => setMessage(`Added ${d.name} to ${t.name}`),
      (e: unknown) => setMessage(e instanceof Error ? e.message : String(e)),
    );
  };

  if (!transport || !project) return <div className="eth-devices-panel__hint">No engine connected</div>;
  // A VCA carries no audio and takes no devices (the engine rejects inserts).
  const isVca = track?.kind === "Vca";
  return (
    <div className="eth-devices-panel" data-feature="devices-panel">
      <div className="eth-devices-panel__target">
        {isVca ? (
          <>
            <strong>{track.name}</strong> is a VCA: it takes no devices
          </>
        ) : track ? (
          <>
            Adds to <strong>{track.name}</strong>
          </>
        ) : (
          "Select a track to add devices"
        )}
      </div>
      <div className="eth-devices-panel__list" aria-busy={types === null}>
        {DEVICE_CATEGORIES.map((c) => {
          const list = (types ?? []).filter((d) => d.category === c.id && d.device_type.type === "Builtin");
          if (!list.length) return null;
          return (
            <section key={c.id} className="eth-devices-panel__group" aria-label={c.label}>
              <h3 className="eth-devices-panel__heading">{c.label}</h3>
              {list.map((d) => {
                const ok = !isVca && canInsert(d, track);
                return (
                  <button
                    key={d.name}
                    type="button"
                    className={clsx("eth-devices-panel__item", !ok && "eth-devices-panel__item--disabled")}
                    disabled={!ok}
                    aria-label={`Add ${d.name}`}
                    title={
                      ok
                        ? `Add ${d.name} to ${track?.name ?? "the track"}`
                        : isVca
                          ? "A VCA takes no devices"
                          : "Instruments go on MIDI tracks"
                    }
                    onClick={() => void add(d)}
                  >
                    <span className="eth-devices-panel__icon" aria-hidden>
                      {CATEGORY_ICON[d.category]}
                    </span>
                    {d.name}
                  </button>
                );
              })}
            </section>
          );
        })}
      </div>
      {message && (
        <div className="eth-devices-panel__status" role="status">
          {message}
        </div>
      )}
    </div>
  );
}
