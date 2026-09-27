/** MockTransport: export (export). */
import { describe, expect, it } from "vitest";
import type { ReplyValue } from "@/generated";
import { cmd } from "../../cmd";
import { CommandFailedError } from "../../EngineTransport";
import { useMock } from "./testUtils";

describe("MockTransport export", () => {
  const f = useMock();

  it("export: progress, download, chunked read, release", async () => {
    await expect(
      f.mock.send(
        cmd("Export", {
          type: "Render",
          job: "j0",
          request: {
            range: { type: "Loop" },
            format: { container: "Flac", bit_depth: "Float32", sample_rate: null },
            mode: { type: "Mix" },
            normalize: false,
            tail_seconds: 0,
            name: null,
          },
        }),
      ),
    ).rejects.toBeInstanceOf(CommandFailedError);
    const reply = await f.mock.send(
      cmd("Export", {
        type: "Render",
        job: "j1",
        request: {
          range: { type: "Project" },
          format: { container: "Wav", bit_depth: "Int16", sample_rate: 44100 },
          mode: { type: "Mix" },
          normalize: true,
          tail_seconds: 1,
          name: "Song",
        },
      }),
    );
    expect(reply).toEqual({ type: "ExportStarted", job: "j1" });
    f.mock.tick(16);
    f.mock.tick(16);
    const exportEvents = f.events.flatMap((e) => (e.type === "Export" ? [e.event] : []));
    expect(exportEvents.map((e) => e.type)).toEqual(["Progress", "Progress", "Done"]);
    const done = exportEvents[2]!;
    if (done.type !== "Done" || done.result.type !== "Download") throw new Error("expected a download");
    const dl = done.result.downloads[0]!;
    expect(dl).toMatchObject({ name: "Song.wav", mime: "audio/wav" });
    let bytes = "";
    for (let offset = 0; ; ) {
      const r = (await f.mock.send(cmd("Export", { type: "ReadChunk", token: dl.token, offset, length: 1000 }))) as Extract<ReplyValue, { type: "Bytes" }>;
      const chunk = atob(r.chunk.data);
      bytes += chunk;
      offset += chunk.length;
      if (r.chunk.eof) break;
    }
    expect(bytes.length).toBe(dl.size);
    expect(bytes.slice(0, 4)).toBe("RIFF");
    expect(bytes.slice(8, 12)).toBe("WAVE");
    await f.mock.send(cmd("Export", { type: "Release", token: dl.token }));
    await expect(f.mock.send(cmd("Export", { type: "ReadChunk", token: dl.token, offset: 0, length: 10 }))).rejects.toBeInstanceOf(CommandFailedError);
  });
});
