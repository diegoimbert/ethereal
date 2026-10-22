/** MockTransport: export (export). */
import { describe, expect, it } from "vitest";
import type { ExportRequest, ReplyValue } from "@/generated";
import { cmd } from "../../cmd";
import { CommandFailedError } from "../../EngineTransport";
import { sanitize } from "./export";
import { project, useMock } from "./testUtils";

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
            format: {
              container: "Flac",
              bit_depth: "Float32",
              sample_rate: null,
            },
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
    for (let offset = 0; ;) {
      const r = (await f.mock.send(
        cmd("Export", {
          type: "ReadChunk",
          token: dl.token,
          offset,
          length: 1000,
        }),
      )) as Extract<ReplyValue, { type: "Bytes" }>;
      const chunk = atob(r.chunk.data);
      bytes += chunk;
      offset += chunk.length;
      if (r.chunk.eof) break;
    }
    expect(bytes.length).toBe(dl.size);
    expect(bytes.slice(0, 4)).toBe("RIFF");
    expect(bytes.slice(8, 12)).toBe("WAVE");
    await f.mock.send(cmd("Export", { type: "Release", token: dl.token }));
    await expect(
      f.mock.send(
        cmd("Export", {
          type: "ReadChunk",
          token: dl.token,
          offset: 0,
          length: 10,
        }),
      ),
    ).rejects.toBeInstanceOf(CommandFailedError);
  });

  it("export: validation and file names mirror the controller", async () => {
    const req = (patch: Partial<ExportRequest>): ExportRequest => ({
      range: { type: "Custom", start: 0, end: 4 },
      format: { container: "Wav", bit_depth: "Int16", sample_rate: null },
      mode: { type: "Mix" },
      normalize: false,
      tail_seconds: 0,
      name: null,
      ...patch,
    });
    const render = (job: string, request: ExportRequest) => f.mock.send(cmd("Export", { type: "Render", job, request }));
    for (const bad of [
      req({ range: { type: "Custom", start: 4, end: 4 } }),
      req({ tail_seconds: -1 }),
      req({
        format: { container: "Wav", bit_depth: "Int16", sample_rate: 1000 },
      }),
      req({ mode: { type: "Stems", tracks: [] } }),
    ]) {
      await expect(render("bad", bad)).rejects.toMatchObject({
        code: "InvalidArgument",
      });
    }
    const t = Object.values(project(f).tracks).find((x) => x.kind !== "Master")!;
    await render("j", req({ name: "a/b", mode: { type: "Stems", tracks: [t.id, t.id] } }));
    f.mock.tick(16);
    f.mock.tick(16);
    const done = f.events.flatMap((e) => (e.type === "Export" && e.event.type === "Done" ? [e.event] : []))[0]!;
    if (done.result.type !== "Download") throw new Error("expected a download");
    expect(done.result.downloads.map((d) => d.name)).toEqual([`${sanitize(`a_b - ${t.name}`)}.wav`]);
    expect(sanitize("..x/y.")).toBe("x_y");
    expect(sanitize(" ")).toBe("Export");
  });

  it("pauses the engine while rendering: transport.rendering grows, locate clamped, record refused", async () => {
    const rendering = () => {
      const states = f.events.flatMap((e) => (e.type === "Transport" ? [e.state] : []));
      return states[states.length - 1]?.rendering ?? null;
    };
    await f.mock.send(
      cmd("Export", {
        type: "Render",
        job: "r",
        request: {
          range: { type: "Custom", start: 2, end: 10 },
          format: { container: "Wav", bit_depth: "Int16", sample_rate: null },
          mode: { type: "Mix" },
          normalize: false,
          tail_seconds: 0,
          name: null,
        },
      }),
    );
    expect(rendering()).toEqual({ start: 2, end: 10, until: 2, progress: 0, preview: true });
    await expect(f.mock.send(cmd("Recording", { type: "SetRecording", enabled: true }))).rejects.toMatchObject({ code: "InvalidState" });
    f.mock.tick(16);
    expect(rendering()).toMatchObject({ until: 6, progress: 0.5 });
    await f.mock.send(cmd("Transport", { type: "Locate", position: 9 }));
    expect(f.mock.playheadPosition).toBe(6);
    await f.mock.send(cmd("Transport", { type: "Locate", position: 0 }));
    expect(f.mock.playheadPosition).toBe(2);
    f.mock.tick(16);
    expect(rendering()).toBeNull();
    await f.mock.send(cmd("Transport", { type: "Locate", position: 9 }));
    expect(f.mock.playheadPosition).toBe(9);
  });

  it("a cancelled render resumes the engine", async () => {
    const request: ExportRequest = {
      range: { type: "Custom", start: 0, end: 4 },
      format: { container: "Wav", bit_depth: "Int16", sample_rate: null },
      mode: { type: "Mix" },
      normalize: false,
      tail_seconds: 0,
      name: null,
    };
    await f.mock.send(cmd("Export", { type: "Render", job: "c", request }));
    await f.mock.send(cmd("Export", { type: "Cancel", job: "c" }));
    const states = f.events.flatMap((e) => (e.type === "Transport" ? [e.state] : []));
    expect(states.at(-2)?.rendering).toBeTruthy();
    expect(states.at(-1)?.rendering).toBeUndefined();
  });
});
