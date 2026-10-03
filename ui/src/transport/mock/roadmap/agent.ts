/**
 * Mock of `Command::Agent` (the agent API, pinned in the agent contract; owned by `ai-chat`).
 * The real registry lives in the controller (`agent-api`); this is a representative subset of
 * its tools over the mock document, so the in-app chat works in `just dev-ui` and in tests.
 *
 * - `ListTools` -> `AgentTools { tools }` (input schemas as JSON text).
 * - `CallTool { name, input }` -> `AgentToolResult { content, is_error }`. A bad tool name,
 *   bad JSON or invalid input is an `is_error` result, never a command error.
 * - Each edit is ONE undo step (`MockHost.applyDocument`), patched like any command.
 *
 * TODO(agent-api): the reply and command shapes are hand-typed until `ui/src/generated` has
 * the Agent domain.
 */

import type { Clip, Command, Note, Project, ReplyValue, Track } from "@/generated";
import { cmd } from "../../cmd";
import { CommandFailedError } from "../../EngineTransport";
import type { MockHost } from "./host";

export type MockAgentCommand = { type: "ListTools" } | { type: "CallTool"; name: string; input: string };

interface ToolDef {
  name: string;
  description: string;
  schema: Record<string, unknown>;
  run(input: Record<string, unknown>): unknown;
}

/** A tool input the model got wrong: reported back as an `is_error` result. */
class ToolInputError extends Error {}

const obj = (properties: Record<string, unknown>, required: string[] = []) => ({
  type: "object",
  properties,
  required,
  additionalProperties: false,
});
const ID = (what: string) => ({ type: "string", description: `${what} id (from get_project_overview)` });
const BEATS = (what: string) => ({ type: "number", minimum: 0, description: `${what}, in beats (quarter notes)` });

/** Most notes a single read returns. */
const NOTE_PAGE = 500;

const KINDS = { midi: "Midi", audio: "Audio", group: "Group", return: "Return" } as const;

function str(input: Record<string, unknown>, key: string, optional?: false): string;
function str(input: Record<string, unknown>, key: string, optional: true): string | undefined;
function str(input: Record<string, unknown>, key: string, optional = false): string | undefined {
  const v = input[key];
  if (v === undefined && optional) return undefined;
  if (typeof v !== "string") throw new ToolInputError(`\`${key}\` must be a string`);
  return v;
}

function num(input: Record<string, unknown>, key: string, opts: { min?: number; max?: number; optional?: boolean } = {}): number | undefined {
  const v = input[key];
  if (v === undefined && opts.optional) return undefined;
  if (typeof v !== "number" || !Number.isFinite(v)) throw new ToolInputError(`\`${key}\` must be a number`);
  if (opts.min !== undefined && v < opts.min) throw new ToolInputError(`\`${key}\` must be >= ${opts.min}`);
  if (opts.max !== undefined && v > opts.max) throw new ToolInputError(`\`${key}\` must be <= ${opts.max}`);
  return v;
}

function bool(input: Record<string, unknown>, key: string): boolean | undefined {
  const v = input[key];
  if (v === undefined) return undefined;
  if (typeof v !== "boolean") throw new ToolInputError(`\`${key}\` must be a boolean`);
  return v;
}

const round = (x: number) => Math.round(x * 1000) / 1000;

export class MockAgent {
  private readonly tools: ToolDef[];

  constructor(private readonly host: MockHost) {
    this.tools = this.defineTools();
  }

  command(c: MockAgentCommand): ReplyValue {
    switch (c.type) {
      case "ListTools":
        return {
          type: "AgentTools",
          tools: this.tools.map((t) => ({ name: t.name, description: t.description, input_schema: JSON.stringify(t.schema) })),
        } as unknown as ReplyValue;
      case "CallTool":
        return { type: "AgentToolResult", ...this.call(c.name, c.input) } as unknown as ReplyValue;
    }
  }

  private call(name: string, inputText: string): { content: string; is_error: boolean } {
    const tool = this.tools.find((t) => t.name === name);
    if (!tool) return { content: `Unknown tool \`${name}\`. Call one of: ${this.tools.map((t) => t.name).join(", ")}.`, is_error: true };
    let input: unknown;
    try {
      input = inputText.trim() ? JSON.parse(inputText) : {};
    } catch {
      return { content: "The input is not valid JSON.", is_error: true };
    }
    if (typeof input !== "object" || input === null || Array.isArray(input)) {
      return { content: "The input must be a JSON object.", is_error: true };
    }
    try {
      const out = tool.run(input as Record<string, unknown>);
      return { content: typeof out === "string" ? out : JSON.stringify(out), is_error: false };
    } catch (e) {
      if (e instanceof ToolInputError) return { content: e.message, is_error: true };
      if (e instanceof CommandFailedError) return { content: e.error.message, is_error: true };
      return { content: String(e), is_error: true };
    }
  }

