/**
 * Mock of `AudioToMidi::*` (v0.3, contracts-4). Owned by `audio-to-midi`: offline
 * conversion jobs (`Start` → `AudioToMidiEvent::{Progress, Done, Failed}`, `Cancel`); the
 * resulting MIDI track/clip is inserted as one undo step. Until the node lands every
 * command fails `Unsupported`, like the engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { AudioToMidiCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export function audioToMidiCommand(c: AudioToMidiCommand): ReplyValue {
  return fail("Unsupported", `AudioToMidi::${c.type} is not implemented yet (audio-to-midi)`);
}
