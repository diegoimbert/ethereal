// Remote engine: the web UI connected to a local `ether-server` (real native engine, null
// audio backend) over WebSocket, next to a second client (this Node process).
//
// start ether-server → web UI boots on its in-browser engine → "Remote" → URL + token →
// the UI now mirrors the server's project → a UI edit reaches the other client and the
// other client's edit reaches the UI → a WAV dropped on the sample browser is uploaded and
// imported → play (the playhead comes from the server) → disconnect → back to the local
// engine. A wrong token is refused.
//
// No sleeps: every step waits on UI or engine state.
import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expect, test, type Page } from "@playwright/test";
import type { Project } from "@/generated";
import { createOnLaunch, createTrack, launch, openEngineServer, openLibrary, playButton } from "./ui";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const TOKEN = `e2e-${Math.random().toString(36).slice(2)}`;

interface Handle {
  state(): { project: Project | null };
}

const project = (page: Page): Promise<Project | null> =>
  page.evaluate(
    () => (window as unknown as { __ether: Handle }).__ether.state().project,
  );

const count = (o: object | undefined) => (o ? Object.keys(o).length : 0);

/** Build `ether-server` and return the binary path (cargo tells where it is). */
function buildServer(): string {
  const out = execFileSync(
    "cargo",
    [
      "build",
      "-p",
      "ether-server",
      "--bin",
      "ether-server",
      "--message-format=json",
    ],
    {
      cwd: repoRoot,
      env: {
        ...process.env,
        CARGO_BUILD_JOBS: process.env.CARGO_BUILD_JOBS ?? "3",
      },
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  for (const line of out.split("\n")) {
    if (!line.startsWith("{")) continue;
    const msg = JSON.parse(line) as {
      reason?: string;
      target?: { name: string };
      executable?: string | null;
    };
    if (
      msg.reason === "compiler-artifact" &&
      msg.target?.name === "ether-server" &&
      msg.executable
    )
      return msg.executable;
  }
  throw new Error("cargo did not report the ether-server binary");
}

let server: ChildProcess | null = null;
let serverUrl = "";
let dataDir = "";

test.beforeAll(async () => {
  test.setTimeout(600_000);
  const bin = buildServer();
  dataDir = mkdtempSync(join(tmpdir(), "ether-remote-e2e-"));
  const child = spawn(
    bin,
    [
      "--port",
      "0",
      "--token",
      TOKEN,
      "--name",
      "e2e-server",
      "--data-dir",
      dataDir,
      "--library",
      join(dataDir, "library"),
    ],
    {
      env: { ...process.env, ETHER_AUDIO: "null", RUST_LOG: "warn" },
      stdio: ["ignore", "pipe", "inherit"],
    },
  );
  server = child;
  serverUrl = await new Promise<string>((resolveUrl, reject) => {
    let buf = "";
    child.stdout!.on("data", (d: Buffer) => {
      buf += d.toString();
      const m = /listening on (ws:\/\/\S+)/.exec(buf);
      if (m) resolveUrl(m[1]!);
    });
    child.on("exit", (code) =>
      reject(new Error(`ether-server exited (${code})`)),
    );
  });
});

test.afterAll(() => {
  server?.kill();
  if (dataDir) rmSync(dataDir, { recursive: true, force: true });
});

/** A second client speaking the protocol directly (Node's WebSocket). */
class Peer {
  private ws!: WebSocket;
  private next = 1;
  private readonly waiting = new Map<
    number,
    (m: { status: string; value?: unknown; error?: unknown }) => void
  >();

  async open(token = TOKEN): Promise<{ type: string; reason?: string }> {
    this.ws = new WebSocket(serverUrl);
    this.ws.binaryType = "arraybuffer";
    await new Promise((r, j) => {
      this.ws.onopen = r;
      this.ws.onerror = j;
    });
    this.ws.send(
      JSON.stringify({ protocol_version: 1, token, client: "e2e peer" }),
    );
    return new Promise((r) => {
      this.ws.onmessage = (ev) => {
        const hello = JSON.parse(String(ev.data)) as {
          type: string;
          reason?: string;
        };
        this.ws.onmessage = (e) => {
          if (typeof e.data !== "string") return;
          const m = JSON.parse(e.data) as {
            kind: string;
            body: { id: number; result: { status: string } };
          };
          if (m.kind === "Reply") this.waiting.get(m.body.id)?.(m.body.result);
        };
        r(hello);
      };
    });
  }

  async send<T = Record<string, unknown>>(
    domain: string,
    command: object,
  ): Promise<T> {
    const id = this.next++;
    const result = await new Promise<{
      status: string;
      value?: unknown;
      error?: unknown;
    }>((r) => {
      this.waiting.set(id, r);
      this.ws.send(
        JSON.stringify({ id, gesture: null, command: { domain, command } }),
      );
    });
    if (result.status !== "Ok")
      throw new Error(`${domain} failed: ${JSON.stringify(result.error)}`);
    return result.value as T;
  }

  close() {
    this.ws.close();
  }
}

/** 0.5 s of a 220 Hz tone, 16-bit mono WAV. */
function wavBytes(): number[] {
  const rate = 48_000;
  const n = rate / 2;
  const b = new DataView(new ArrayBuffer(44 + n * 2));
  const str = (o: number, s: string) =>
    [...s].forEach((c, i) => b.setUint8(o + i, c.charCodeAt(0)));
  str(0, "RIFF");
  b.setUint32(4, 36 + n * 2, true);
  str(8, "WAVEfmt ");
  b.setUint32(16, 16, true);
  b.setUint16(20, 1, true);
  b.setUint16(22, 1, true);
  b.setUint32(24, rate, true);
  b.setUint32(28, rate * 2, true);
  b.setUint16(32, 2, true);
  b.setUint16(34, 16, true);
  str(36, "data");
  b.setUint32(40, n * 2, true);
  for (let i = 0; i < n; i++)
    b.setInt16(
      44 + i * 2,
      Math.round(16000 * Math.sin((2 * Math.PI * 220 * i) / rate)),
      true,
    );
  return [...new Uint8Array(b.buffer)];
}

async function connect(page: Page, token: string) {
  await openEngineServer(page);
  await page.getByLabel("Server address").fill(serverUrl);
  await page.getByLabel("Token").fill(token);
  await page.getByRole("button", { name: "Connect" }).click();
}

test("web UI drives a remote ether-server", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));

  await launch(page);
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect
    .poll(() => project(page).then((p) => p !== null), { timeout: 30_000 })
    .toBe(true);
  const localId = (await project(page))!.id;

  // --- A wrong token is refused; the UI stays on the local engine.
  await connect(page, "wrong");
  await expect(page.getByText("Wrong or missing token.")).toBeVisible();
  await page.getByRole("button", { name: "Done" }).click();
  expect((await project(page))!.id).toBe(localId);

  // --- Connect: the UI now mirrors the server's project.
  await connect(page, TOKEN);
  await expect(page.getByTestId("remote-button")).toHaveText(/e2e-server/);
  // base-131: the server has no project open: the project screen shows, and a project
  // is created there. Wait for that before asking the server for it.
  await createOnLaunch(page, "Remote song");
  await expect
    .poll(async () => (await project(page))?.id ?? localId, { timeout: 20_000 })
    .not.toBe(localId);
  const peer = new Peer();
  expect((await peer.open()).type).toBe("Welcome");
  const serverProject = (
    await peer.send<{ project: Project }>("Project", { type: "Get" })
  ).project;
  await expect
    .poll(async () => (await project(page))?.id)
    .toBe(serverProject.id);
  expect(serverProject.id).not.toBe(localId);

  // --- A UI edit reaches the server (and the other client).
  const baseTracks = count(serverProject.tracks);
  await createTrack(page, "Midi");
  await expect
    .poll(async () =>
      count(
        (await peer.send<{ project: Project }>("Project", { type: "Get" }))
          .project.tracks,
      ),
    )
    .toBe(baseTracks + 1);

  // --- The other client's edit reaches the UI.
  await peer.send("Project", {
    type: "Rename",
    id: serverProject.id,
    name: "Shared remotely",
  });
  await expect(page.getByTestId("project-name")).toHaveText("Shared remotely");

  // --- Drop a WAV on the sample browser: uploaded, then imported into the project.
  await openLibrary(page, null);
  await page.locator('[data-feature="browser"]').evaluate((el, bytes) => {
    const dt = new DataTransfer();
    dt.items.add(
      new File([new Uint8Array(bytes)], "tone.wav", { type: "audio/wav" }),
    );
    el.dispatchEvent(
      new DragEvent("dragover", {
        dataTransfer: dt,
        bubbles: true,
        cancelable: true,
      }),
    );
    el.dispatchEvent(
      new DragEvent("drop", {
        dataTransfer: dt,
        bubbles: true,
        cancelable: true,
      }),
    );
  }, wavBytes());
  await expect
    .poll(
      async () =>
        Object.values((await project(page))?.media ?? {}).map((m) => [
          m.name,
          m.frames,
        ]),
      { timeout: 20_000 },
    )
    .toEqual([["tone.wav", 24_000]]);

  // --- Play on the server: its playhead drives the transport bar.
  await playButton(page).click();
  await expect
    .poll(
      async () =>
        (await page.getByTestId("position-time").textContent())?.trim(),
      { timeout: 10_000 },
    )
    .not.toBe("0:00.000");
  await page.getByRole("button", { name: "Stop" }).first().click();

  // --- Disconnect: back to the in-browser engine and its project.
  await page.getByTestId("remote-button").click();
  await page.getByRole("button", { name: "Disconnect" }).click();
  await expect(page.getByTestId("remote-button")).toHaveCount(0);
  await expect.poll(async () => (await project(page))?.id).toBe(localId);
  peer.close();

  expect(
    errors.filter((e) => /panicked|RuntimeError|unreachable/.test(e)),
  ).toEqual([]);
});
