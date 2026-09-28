/**
 * Roadmap v2 (contracts-2) MockTransport simulations, one file per feature node (each node
 * owns its file and its `*.test.ts`; see `.github/ownership.toml`):
 *
 * | file            | node            | commands                                          |
 * |-----------------|-----------------|---------------------------------------------------|
 * | `tempo.ts`      | tempo-metronome | `Tempo::*`                                        |
 * | `clipEditing.ts`| clip-editing    | `Marker::*`, `Clip::{SetFadeCurves, SetReversed, Crossfade}` |
 * | `groove.ts`     | groove          | `Groove::*`, quantize swing                       |
 * | `drumRack.ts`   | drum-rack       | `DrumRack::*`, `Slice::*`, rack structure rules    |
 * | `midiLearn.ts`  | midi-learn      | `MidiMap::*`, `MockMidiLearn`                     |
 * | `sidechain.ts`  | sidechain       | `Device::SetSidechain`                            |
 * | `export.ts`     | export          | `Export::*` (`MockExports`)                       |
 * | `collab.ts`     | collab          | `Collab::*` (unsupported)                          |
 * | `remote.ts`     | remote-engine   | uploads (unsupported)                             |
 * | `social.ts`     | collab-social   | `Chat::*`, `PinnedNote::*` (unsupported)          |
 *
 * v0.2 (contracts-3), same rules (see docs/ROADMAP.md "v0.2"):
 *
 * | file                 | node              | commands                                  |
 * |----------------------|-------------------|-------------------------------------------|
 * | `comping.ts`         | comping           | `Take::*`                                 |
 * | `freezeBounce.ts`    | freeze-bounce     | `Freeze::*`                               |
 * | `timeEdits.ts`       | time-edits        | `TimeEdit::*`                             |
 * | `presets.ts`         | presets           | `Preset::*`                               |
 * | `browserV2.ts`       | browser-v2        | `Browser::*`                              |
 * | `analysis.ts`        | fx-analysis       | `Analysis::*` (`MockAnalysis`)            |
 * | `racksModulation.ts` | racks-modulation  | `Rack::*`, `Modulation::*`                |
 * | `mediaReferences.ts` | media-references  | `MediaRef::*`                             |
 * | `groupsBuses.ts`     | groups-buses      | `Track::{GroupSelected, Ungroup, SetVca}` |
 *
 * Device groups' mock descriptors are generated JSON in `../devices/` (one file per node).
 *
 * `shared.ts` and `host.ts` are base files (cascades, the MockTransport host interface).
 * The core reducer / MockTransport call into these with one line per feature.
 */

import type { Command } from "@/generated";
import type { ReducerContext } from "../documentReducer";
import { markerCommand } from "./clipEditing";
import { drumRackCommand, sliceCommand } from "./drumRack";
import { grooveCommand } from "./groove";
import { midiMapCommand } from "./midiLearn";
import { pinnedNoteCommand } from "./social";
import { tempoCommand } from "./tempo";
// v0.2 (contracts-3): one file per node.
import { takeCommand } from "./comping";
import { modulationCommand, rackCommand } from "./racksModulation";

/** Roadmap domains that are document commands (undoable, allowed in a `Batch`). */
export function isRoadmapDocumentCommand(command: Command): boolean {
  switch (command.domain) {
    case "Tempo":
    case "Marker":
    case "Groove":
    case "DrumRack":
    case "Slice":
    case "PinnedNote":
      return true;
    case "MidiMap":
      return command.command.type !== "Learn" && command.command.type !== "List";
    // v0.2 (contracts-3).
    case "Take":
    case "Rack":
      return true;
    case "Modulation":
      return command.command.type !== "ListModulatorKinds";
    default:
      return false;
  }
}

/**
 * Apply a roadmap document command; `false` if `command` is not one. `deleteDevice` is the
 * core device cascade (used when pads are removed).
 */
export function reduceRoadmapCommand(ctx: ReducerContext, command: Command, deleteDevice: (id: string) => void): boolean {
  switch (command.domain) {
    case "Tempo":
      tempoCommand(ctx, command.command);
      return true;
    case "Marker":
      markerCommand(ctx, command.command);
      return true;
    case "Groove":
      grooveCommand(ctx, command.command);
      return true;
    case "DrumRack":
      drumRackCommand(ctx, command.command, deleteDevice);
      return true;
    case "Slice":
      sliceCommand(ctx, command.command);
      return true;
    case "MidiMap":
      midiMapCommand(ctx, command.command);
      return true;
    // v0.2 (contracts-3).
    case "Take":
      takeCommand(ctx, command.command);
      return true;
    case "Rack":
      rackCommand(ctx, command.command);
      return true;
    case "Modulation":
      modulationCommand(ctx, command.command);
      return true;
    case "PinnedNote":
      pinnedNoteCommand(ctx, command.command);
      return true;
    default:
      return false;
  }
}