  // ─── Reads ──────────────────────────────────────────────────────────────────────────────

  private get project(): Project {
    return this.host.project();
  }

  private track(id: string): Track {
    return this.project.tracks[id] ?? fail(`No track with id \`${id}\`.`);
  }

  private midiClip(id: string): Clip {
    const clip = this.project.clips[id] ?? fail(`No clip with id \`${id}\`.`);
    if (clip.content.type !== "Midi") fail(`Clip \`${id}\` is an audio clip; notes need a MIDI clip.`);
    return clip;
  }

  private tracksInOrder(): Track[] {
    return Object.values(this.project.tracks).sort((a, b) => (a.order < b.order ? -1 : a.order > b.order ? 1 : 0));
  }

  private trackSummary(t: Track) {
    const p = this.project;
    const devices = Object.values(p.devices)
      .filter((d) => d.track === t.id && d.pad === null && !d.chain)
      .sort((a, b) => (a.order < b.order ? -1 : 1))
      .map((d) => ({ id: d.id, name: d.name, enabled: d.enabled }));
    const clips = Object.values(p.clips)
      .filter((c) => c.track === t.id && !c.lane)
      .sort((a, b) => a.start - b.start)
      .map((c) => ({
        id: c.id,
        name: c.name,
        kind: c.content.type.toLowerCase(),
        start_beats: round(c.start),
        length_beats: round(c.length),
        ...(c.content.type === "Midi" ? { notes: Object.values(p.notes).filter((n) => n.clip === c.id).length } : {}),
      }));
    return {
      id: t.id,
      name: t.name,
      kind: t.kind.toLowerCase(),
      parent: t.parent,
      volume_db: round(t.mixer.volume),
      pan: round(t.mixer.pan),
      mute: t.mixer.mute,
      solo: t.mixer.solo,
      devices,
      clips,
    };
  }

  private overview() {
    const p = this.project;
    const tempo = Object.values(p.tempo_points).sort((a, b) => a.time - b.time)[0];
    const sig = Object.values(p.time_signatures).sort((a, b) => a.time - b.time)[0];
    return {
      name: p.settings.name,
      tempo_bpm: tempo?.bpm ?? 120,
      time_signature: sig ? `${sig.signature.numerator}/${sig.signature.denominator}` : "4/4",
      loop: { enabled: p.settings.loop_enabled, start_beats: p.settings.loop_region.start, end_beats: p.settings.loop_region.end },
      tracks: this.tracksInOrder().map((t) => this.trackSummary(t)),
    };
  }

  private notesOf(clip: string): Note[] {
    return Object.values(this.project.notes)
      .filter((n) => n.clip === clip)
      .sort((a, b) => a.start - b.start || a.pitch - b.pitch);
  }

  // ─── Edits ──────────────────────────────────────────────────────────────────────────────

  private edit(label: string, commands: Command[]): void {
    this.host.applyDocument(commands, label);
  }

