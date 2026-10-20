// MIDI files on the web build, through the real wasm engine (`Media::ReadMidiFile` /
// `WriteMidiFile`, `ether_media::smf`): a .mid dropped from the OS below the tracks becomes
// one MIDI track per file track, with the file's tempo in a project without clips; "export
// MIDI…" on the track writes a Standard MIDI File that reads back with the same notes.
//
// No sleeps: every step waits on UI or engine state.
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { launch, newProject } from "./ui";

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project> => page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

/** A variable-length quantity (SMF delta times). */
function vlq(n: number): number[] {
  const out = [n & 0x7f];
  for (n >>= 7; n > 0; n >>= 7) out.unshift((n & 0x7f) | 0x80);
  return out;
}

function chunk(type: string, body: number[]): Buffer {
  const head = Buffer.alloc(8);
  head.write(type, 0);
  head.writeUInt32BE(body.length, 4);
  return Buffer.concat([head, Buffer.from(body)]);
}

/** A type 1 file at 480 PPQ: 100 BPM, then "lead" (C4 E4 G4, a beat each) and "bass" (C2, 2 beats). */
function songMid(): Buffer {
  const name = (s: string) => [0x00, 0xff, 0x03, s.length, ...Buffer.from(s)];
  const end = [0x00, 0xff, 0x2f, 0x00];
  const conductor = [0x00, 0xff, 0x51, 0x03, ...[0x09, 0x27, 0xc0], ...end]; // 600000 us = 100 BPM
  const lead = [...name("lead")];
  for (const p of [60, 64, 67]) lead.push(...vlq(0), 0x90, p, 100, ...vlq(480), 0x80, p, 64);
  const bass = [...name("bass"), 0x00, 0x91, 36, 90, ...vlq(960), 0x81, 36, 0];
  const header = Buffer.from([0x4d, 0x54, 0x68, 0x64, 0, 0, 0, 6, 0, 1, 0, 3, 0x01, 0xe0]);
  return Buffer.concat([header, chunk("MTrk", conductor), chunk("MTrk", [...lead, ...end]), chunk("MTrk", [...bass, ...end])]);
}

/** Drop OS files on `target` at its center, as a file manager does. */
async function dropFile(target: Locator, file: { name: string; bytes: Buffer }): Promise<void> {
  const box = (await target.boundingBox())!;
  const x = box.x + box.width / 2;
  const y = box.y + box.height / 2;
  await target.evaluate(
    (el, { name, data, x, y }) => {
      const bin = atob(data);
      const bytes = new Uint8Array(bin.length);
      for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
      const dt = new DataTransfer();
      dt.items.add(new File([bytes], name));
      const at = document.elementFromPoint(x, y) ?? el;
      for (const type of ["dragenter", "dragover", "drop"]) {
        at.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt, clientX: x, clientY: y }));
      }
    },
    { name: file.name, data: file.bytes.toString("base64"), x, y },
  );
}

test.beforeEach(async ({ page }) => {
  await launch(page);
  await expect(page.getByRole("button", { name: "play", exact: true })).toBeVisible({ timeout: 30_000 });
});

test("a .mid dropped below the tracks becomes MIDI tracks; export MIDI… writes it back", async ({ page }) => {
  await newProject(page, "Midi file");
  const before = await project(page);
  expect(Object.keys(before.clips)).toHaveLength(0);
  await dropFile(page.locator(".eth-arr__drop-area"), { name: "Song.mid", bytes: songMid() });

  await expect.poll(async () => Object.keys((await project(page)).clips).length, { timeout: 20_000 }).toBe(2);
  const p = await project(page);
  const lead = Object.values(p.tracks).find((t) => t.name === "lead")!;
  const bass = Object.values(p.tracks).find((t) => t.name === "bass")!;
  expect([lead.kind, bass.kind]).toEqual(["Midi", "Midi"]);
  const leadClip = Object.values(p.clips).find((c) => c.track === lead.id)!;
  const notes = Object.values(p.notes)
    .filter((n) => n.clip === leadClip.id)
    .sort((a, b) => a.start - b.start)
    .map((n) => [n.pitch, n.start, n.duration]);
  expect(notes).toEqual([
    [60, 0, 1],
    [64, 1, 1],
    [67, 2, 1],
  ]);
  // A project without clips takes the file's tempo.
  expect(Object.values(p.tempo_points).find((t) => t.time === 0)!.bpm).toBeCloseTo(100, 3);
  // One undo step removes it all.
  await page.getByTestId("arrangement-content").click({ position: { x: 5, y: 5 } });
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(async () => Object.keys((await project(page)).clips).length).toBe(0);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(async () => Object.keys((await project(page)).clips).length).toBe(2);

  // Export the lead track.
  const download = page.waitForEvent("download");
  await page.getByRole("group", { name: "lead track" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "export MIDI…" }).click();
  const file = await (await download).path();
  const { readFileSync } = await import("node:fs");
  const bytes = readFileSync(file);
  expect(bytes.subarray(0, 4).toString()).toBe("MThd");
  // Type 1, a tempo track and the lead track.
  expect([bytes.readUInt16BE(8), bytes.readUInt16BE(10)]).toEqual([1, 2]);
  expect(bytes.includes(Buffer.from("lead"))).toBe(true);
});
