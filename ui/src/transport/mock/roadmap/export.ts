/**
 * Mock of `Export::*` (`MockExports`): `Render` replies `ExportStarted`, then each playhead
 * step (`MockTransport.tick()` with manual timers) advances one phase: Progress 0.5, then
 * Progress 1.0 + Done with `Download` results (a small silent WAV per file) readable with
 * `ReadChunk`. Validation, file naming and the chunk cap mirror the Rust controller
 * (`ether-controller/src/export`). Owned by `export`.
 */

import type {
  ExportCommand,
  ExportDownload,
  ExportRange,
  ExportRequest,
  Project,
  ReplyValue,
} from "@/generated";
import { fail } from "../documentReducer";
import type { MockHost } from "./host";

const UNIT: ReplyValue = { type: "Unit" };

export class MockExports {
  private job: { job: string; request: ExportRequest; phase: number } | null =
    null;
  private readonly downloads = new Map<
    string,
    { info: ExportDownload; bytes: Uint8Array }
  >();

  constructor(private readonly host: MockHost) {}

  command(c: ExportCommand): ReplyValue {
    switch (c.type) {
      case "Render": {
        if (this.job) fail("InvalidState", "an export is already running");
        const r = c.request;
        const project = this.host.project();
        if (r.format.container === "Flac" && r.format.bit_depth === "Float32")
          fail("InvalidArgument", "FLAC supports 16 or 24 bits only");
        const rate = r.format.sample_rate;
        if (rate !== null && !(rate >= 8000 && rate <= 384000))
          fail("InvalidArgument", `unsupported sample rate ${rate} Hz`);
        if (!(Number.isFinite(r.tail_seconds) && r.tail_seconds >= 0))
          fail("InvalidArgument", "tail must be >= 0 seconds");
        const [start, end] = rangeOf(project, r.range);
        if (!(Number.isFinite(start) && Number.isFinite(end) && start >= 0))
          fail("InvalidArgument", "invalid export range");
        if (!(end > start + 1e-9))
          fail("InvalidArgument", "nothing to export: the range is empty");
        if (r.mode.type === "Stems") {
          if (r.mode.tracks.length === 0)
            fail("InvalidArgument", "no tracks selected for stems");
          for (const t of r.mode.tracks)
            if (!project.tracks[t]) fail("NotFound", `track ${t} not found`);
        }
        // Previous results are dropped by a new render.
        this.downloads.clear();
        this.job = { job: c.job, request: r, phase: 0 };
        return { type: "ExportStarted", job: c.job };
      }
      case "Cancel":
        if (this.job?.job === c.job) {
          this.job = null;
          this.host.emit({
            type: "Export",
            event: { type: "Cancelled", job: c.job },
          });
        }
        return UNIT;
      case "ReadChunk": {
        const d =
          this.downloads.get(c.token) ??
          fail("NotFound", `download ${c.token}`);
        const start = Math.max(0, Math.floor(c.offset));
        const end = Math.min(
          d.bytes.length,
          start + Math.min(c.length, MAX_CHUNK_BYTES),
        );
        let bin = "";
        for (let i = start; i < end; i++)
          bin += String.fromCharCode(d.bytes[i]!);
        return {
          type: "Bytes",
          chunk: { offset: start, data: btoa(bin), eof: end >= d.bytes.length },
        };
      }
      case "Release":
        this.downloads.delete(c.token);
        return UNIT;
    }
  }

  /** Advance the running export by one phase (called on every playhead step). */
  step(): void {
    const job = this.job;
    if (!job) return;
    job.phase += 1;
    if (job.phase === 1) {
      this.host.emit({
        type: "Export",
        event: { type: "Progress", job: job.job, progress: 0.5 },
      });
      return;
    }
    this.job = null;
    const project = this.host.project();
    const r = job.request;
    const ext = r.format.container === "Flac" ? "flac" : "wav";
    const base = sanitize(r.name ?? project.settings.name);
    const used = new Set<string>();
    const unique = (stem: string) => {
      let name = `${stem}.${ext}`;
      for (let i = 2; used.has(name.toLowerCase()); i++)
        name = `${stem} (${i}).${ext}`;
      used.add(name.toLowerCase());
      return name;
    };
    const names =
      r.mode.type === "Mix"
        ? [unique(base)]
        : [...new Set(r.mode.tracks)].map((t) =>
            unique(sanitize(`${base} - ${project.tracks[t]?.name ?? "Track"}`)),
          );
    const bits =
      r.format.bit_depth === "Int16"
        ? 16
        : r.format.bit_depth === "Int24"
          ? 24
          : 32;
    const downloads = names.map((name) => {
      const bytes = silentWav(r.format.sample_rate ?? 48000, bits);
      const info: ExportDownload = {
        token: this.host.newId(),
        name,
        mime: `audio/${ext}`,
        size: bytes.length,
      };
      this.downloads.set(info.token, { info, bytes });
      return info;
    });
    this.host.emit({
      type: "Export",
      event: { type: "Progress", job: job.job, progress: 1 },
    });
    this.host.emit({
      type: "Export",
      event: {
        type: "Done",
        job: job.job,
        result: { type: "Download", downloads },
      },
    });
  }
}

/** Most bytes one `ReadChunk` serves (same cap as the Rust controller). */
const MAX_CHUNK_BYTES = 1 << 20;

/** Timeline range of a request in beats (mirrors `export::job::range_of`). */
function rangeOf(project: Project, range: ExportRange): [number, number] {
  switch (range.type) {
    case "Loop":
      return [
        project.settings.loop_region.start,
        project.settings.loop_region.end,
      ];
    case "Custom":
      return [range.start, range.end];
    case "Project": {
      let end = 0;
      for (const c of Object.values(project.clips))
        end = Math.max(end, c.start + c.length);
      for (const pt of Object.values(project.automation_points)) {
        if (project.automation_lanes[pt.lane]?.owner.type === "Track")
          end = Math.max(end, pt.time);
      }
      return [0, end];
    }
  }
}

/** A safe single file-name segment (mirrors `export::job::sanitize`). */
export function sanitize(name: string): string {
  // eslint-disable-next-line no-control-regex
  const cleaned = name.replace(/[\u0000-\u001f\u007f-\u009f/\\:*?"<>|]/g, "_");
  let s = cleaned.trim().replace(/^\.+/, "").trim();
  s = [...s]
    .slice(0, 120)
    .join("")
    .replace(/[. ]+$/, "");
  return s === "" ? "Export" : s;
}

/** 100 ms of stereo silence as a WAV file (PCM, or IEEE float for 32 bits). */
function silentWav(sampleRate: number, bits: 16 | 24 | 32): Uint8Array {
  const channels = 2;
  const frames = Math.round(sampleRate / 10);
  const bytesPerSample = bits / 8;
  const dataSize = frames * channels * bytesPerSample;
  const buf = new ArrayBuffer(44 + dataSize);
  const v = new DataView(buf);
  const str = (off: number, s: string) => {
    for (let i = 0; i < s.length; i++) v.setUint8(off + i, s.charCodeAt(i));
  };
  str(0, "RIFF");
  v.setUint32(4, 36 + dataSize, true);
  str(8, "WAVE");
  str(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, bits === 32 ? 3 : 1, true);
  v.setUint16(22, channels, true);
  v.setUint32(24, sampleRate, true);
  v.setUint32(28, sampleRate * channels * bytesPerSample, true);
  v.setUint16(32, channels * bytesPerSample, true);
  v.setUint16(34, bits, true);
  str(36, "data");
  v.setUint32(40, dataSize, true);
  return new Uint8Array(buf);
}
