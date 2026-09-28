// Importing audio in a collab session (file-import, CONTRACTS.md §12.13): two browser
// contexts, each with its own in-browser engine, on a local `ether-collab-relay`. A file
// dropped on one site is uploaded to that site's engine, which pushes its bytes to the
// session: the other site gets the media, the clip, and hears it (its meter moves).
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { playButton } from "./ui";

/** A mono 16-bit WAV: `seconds` of a 220 Hz sine at 48 kHz. */
function sineWav(seconds: number): Buffer {
  const rate = 48000;
  const frames = Math.round(rate * seconds);
  const b = Buffer.alloc(44 + frames * 2);
  b.write("RIFF", 0);
  b.writeUInt32LE(36 + frames * 2, 4);
  b.write("WAVEfmt ", 8);
  b.writeUInt32LE(16, 16);
  b.writeUInt16LE(1, 20);
  b.writeUInt16LE(1, 22);
  b.writeUInt32LE(rate, 24);
  b.writeUInt32LE(rate * 2, 28);
  b.writeUInt16LE(2, 32);
  b.writeUInt16LE(16, 34);
  b.write("data", 36);
  b.writeUInt32LE(frames * 2, 40);
  for (let i = 0; i < frames; i++) b.writeInt16LE(Math.round(12000 * Math.sin((2 * Math.PI * 220 * i) / rate)), 44 + i * 2);
  return b;
}

interface DropFile {
  name: string;
  bytes: Buffer;
}

/** Drop OS files on `target` at (`dx`, `dy`) from its top-left (default: its center), as a file manager does. */
async function dropFiles(target: Locator, files: DropFile[], at?: { dx: number; dy: number }): Promise<void> {
  const box = (await target.boundingBox())!;
  const x = box.x + (at?.dx ?? box.width / 2);
  const y = box.y + (at?.dy ?? box.height / 2);
  const payload = files.map((f) => ({ name: f.name, data: f.bytes.toString("base64") }));
  await target.evaluate(
    (el, { payload, x, y }) => {
      const dt = new DataTransfer();
      for (const f of payload) {
        const bin = atob(f.data);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        dt.items.add(new File([bytes], f.name));
      }
      const at = document.elementFromPoint(x, y) ?? el;
      for (const type of ["dragenter", "dragover", "drop"]) {
        at.dispatchEvent(new DragEvent(type, { bubbles: true, cancelable: true, dataTransfer: dt, clientX: x, clientY: y }));
      }
    },
    { payload, x, y },
  );
}

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;
const SESSION = `imp-${Math.random().toString(36).slice(2, 8)}`;

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project);

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** Build `ether-collab-relay` and return the binary path (cargo tells where it is). */
function buildRelay(): string {
  const out = execFileSync("cargo", ["build", "-p", "ether-collab", "--bin", "ether-collab-relay", "--message-format=json"], {
    cwd: repoRoot,
    env: { ...process.env, CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3" },
    encoding: "utf8",
    maxBuffer: 256 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as { reason?: string; target?: { name: string }; executable?: string | null };
    if (msg.reason === "compiler-artifact" && msg.target?.name === "ether-collab-relay" && msg.executable) return msg.executable;
  }
  throw new Error("cargo did not report the ether-collab-relay binary");
}

let relay: ChildProcess | null = null;
let relayUrl = "";

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const child = spawn(buildRelay(), ["--port", "0", "--token", TOKEN], {
    env: { ...process.env, RUST_LOG: "warn" },
    stdio: ["ignore", "pipe", "inherit"],
  });
  relay = child;
  relayUrl = await new Promise<string>((resolveUrl, reject) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) resolveUrl(m[1]!);
    });
    child.on("exit", (code) => reject(new Error(`ether-collab-relay exited (${code})`)));
  });
});

test.afterAll(() => {
  relay?.kill();
});

async function open(page: Page): Promise<void> {
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
}

async function join(page: Page, name: string) {
  await page.getByTestId("collab-button").click();
  await page.getByLabel("Relay address").fill(relayUrl);
  await page.getByLabel("Session").fill(SESSION);
  await page.getByLabel("Your name").fill(name);
  await page.getByLabel("Token").fill(TOKEN);
  await page.getByRole("button", { name: "Join" }).click();
  await expect(page.getByTestId("collab-button")).toHaveText(`● ${SESSION}`, { timeout: 20_000 });
  await page.keyboard.press("Escape");
}

test("a file imported on one site plays on the other", async ({ browser }) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  const ctxA = await browser.newContext();
  const ctxB = await browser.newContext();
  const a = await ctxA.newPage();
  const b = await ctxB.newPage();
  for (const p of [a, b]) p.on("pageerror", (e) => errors.push(e.message));
  await open(a);
  await open(b);
  await join(a, "Ada");
  await join(b, "Bob");
  await expect.poll(async () => (await project(b))?.id, { timeout: 20_000 }).toBe((await project(a))!.id);

  // A drops a 4 s file below the tracks (left edge: at the song start): a new track with
  // its clip.
  await dropFiles(a.locator(".eth-arr__drop-area"), [{ name: "Shared Pad.wav", bytes: sineWav(4) }], { dx: 8, dy: 8 });
  const media = async (p: Page) => Object.values((await project(p))?.media ?? {}).find((m) => m.name === "Shared Pad.wav");
  await expect.poll(async () => (await media(a))?.frames ?? 0, { timeout: 20_000 }).toBeGreaterThan(0);

  // B gets the media, its bytes (it decodes them: known length) and the clip.
  await expect.poll(async () => (await media(b))?.frames ?? 0, { timeout: 30_000 }).toBe((await media(a))!.frames);
  const id = (await media(b))!.id;
  const clip = async () => Object.values((await project(b))!.clips).find((c) => c.content.type === "Audio" && c.content.media === id);
  await expect.poll(async () => (await clip()) !== undefined, { timeout: 20_000 }).toBe(true);
  const track = (await clip())!.track;

  // B plays from the song start and hears it.
  expect((await clip())!.start).toBe(0);
  await playButton(b).click();
  await expect.poll(() => peakOf(b, track), { timeout: 20_000 }).toBeGreaterThan(0.05);

  await ctxA.close();
  await ctxB.close();
  expect(errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e))).toEqual([]);
});
