/** MockTransport: uploads (remote-engine; staged since file-import). */
import { describe, expect, it } from "vitest";
import type { Event } from "@/generated";
import { bytesToBase64 } from "../../ws/binaryFrame";
import { cmd } from "../../cmd";
import { useMock } from "./testUtils";
import { wavInfo } from "./remote";

/** A 16-bit PCM WAV of `frames` silent frames. */
function wav(rate: number, channels: number, frames: number): Uint8Array {
  const data = frames * channels * 2;
  const b = new Uint8Array(44 + data);
  const v = new DataView(b.buffer);
  const tag = (at: number, s: string) => [...s].forEach((c, i) => (b[at + i] = c.charCodeAt(0)));
  tag(0, "RIFF");
  v.setUint32(4, 36 + data, true);
  tag(8, "WAVE");
  tag(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, channels, true);
  v.setUint32(24, rate, true);
  v.setUint32(28, rate * channels * 2, true);
  v.setUint16(32, channels * 2, true);
  v.setUint16(34, 16, true);
  tag(36, "data");
  v.setUint32(40, data, true);
  return b;
}

describe("MockTransport uploads", () => {
  const f = useMock();

  async function upload(name: string, bytes: Uint8Array, chunk = 100): Promise<string> {
    const upload = `u-${name.replace(/\W/g, "")}`;
    await f.mock.send(cmd("Media", { type: "BeginUpload", upload, name, size: bytes.length }));
    for (let at = 0; at < bytes.length; at += chunk) {
      await f.mock.send(cmd("Media", { type: "UploadChunk", upload, offset: at, data: bytesToBase64(bytes.subarray(at, at + chunk)) }));
    }
    return upload;
  }

  it("stage chunks and import a WAV with its real format", async () => {
    const events: Event[] = [];
    f.mock.onEvent((e) => events.push(e));
    const bytes = wav(22050, 1, 441);
    const u = await upload("Kick.wav", bytes);
    expect(events.filter((e) => e.type === "Media" && e.event.type === "UploadProgress").length).toBeGreaterThan(1);
    const reply = await f.mock.send(cmd("Media", { type: "Import", id: "01JMEDIAUPLOAD0000000000000", source: { type: "Upload", upload: u } }));
    expect(reply).toMatchObject({ type: "Media", media: { name: "Kick.wav", sample_rate: 22050, channels: 1, frames: 441 } });
    // Consumed.
    await expect(
      f.mock.send(cmd("Media", { type: "Import", id: "01JMEDIAUPLOAD0000000000001", source: { type: "Upload", upload: u } })),
    ).rejects.toMatchObject({ code: "NotFound" });
  });

  it("reject gaps, oversize, incomplete imports and non-audio files", async () => {
    await expect(f.mock.send(cmd("Media", { type: "BeginUpload", upload: "big", name: "a.wav", size: 2 ** 31 }))).rejects.toMatchObject({
      code: "InvalidArgument",
    });
    await f.mock.send(cmd("Media", { type: "BeginUpload", upload: "g", name: "a.wav", size: 10 }));
    await expect(
      f.mock.send(cmd("Media", { type: "UploadChunk", upload: "g", offset: 4, data: bytesToBase64(new Uint8Array(2)) })),
    ).rejects.toMatchObject({ code: "InvalidState" });
    await expect(
      f.mock.send(cmd("Media", { type: "Import", id: "01JMEDIAUPLOAD0000000000002", source: { type: "Upload", upload: "g" } })),
    ).rejects.toMatchObject({ code: "InvalidState" });
    const txt = await upload("notes.txt", new Uint8Array([1, 2, 3]));
    await expect(
      f.mock.send(cmd("Media", { type: "Import", id: "01JMEDIAUPLOAD0000000000003", source: { type: "Upload", upload: txt } })),
    ).rejects.toMatchObject({ code: "Decode" });
  });

  it("parse WAV headers", () => {
    expect(wavInfo(wav(48000, 2, 10))).toEqual({ sample_rate: 48000, channels: 2, frames: 10 });
    expect(wavInfo(new Uint8Array([1, 2, 3]))).toBeNull();
  });
});
