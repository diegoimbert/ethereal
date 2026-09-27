/**
 * Mock of uploads from the UI machine (`Media::{BeginUpload, UploadChunk, CancelUpload}`,
 * `MediaSource::Upload`): unsupported in the mock. Owned by `remote-engine`.
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
