/**
 * Piano-roll groove controls: a Quantize popover (grid, strength, swing, ends) and a
 * Humanize popover (timing, velocity). Each apply is one command = one undo step and acts
 * on the selected notes, or on every note of the clip when nothing is selected.
 */

import { useCallback } from "react";
import type { Beats, ClipId, Command, NoteId } from "@/generated";
import { Button, NumberField, Popover, Select, Toggle } from "@/kit";
import { useTransport } from "@/transport";
import {
  grooveQuantizeCommand,
  humanizeCommand,
  newSeed,
  QUANTIZE_GRIDS,
  useGrooveSettings,
  type QuantizeGrid,
} from "./grooveCommands";
import "./groove.css";

export interface GrooveControlsProps {
  clip: ClipId;
  /** Selected note ids (empty = the whole clip). */
  selected: ReadonlyArray<NoteId>;
  /** The piano roll's current grid step, used by the "Roll grid" choice. */
  rollStep: Beats;
}

const GRID_OPTIONS = QUANTIZE_GRIDS.map((g) => ({ value: g.value, label: g.label }));

function useSendCommand(): (command: Command) => void {
  const transport = useTransport();
  return useCallback(
    (command: Command) => {
      transport.send(command).catch((e: unknown) => console.warn("[groove] command failed:", e));
    },
    [transport],
  );
}

/** Percent field over a 0..1 value. */
function Percent({ label, value, onChange }: { label: string; value: number; onChange: (v: number) => void }) {
  return (
    <label className="eth-groove__field">
      <span className="eth-groove__label">{label}</span>
      <NumberField
        size="sm"
        aria-label={label}
        value={Math.round(value * 100)}
        min={0}
        max={100}
        unit="%"
        onChange={(v) => onChange(v / 100)}
      />
    </label>
  );
}

export function GrooveControls({ clip, selected, rollStep }: GrooveControlsProps) {
  const send = useSendCommand();
  const quantize = useGrooveSettings((s) => s.quantize);
  const humanize = useGrooveSettings((s) => s.humanize);
  const { setQuantize, setHumanize } = useGrooveSettings.getState();
  const scope = selected.length > 0 ? `${selected.length} selected` : "all notes";

  return (
    <>
      <Popover
        aria-label="Quantize"
        trigger={(p) => (
          <Button size="sm" title="Quantize settings" {...p}>
            Quantize…
          </Button>
        )}
      >
        {(close) => (
          <div className="eth-groove__popover" data-testid="groove-quantize">
            <label className="eth-groove__field">
              <span className="eth-groove__label">Grid</span>
              <Select<QuantizeGrid>
                size="sm"
                aria-label="Quantize grid"
                options={GRID_OPTIONS}
                value={quantize.grid}
                onChange={(grid) => setQuantize({ grid })}
              />
            </label>
            <Percent label="Strength" value={quantize.strength} onChange={(strength) => setQuantize({ strength })} />
            <Percent label="Swing" value={quantize.swing} onChange={(swing) => setQuantize({ swing })} />
            <Toggle size="sm" label="Quantize ends" checked={quantize.ends} onChange={(ends) => setQuantize({ ends })} />
            <Button
              size="sm"
              tone="accent"
              onClick={() => {
                send(grooveQuantizeCommand(clip, selected, quantize, rollStep));
                close();
              }}
            >
              Quantize {scope}
            </Button>
          </div>
        )}
      </Popover>
      <Popover
        aria-label="Humanize"
        trigger={(p) => (
          <Button size="sm" title="Randomize timing and velocity" {...p}>
            Humanize…
          </Button>
        )}
      >
        {(close) => (
          <div className="eth-groove__popover" data-testid="groove-humanize">
            <Percent label="Timing (±1/16)" value={humanize.timing} onChange={(timing) => setHumanize({ timing })} />
            <Percent label="Velocity" value={humanize.velocity} onChange={(velocity) => setHumanize({ velocity })} />
            <Button
              size="sm"
              tone="accent"
              onClick={() => {
                send(humanizeCommand(clip, selected, humanize, newSeed()));
                close();
              }}
            >
              Humanize {scope}
            </Button>
          </div>
        )}
      </Popover>
    </>
  );
}
