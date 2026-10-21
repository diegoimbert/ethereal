import { useReadOnly } from "@/state/viewer";
import { AudioLines, CornerDownLeft, Piano, Plus } from "lucide-react";
import { useRef, useState, type PointerEvent, type ReactNode } from "react";
import { Knob, Popover, type TriggerProps } from "@/kit";
import { trackColor } from "@/theme";
import { addTrackAndSelect, type NewTrackKind } from "./actions";
import { useArrangement } from "./context";
import { startTrackDrag } from "./trackDrag";
import { useArrangementUi } from "./uiStore";
import { useT, type MessageKey } from "@/i18n";

/** The id the drop indicator uses for a track being added (no track has it). */
const NEW_TRACK_DRAG_ID = "__new_track__";

/** `color`: a track palette index (`--eth-track-<i>`), for the card's badge and knob. Texts are message keys. */
const KINDS: ReadonlyArray<{ kind: NewTrackKind; label: MessageKey; aria: MessageKey; hint: MessageKey; icon: ReactNode; color: number }> = [
  {
    kind: "Midi",
    label: "arrangement.new_track.midi",
    aria: "arrangement.new_track.create_midi",
    hint: "arrangement.new_track.midi_hint",
    icon: <Piano aria-hidden />,
    color: 1,
  },
  {
    kind: "Audio",
    label: "arrangement.new_track.audio",
    aria: "arrangement.new_track.create_audio",
    hint: "arrangement.new_track.audio_hint",
    icon: <AudioLines aria-hidden />,
    color: 2,
  },
  {
    kind: "Return",
    label: "arrangement.new_track.return",
    aria: "arrangement.new_track.create_return",
    hint: "arrangement.new_track.return_hint",
    icon: <CornerDownLeft aria-hidden />,
    color: 8,
  },
];

/**
 * "Add track": a menu of the track kinds (MIDI, audio, return). A click on one adds it after
 * the last regular track (a return after the selected return, else after the other returns);
 * dragging one between rows (a MIDI or audio one also onto a group) creates it there. A
 * return can only be dropped among the returns.
 */
export function AddTrackMenu({ trigger }: { trigger: (props: TriggerProps) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const t = useT();
  return (
    <Popover open={open} onOpenChange={setOpen} placement="bottom-start" role="presentation" haspopup="menu" className="eth-popover--menu" trigger={trigger}>
      <div className="eth-menu eth-arr-add-track" role="menu" aria-label={t("arrangement.new_track.add")}>
        {KINDS.map((k) => (
          <NewTrackItem key={k.kind} {...k} close={() => setOpen(false)} />
        ))}
        <p className="eth-arr-add-track__hint">{t("arrangement.new_track.drag_hint")}</p>
      </div>
    </Popover>
  );
}

/** One kind of the menu: click to add at the end, or drag where it goes. */
function NewTrackItem({ kind, label, aria, hint, icon, color, close }: (typeof KINDS)[number] & { close(): void }) {
  const t = useT();
  const ctx = useArrangement();
  const dragged = useRef(false);

  const onPointerDown = (e: PointerEvent<HTMLButtonElement>) => {
    dragged.current = false;
    if (e.button !== 0) return;
    startTrackDrag({
      ctx,
      id: NEW_TRACK_DRAG_ID,
      kind,
      startX: e.clientX,
      startY: e.clientY,
      vertical: false,
      insideOnly: true,
      onStart: () => {
        dragged.current = true;
        // The menu gets out of the way of the drop.
        close();
      },
      onDrop: (target) => void addTrackAndSelect(ctx.transport, kind, { parent: target.parent, before: target.before }),
    });
  };

  return (
    <button
      type="button"
      role="menuitem"
      className="eth-menu__item eth-arr-add-track__item"
      data-kind={kind}
      style={{ ["--eth-track-color" as string]: trackColor(color) }}
      aria-label={t(aria)}
      title={t(hint)}
      onPointerDown={onPointerDown}
      onClick={() => {
        // A drag ends with a click too: the drop already added it.
        if (dragged.current) {
          dragged.current = false;
          return;
        }
        close();
        void addTrackAndSelect(ctx.transport, kind);
      }}
    >
      {/* Looks like a track header: the badge, the name, then a pan knob that is only
          decoration (inert). */}
      <span className="eth-arr-header__badge" aria-hidden>
        {icon}
      </span>
      <span className="eth-menu__label">{t(label)}</span>
      <span className="eth-arr-add-track__deco" aria-hidden inert>
        <span className="eth-arr-pan">
          <Knob size="sm" bipolar hideLabel value={0.5} />
        </span>
      </span>
    </button>
  );
}

/** Subtle "+ Add track" row below the last track (in the header column): the same menu. */
export function AddTrackRow() {
  const headerWidth = useArrangementUi((s) => s.headerWidth);
  const t = useT();
  // A shared project's viewer adds nothing.
  if (useReadOnly()) return null;
  return (
    // The menu anchors to this slot (the button fills it): it opens under the button.
    <div className="eth-arr__add-track-slot" style={{ width: headerWidth }}>
      <AddTrackMenu
        trigger={(p) => (
          <button
            type="button"
            className="eth-arr__add-track"
            aria-label={t("arrangement.new_track.add_at_end")}
            onPointerDown={(e) => e.stopPropagation()}
            {...p}
          >
            <Plus aria-hidden /> {t("arrangement.new_track.add")}
          </button>
        )}
      />
    </div>
  );
}
