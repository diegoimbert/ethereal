// UI cost of applying patches to a big document and re-reading it: the immer apply of a
// one-note edit, a bulk patch (8k notes), and `notesOfClip` right after (its group-by is
// keyed on the notes table). Prints timings; asserts nothing about them (machine-dependent).
// Default 16k notes; `ETHER_UI_PERF_NOTES=16384,65536,262144` for the big ones.
import { describe, expect, it } from "vitest";
import type { Note, Patch } from "@/generated";
import { createDemoProject } from "@/transport/mock/demoProject";
import { EMPTY_HISTORY, useProjectStore } from "./projectStore";
import { notesOfClip } from "./selectors";

const store = () => useProjectStore.getState();
const SIZES = (process.env.ETHER_UI_PERF_NOTES ?? "16384").split(",").map(Number);

/** `total` notes over 1024 MIDI clips (copies of the demo's first MIDI clip). */
function bigProject(total: number) {
  const p = createDemoProject();
  const base = Object.values(p.clips).find((c) => c.content.type === "Midi")!;
  const clips: string[] = [];
  for (let c = 0; c < 1024; c++) {
    const id = `01HZZY${c.toString(36).padStart(20, "0")}`.toUpperCase();
    p.clips[id] = { ...base, id, start: base.start + c * 4 };
    clips.push(id);
  }
  const perClip = Math.ceil(total / clips.length);
  for (let n = 0; n < total; n++) {
    const id = `01HZZZ${n.toString(36).padStart(20, "0")}`.toUpperCase();
    const k = n % perClip;
    p.notes[id] = {
      id,
      clip: clips[Math.floor(n / perClip)]!,
      pitch: 36 + (k % 48),
      velocity: 0.8,
      release_velocity: 0.5,
      start: k * 0.25,
      duration: 0.25,
      muted: false,
    } as Note;
  }
  return p;
}

function ms(f: () => void): number {
  const t = performance.now();
  f();
  return performance.now() - t;
}

describe("patch apply cost", () => {
  it("prints the cost of note edits on big documents", () => {
    for (const total of SIZES) {
      store().reset();
      const big = bigProject(total);
      const json = JSON.stringify(big);
      let parsed: ReturnType<typeof bigProject> | null = null;
      const parseMs = ms(() => (parsed = JSON.parse(json) as ReturnType<typeof bigProject>));
      const loadMs = ms(() => store().loadProject(parsed!));
      const groupMs = ms(() => {
        for (const c of Object.keys(store().project!.clips)) notesOfClip(store().project!, c);
      });
      console.log(
        `ui load ${total} notes: JSON ${(json.length / 1e6).toFixed(1)} MB, parse ${parseMs.toFixed(1)} ms, loadProject ${loadMs.toFixed(1)} ms, first notesOfClip of every clip ${groupMs.toFixed(1)} ms`,
      );
      const notes = Object.values(store().project!.notes);
      const one = notes[Math.floor(notes.length / 2)]!;
      const clip = one.clip;
      notesOfClip(store().project!, clip);
      let rev = 0;
      const p = (changes: Patch["changes"]): Patch => ({ revision: ++rev, changes, history: EMPTY_HISTORY });
      const edit: number[] = [];
      const read: number[] = [];
      for (let i = 0; i < 5; i++) {
        edit.push(ms(() => store().applyPatch(p([{ type: "Upsert", entity: { type: "Note", value: { ...one, start: one.start + 0.01 * i } } }]))));
        read.push(ms(() => notesOfClip(store().project!, clip)));
      }
      const bulk = notes.slice(0, 8192).map((n) => ({ type: "Upsert" as const, entity: { type: "Note" as const, value: { ...n, pitch: n.pitch + 1 } } }));
      const bulkMs = ms(() => store().applyPatch(p(bulk)));
      const best = (v: number[]) => Math.min(...v).toFixed(2);
      console.log(
        `ui patch ${total} notes: one-note edit apply ${best(edit)} ms, notesOfClip after it ${best(read)} ms; 8192-note patch ${bulkMs.toFixed(1)} ms`,
      );
      expect(store().project!.notes[one.id]).toBeDefined();
    }
  }, 300_000);
});
