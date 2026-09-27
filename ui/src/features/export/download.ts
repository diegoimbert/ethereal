/**
 * Web/remote exports: the engine keeps the finished files and the UI pulls them with
 * `Export::ReadChunk` (base64), then saves them as a browser download. Native hosts write
 * the files into the project folder instead (`ExportResult::Files`).
 */

import type { ExportDownload } from "@/generated";
import { cmd, type EngineTransport } from "@/transport";

/** Bytes asked per `ReadChunk` (the host serves at least 256 KiB). */
export const CHUNK_BYTES = 1 << 20;

function base64ToBytes(data: string): Uint8Array {
  const bin = atob(data);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

/** Read a whole download from the engine. */
export async function fetchDownload(
  transport: EngineTransport,
  d: ExportDownload,
): Promise<Uint8Array> {
  const parts: Uint8Array[] = [];
  let offset = 0;
  for (;;) {
    const reply = await transport.send(
      cmd("Export", {
        type: "ReadChunk",
        token: d.token,
        offset,
        length: CHUNK_BYTES,
      }),
    );
    if (reply.type !== "Bytes")
      throw new Error(`unexpected reply ${reply.type}`);
    const bytes = base64ToBytes(reply.chunk.data);
    parts.push(bytes);
    offset += bytes.length;
    if (reply.chunk.eof || bytes.length === 0) break;
  }
  const out = new Uint8Array(offset);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/** Hand bytes to the browser as a file download. */
export function saveFile(name: string, mime: string, bytes: Uint8Array): void {
  const url = URL.createObjectURL(
    new Blob([bytes as BlobPart], { type: mime }),
  );
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.rel = "noopener";
  document.body.appendChild(a);
  a.click();
  a.remove();
  // Give the browser time to start the download before revoking.
  setTimeout(() => URL.revokeObjectURL(url), 30_000);
}
