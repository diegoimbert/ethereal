/**
 * Mock of uploads from the UI machine (`Media::{BeginUpload, UploadChunk, CancelUpload}`,
 * `MediaSource::Upload`). Owned by `remote-engine`.
 *
 * The mock stands in for a *local* engine, like the web (wasm) host, whose store has no
 * upload staging: those hosts reply `Unsupported`, and the UI only offers uploads when it
 * talks to a remote engine (`transport.kind === "remote"`, see
 * `ui/src/features/remote/uploadDrop.ts`). The remote behaviour (staging, offsets, resume,
 * cancel, limits) is specified and tested in `crates/ether-controller/src/upload/mod.rs`.
 */

import type { MediaCommand, ReplyValue } from "@/generated";
import { fail } from "../documentReducer";

export type UploadCommand = Extract<MediaCommand, { type: "BeginUpload" | "UploadChunk" | "CancelUpload" }>;

export function uploadCommand(c: UploadCommand): ReplyValue {
  return fail("Unsupported", `uploads are not available in the mock engine (${c.type})`);
}

/** `MediaSource::Upload` (import/preview of a completed upload). */
export function uploadSource(upload: string): never {
  return fail("Unsupported", `uploads are not available in the mock engine (${upload})`);
}
