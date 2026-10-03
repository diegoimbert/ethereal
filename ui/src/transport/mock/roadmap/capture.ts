/**
 * Mock of `Capture::*` (v0.3, contracts-4). Owned by `capture-midi`: the always-on MIDI
 * capture buffer (`Capture` turns what was just played into a clip as one undo step,
 * `Clear`, `Status`). Until the node lands every command fails `Unsupported`, like the
 * engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { CaptureCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export function captureCommand(c: CaptureCommand): ReplyValue {
  return fail("Unsupported", `Capture::${c.type} is not implemented yet (capture-midi)`);
}
