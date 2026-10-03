/**
 * Mock of `Version::*` (v0.3, contracts-4). Owned by `project-versions`: rolling autosaved
 * versions of the current project (`List`, `Create`, `Restore`, `Delete`, `Rename`,
 * `Compare`) and crash recovery (`ListRecoverable`, `Recover`, `DiscardRecovery`) over the
 * mock project store. Until the node lands every command fails `Unsupported`, like the
 * engine (`crates/ether-controller/tests/roadmap_v4.rs`).
 */

import type { ReplyValue, VersionCommand } from "@/generated";
import { fail } from "../documentReducer";

export function versionCommand(c: VersionCommand): ReplyValue {
  return fail("Unsupported", `Version::${c.type} is not implemented yet (project-versions)`);
}