  private defineTools(): ToolDef[] {
    return [
      {
        name: "get_project_overview",
        description:
          "Compact overview of the open project: name, tempo (BPM), time signature, loop, and every track in order with its id, kind, mixer (volume in dB, pan -1..1, mute, solo), devices and clips (start/length in beats, note counts). Call this first to learn ids.",
        schema: obj({}),
        run: () => this.overview(),
      },
      {
        name: "get_track",
        description: "One track in detail: mixer, devices and clips (positions in beats).",
        schema: obj({ track_id: ID("Track") }, ["track_id"]),
        run: (i) => this.trackSummary(this.track(str(i, "track_id"))),
      },
      {
        name: "get_clip_notes",
        description: `Notes of a MIDI clip: pitch (MIDI 0-127, 60 = middle C), start and duration in beats relative to the clip start, velocity 1-127. At most ${NOTE_PAGE} notes per call; pass \`offset\` for the next page.`,
        schema: obj({ clip_id: ID("MIDI clip"), offset: { type: "integer", minimum: 0, description: "Notes to skip (pagination)" } }, ["clip_id"]),
        run: (i) => {
          const clip = this.midiClip(str(i, "clip_id"));
          const offset = num(i, "offset", { min: 0, optional: true }) ?? 0;
          const all = this.notesOf(clip.id);
          const page = all.slice(offset, offset + NOTE_PAGE).map((n) => ({
            id: n.id,
            pitch: n.pitch,
            start_beats: round(n.start),
            duration_beats: round(n.duration),
            velocity: Math.round(n.velocity * 127),
          }));
          return { total: all.length, offset, notes: page, ...(offset + NOTE_PAGE < all.length ? { note: `Truncated: call again with offset ${offset + NOTE_PAGE}.` } : {}) };
        },
      },
      {
        name: "create_track",
        description: "Create a track at the end of the arrangement. A MIDI track gets a default Synth instrument so its notes are audible. Returns the new track id.",
        schema: obj(
          { kind: { type: "string", enum: Object.keys(KINDS), description: "Track type" }, name: { type: "string", description: "Track name (optional)" } },
          ["kind"],
        ),
        run: (i) => {
          const kindKey = str(i, "kind");
          const kind = KINDS[kindKey as keyof typeof KINDS] ?? fail(`\`kind\` must be one of ${Object.keys(KINDS).join(", ")}.`);
          const name = str(i, "name", true)?.trim() || null;
          const id = this.host.newId();
          const commands: Command[] = [cmd("Track", { type: "Create", id, kind, name, color: null, parent: null, before: null })];
          if (kind === "Midi") {
            commands.push(
              cmd("Device", { type: "Insert", id: this.host.newId(), track: id, device: { type: "Builtin", device: { type: "Synth" } }, before: null }),
            );
          }
          this.edit("AI: Create Track", commands);
          return { track_id: id, name: this.track(id).name };
        },
      },
      {
        name: "delete_track",
        description: "Delete a track with its clips and devices.",
        schema: obj({ track_id: ID("Track") }, ["track_id"]),
        run: (i) => {
          const t = this.track(str(i, "track_id"));
          if (t.kind === "Master") fail("The master track cannot be deleted.");
          this.edit("AI: Delete Track", [cmd("Track", { type: "Delete", id: t.id })]);
          return { deleted: t.id };
        },
      },
      {
        name: "rename_track",
        description: "Rename a track.",
        schema: obj({ track_id: ID("Track"), name: { type: "string" } }, ["track_id", "name"]),
        run: (i) => {
          const t = this.track(str(i, "track_id"));
          const name = str(i, "name").trim() || fail("`name` is empty.");
          this.edit("AI: Rename Track", [cmd("Track", { type: "Rename", id: t.id, name })]);
          return { track_id: t.id, name };
        },
      },
      {
        name: "set_track_mix",
        description: "Set a track's mixer: volume in dB (-inf..+6, 0 = unity), pan -1 (left) .. 1 (right), mute, solo. Only the given fields change.",
        schema: obj(
          {
            track_id: ID("Track"),
            volume_db: { type: "number", maximum: 6, description: "Fader level in dB" },
            pan: { type: "number", minimum: -1, maximum: 1 },
            mute: { type: "boolean" },
            solo: { type: "boolean" },
          },
          ["track_id"],
        ),
        run: (i) => {
          const t = this.track(str(i, "track_id"));
          const commands: Command[] = [];
          const volume = num(i, "volume_db", { max: 6, optional: true });
          const pan = num(i, "pan", { min: -1, max: 1, optional: true });
          const mute = bool(i, "mute");
          const solo = bool(i, "solo");
          if (volume !== undefined) commands.push(cmd("Mixer", { type: "SetVolume", track: t.id, volume }));
          if (pan !== undefined) commands.push(cmd("Mixer", { type: "SetPan", track: t.id, pan }));
          if (mute !== undefined) commands.push(cmd("Mixer", { type: "SetMute", track: t.id, mute }));
          if (solo !== undefined) commands.push(cmd("Mixer", { type: "SetSolo", track: t.id, solo, exclusive: false }));
          if (!commands.length) fail("Nothing to change: pass volume_db, pan, mute or solo.");
          this.edit("AI: Track Mix", commands);
          return this.trackSummary(this.track(t.id));
        },
      },
      {
        name: "create_midi_clip",
        description: "Create an empty MIDI clip on a MIDI track. Returns the clip id; add notes with add_notes.",
        schema: obj(
          { track_id: ID("MIDI track"), start_beats: BEATS("Clip start"), length_beats: { type: "number", exclusiveMinimum: 0, description: "Clip length, in beats" }, name: { type: "string" } },
          ["track_id", "start_beats", "length_beats"],
        ),
        run: (i) => {
          const t = this.track(str(i, "track_id"));
          if (t.kind !== "Midi") fail(`Track \`${t.name}\` is not a MIDI track.`);
          const start = num(i, "start_beats", { min: 0 })!;
          const length = num(i, "length_beats", { min: 1e-6 })!;
          const id = this.host.newId();
          this.edit("AI: Create MIDI Clip", [cmd("Clip", { type: "CreateMidi", id, track: t.id, start, length, name: str(i, "name", true) ?? null })]);
          return { clip_id: id };
        },
      },
      {
        name: "add_notes",
        description:
          "Add notes to a MIDI clip. pitch: MIDI 0-127 (60 = middle C / C4); start_beats: relative to the clip start; duration_beats > 0; velocity 1-127 (default 100). Returns the new note ids.",
        schema: obj(
          {
            clip_id: ID("MIDI clip"),
            notes: {
              type: "array",
              minItems: 1,
              items: obj(
                {
                  pitch: { type: "integer", minimum: 0, maximum: 127 },
                  start_beats: BEATS("Start relative to the clip"),
                  duration_beats: { type: "number", exclusiveMinimum: 0 },
                  velocity: { type: "integer", minimum: 1, maximum: 127 },
                },
                ["pitch", "start_beats", "duration_beats"],
              ),
            },
          },
          ["clip_id", "notes"],
        ),
        run: (i) => {
          const clip = this.midiClip(str(i, "clip_id"));
          const raw = i.notes;
          if (!Array.isArray(raw) || raw.length === 0) fail("`notes` must be a non-empty array.");
          const notes = raw.map((n: unknown, k) => {
            if (typeof n !== "object" || n === null) fail(`notes[${k}] must be an object.`);
            const o = n as Record<string, unknown>;
            const pitch = num(o, "pitch", { min: 0, max: 127 })!;
            return {
              id: this.host.newId(),
              pitch: Math.round(pitch),
              start: num(o, "start_beats", { min: 0 })!,
              duration: num(o, "duration_beats", { min: 1e-6 })!,
              velocity: (num(o, "velocity", { min: 1, max: 127, optional: true }) ?? 100) / 127,
            };
          });
          this.edit("AI: Add Notes", [cmd("Note", { type: "Add", clip: clip.id, notes })]);
          return { note_ids: notes.map((n) => n.id) };
        },
      },
      {
        name: "remove_notes",
        description: "Remove notes by id (ids from get_clip_notes).",
        schema: obj({ note_ids: { type: "array", minItems: 1, items: { type: "string" } } }, ["note_ids"]),
        run: (i) => {
          const ids = i.note_ids;
          if (!Array.isArray(ids) || !ids.length || ids.some((x) => typeof x !== "string")) fail("`note_ids` must be a non-empty array of ids.");
          const missing = (ids as string[]).filter((id) => !this.project.notes[id]);
          if (missing.length) fail(`Unknown note ids: ${missing.join(", ")}.`);
          this.edit("AI: Remove Notes", [cmd("Note", { type: "Remove", ids: ids as string[] })]);
          return { removed: ids.length };
        },
      },
      {
        name: "set_tempo",
        description: "Set the project tempo, in BPM (20-999).",
        schema: obj({ bpm: { type: "number", minimum: 20, maximum: 999 } }, ["bpm"]),
        run: (i) => {
          const bpm = num(i, "bpm", { min: 20, max: 999 })!;
          this.edit("AI: Set Tempo", [cmd("Transport", { type: "SetTempo", bpm })]);
          return { tempo_bpm: bpm };
        },
      },
      {
        name: "transport",
        description: "Control playback: play, stop, or seek to a position in beats.",
        schema: obj({ action: { type: "string", enum: ["play", "stop", "seek"] }, position_beats: BEATS("Seek target") }, ["action"]),
        run: (i) => {
          const action = str(i, "action");
          if (action === "play") this.host.execute(cmd("Transport", { type: "Play" }));
          else if (action === "stop") this.host.execute(cmd("Transport", { type: "Stop" }));
          else if (action === "seek") this.host.execute(cmd("Transport", { type: "Locate", position: num(i, "position_beats", { min: 0 })! }));
          else fail("`action` must be play, stop or seek.");
          return { ok: true };
        },
      },
      {
        name: "undo",
        description: "Undo the last edit (the user's or yours).",
        schema: obj({}),
        run: () => {
          this.host.execute(cmd("Edit", { type: "Undo" }));
          return { ok: true };
        },
      },
      {
        name: "redo",
        description: "Redo the last undone edit.",
        schema: obj({}),
        run: () => {
          this.host.execute(cmd("Edit", { type: "Redo" }));
          return { ok: true };
        },
      },
    ];
  }
}

function fail(message: string): never {
  throw new ToolInputError(message);
}
