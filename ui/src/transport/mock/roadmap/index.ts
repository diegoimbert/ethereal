/**
 * MockTransport simulations of the roadmap features, one file (and `*.test.ts`) per feature.
 *
 * Roadmap v2 (contracts-2):
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
 * | `remote.ts`     | remote-engine   | uploads (simulated)                               |
 *
 * v0.2 (contracts-3, docs/ROADMAP.md "v0.2"):
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
 * v0.3 (contracts-4, docs/ROADMAP.md "v0.3"):
 *
 * | file             | node                | commands                                              |
 * |------------------|---------------------|-------------------------------------------------------|
 * | `expression.ts`  | midi-expression     | `Expression::*` (document)                            |
 * | `mpe.ts`         | mpe                 | `Expression::SetTrackMpe` (document)                  |
 * | `capture.ts`     | capture-midi        | `Capture::*`                                          |
 * | `audioToMidi.ts` | audio-to-midi       | `AudioToMidi::*`                                      |
 * | `fxSpace.ts`     | fx-space            | `Device::{SetIr (document), ListFactoryIrs}`          |
 * | `external.ts`    | external-instrument | `External::{SetRouting (document), ListPorts, MeasureLatency}` |
 * | `undoHistory.ts` | undo-history        | `History::*`                                          |
 * | `templates.ts`   | templates           | `Template::{Insert (document), *}`                    |
 * | `versions.ts`    | project-versions    | `Version::*`                                          |
 * | `keymap.ts`      | keymap              | `Keymap::*`                                           |
 *
 * Device groups' mock descriptors are generated JSON in `../devices/`.
 *
 * `shared.ts` and `host.ts` are base files (cascades, the MockTransport host interface).
 * The core reducer / MockTransport call into these with one line per feature.
 */

import { contributions } from "@/ext";
import type { Command } from "@/generated";
import type { ReducerContext } from "../documentReducer";
import { markerCommand } from "./clipEditing";
import { drumRackCommand, sliceCommand } from "./drumRack";
import { grooveCommand } from "./groove";
import { midiMapCommand } from "./midiLearn";
import { tempoCommand } from "./tempo";
// v0.2 (contracts-3): one file per node.
import { takeCommand } from "./comping";
import { modulationCommand, rackCommand } from "./racksModulation";
// v0.3 (contracts-4): one file per node.
import { expressionCommand } from "./expression";
import { setExternalRouting } from "./external";
import { insertTemplate } from "./templates";

/** Roadmap domains that are document commands (undoable, allowed in a `Batch`). */
export function isRoadmapDocumentCommand(command: Command): boolean {
  switch (command.domain) {
    case "Tempo":
    case "Marker":
    case "Groove":
    case "DrumRack":
    case "Slice":
      return true;
    case "MidiMap":
      return command.command.type !== "Learn" && command.command.type !== "List";
    // v0.2 (contracts-3).
    case "Take":
    case "Rack":
      return true;
    case "Modulation":
      return command.command.type !== "ListModulatorKinds";
    // v0.3 (contracts-4).
    case "Expression":
      return true;
    case "External":
      return command.command.type === "SetRouting";
    case "Template":
      return command.command.type === "Insert";
    default:
      // An extension's document domain (`@/ext` "mock.docDomain").
      return contributions("mock.docDomain").some((d) => d.domain === command.domain);
  }
}

/**
 * Apply a roadmap document command; `false` if `command` is not one. `deleteDevice` is the
 * core device cascade (used when pads are removed).
 */
export function reduceRoadmapCommand(ctx: ReducerContext, command: Command, deleteDevice: (id: string) => void): boolean {
  const extension = contributions("mock.docDomain").find((d) => d.domain === command.domain);
  if (extension) {
    extension.apply(ctx, (command as { command: unknown }).command);
    return true;
  }
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
      sliceCommand(ctx, command.command, deleteDevice);
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
    // v0.3 (contracts-4).
    case "Expression":
      expressionCommand(ctx, command.command);
      return true;
    case "External":
      if (command.command.type !== "SetRouting") return false;
      setExternalRouting(ctx, command.command);
      return true;
    case "Template":
      if (command.command.type !== "Insert") return false;
      insertTemplate(ctx, command.command);
      return true;
    default:
      return false;
  }
}
