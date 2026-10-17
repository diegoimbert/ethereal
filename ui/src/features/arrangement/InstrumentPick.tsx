import type { Track } from "@/generated";
import { DeviceLibrary } from "@/features/devices/DeviceLibrary";
import { insertFromLibrary } from "@/features/devices/userInsert";
import { Popover } from "@/kit";
import { useTransport } from "@/transport";
import { useArrangementUi } from "./uiStore";
import { useT } from "@/i18n";

/**
 * The instrument library under a MIDI track's header, open right after the track was added
 * from Add track, by click or drag (`instrumentPick`, `addTrackAndSelect`): picking one replaces the track's Synth;
 * clicking away keeps it. Placed below the header, above it when there is no room.
 */
export function InstrumentPick({ track }: { track: Track }) {
  const transport = useTransport();
  const t = useT();
  const open = useArrangementUi((s) => s.instrumentPick === track.id);
  if (!open) return null;
  const close = () => useArrangementUi.getState().setInstrumentPick(null);
  return (
    <div className="eth-arr-header__pick">
      <Popover
        open
        onOpenChange={(o) => {
          if (!o) close();
        }}
        placement="bottom-start"
        aria-label={t("arrangement.header.choose_instrument", { name: track.name })}
        trigger={() => <span className="eth-arr-header__pick-anchor" />}
      >
        {/* React events bubble out of the portal into the header: its press would start a
            track drag (and take the click) or select the track. */}
        {/* eslint-disable-next-line jsx-a11y/no-static-element-interactions -- only keeps its events from reaching the track header around it */}
        <div
          className="eth-arr-header__pick-panel"
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => e.stopPropagation()}
          onDoubleClick={(e) => e.stopPropagation()}
          onContextMenu={(e) => e.stopPropagation()}
          onKeyDown={(e) => {
            // Escape reaches the Popover (it closes); other keys stay out of the header.
            if (e.key !== "Escape") e.stopPropagation();
          }}
        >
          <DeviceLibrary
            category="Instrument"
            onClose={close}
            onPick={(payload) => {
              close();
              void insertFromLibrary(transport, track, payload);
            }}
          />
        </div>
      </Popover>
    </div>
  );
}
