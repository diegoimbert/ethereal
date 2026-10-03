// external-instrument: the External Instrument / External Audio Effect on the web build (real
// wasm engine + controller). The web has no hardware I/O: the routing panel shows the stored
// routing read-only with a note, ports and measurement reply Unsupported, and the devices stay
// silent. A routing made on another machine (here: sent straight to the controller, as a
// collaborator's edit or an opened project would bring it) is shown as stored,
// and undo restores the previous routing in one step.
// `EXTERNAL_SHOTS=<dir>` also captures the panels (dark 1440×900, and light).
import { expect, test, type Locator, type Page } from "@playwright/test";
import type { Command, ExternalRouting, Project } from "@/generated";
import { addDevice, openDeviceTab, playButton } from "./ui";

interface Handle {
  state(): { project: Project | null };
  meter(track: string): { peak: [number, number] } | null;
}

interface Probe {
  replies: Record<number, { status: string; error?: { code: string } }>;
  post(json: string): void;
}

const shots = process.env.EXTERNAL_SHOTS;

const project = (page: Page): Promise<Project> =>
  page.evaluate(() => (window as unknown as { __ether: Handle }).__ether.state().project!);

const routingOf = async (page: Page, device: string): Promise<ExternalRouting | null> => {
  const k = (await project(page)).devices[device]?.kind;
  return k?.type === "Builtin" && (k.device.type === "ExternalInstrument" || k.device.type === "ExternalAudioEffect") ? k.device.routing : null;
};

const peakOf = (page: Page, track: string): Promise<number> =>
  page.evaluate((t) => {
    const m = (window as unknown as { __ether: Handle }).__ether.meter(t);
    return m ? Math.max(m.peak[0], m.peak[1]) : 0;
  }, track);

/** Deterministic ULIDs: prefix + decimal digits (valid Crockford base32). */
const ulid = (kind: number, n: number) => `01J${kind}${String(n).padStart(22, "0")}`;

async function installProbe(page: Page) {
  await page.evaluate(() => {
    const ep = (
      window as unknown as {
        __etherEngine: { handles(): { controller: Worker } | null; post(json: string): void };
      }
    ).__etherEngine;
    const probe: Probe = { replies: {}, post: (json) => ep.post(json) };
    ep.handles()!.controller.addEventListener("message", (e: MessageEvent<{ type: string; json?: string }>) => {
      if (e.data.type !== "server" || !e.data.json) return;
      for (const m of JSON.parse(e.data.json) as { kind: string; body: Record<string, unknown> }[]) {
        if (m.kind === "Reply") {
          const { id, result } = m.body as { id: number; result: { status: string; error?: { code: string } } };
          probe.replies[id] = result;
        }
      }
    });
    (window as unknown as { __extProbe: Probe }).__extProbe = probe;
  });
}

let nextId = 3_000_000_000;

/** Send a command straight to the controller; resolves with its reply. */
async function send(page: Page, command: Command): Promise<{ status: string; error?: { code: string } }> {
  const id = nextId++;
  await page.evaluate(
    ([id, command]) => {
      (window as unknown as { __extProbe: Probe }).__extProbe.post(JSON.stringify({ id, gesture: null, command }));
    },
    [id, command] as const,
  );
  let reply: { status: string; error?: { code: string } } | undefined;
  await expect
    .poll(
      async () => {
        reply = await page.evaluate((id) => (window as unknown as { __extProbe: Probe }).__extProbe.replies[id], id);
        return reply !== undefined;
      },
      { timeout: 15_000 },
    )
    .toBe(true);
  return reply!;
}

async function open(page: Page, theme: string) {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.addInitScript((t) => localStorage.setItem("eth-theme", t), theme);
  await page.goto("/");
  await expect(playButton(page)).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => project(page).then((p) => p !== null), { timeout: 30_000 }).toBe(true);
  await installProbe(page);
}

async function insert(page: Page, kind: "Midi" | "Audio", name: string, type: string): Promise<{ track: string; device: string; card: Locator }> {
  const track = ulid(kind === "Midi" ? 1 : 2, nextId++);
  expect((await send(page, { domain: "Track", command: { type: "Create", id: track, kind, name, color: null, parent: null, before: null } })).status).toBe("Ok");
  await openDeviceTab(page, name);
  const before = new Set(Object.keys((await project(page)).devices));
  await addDevice(page, type);
  let device = "";
  await expect
    .poll(async () => {
      const p = await project(page);
      device = Object.keys(p.devices).find((d) => !before.has(d) && p.devices[d]!.track === track) ?? "";
      return device !== "";
    })
    .toBe(true);
  const card = page.locator(`section[data-device="${device}"]`);
  await expect(card).toBeVisible();
  return { track, device, card };
}

test("external devices on the web: read-only routing, unsupported hardware, silent", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await open(page, "dark");

  const inst = await insert(page, "Midi", "Hardware Synth", "ExternalInstrument");
  const panel = inst.card.getByTestId("hardware-routing");
  await expect(panel).toBeVisible();
  await expect(panel.getByRole("note")).toContainText("need the desktop app");
  await expect(panel.getByRole("button", { name: /Measure/ })).toBeDisabled();
  await expect(panel.getByRole("combobox", { name: "MIDI output" })).toBeDisabled();
  // The params of the layout: return gain and latency.
  await expect(inst.card.locator('[data-mod-target$=":0"]')).toBeVisible();
  await expect(inst.card.locator('[data-mod-target$=":1"]')).toBeVisible();

  // The engine: no ports, no measurement here.
  expect((await send(page, { domain: "External", command: { type: "ListPorts" } })).error?.code).toBe("Unsupported");
  expect((await send(page, { domain: "External", command: { type: "MeasureLatency", device: inst.device } })).error?.code).toBe("Unsupported");

  // A routing made on another machine: kept in the document, shown as stored.
  const routing: ExternalRouting = { midi_out: "Studio Synth", midi_channel: 3, audio_send: null, audio_return: { first: 0, count: 2 } };
  expect((await send(page, { domain: "External", command: { type: "SetRouting", device: inst.device, routing } })).status).toBe("Ok");
  await expect.poll(() => routingOf(page, inst.device)).toEqual(routing);
  await expect(panel.getByRole("combobox", { name: "MIDI output" })).toHaveText(/^Studio Synth$/);
  await expect(panel.getByRole("combobox", { name: "MIDI channel" })).toContainText("Ch 3");
  await expect(panel.getByRole("combobox", { name: "Audio return" })).toContainText(/In 1\/2/);

  if (shots) {
    await page.screenshot({ path: `${shots}/external-web-dark.png` });
  }

  const fx = await insert(page, "Audio", "Outboard", "ExternalAudioEffect");
  const fxPanel = fx.card.getByTestId("hardware-routing");
  await expect(fxPanel.getByRole("combobox", { name: "Audio send" })).toBeVisible();
  await expect(fxPanel.getByRole("combobox", { name: "MIDI output" })).toHaveCount(0);

  // Playing: nothing comes back on the web (silent instrument).
  await playButton(page).click();
  await page.waitForTimeout(800);
  expect(await peakOf(page, inst.track)).toBe(0);
  await page.getByRole("toolbar", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true }).first().click();

  // Undo: the effect insert, the Outboard track, then the routing in one step.
  for (let i = 0; i < 2; i++) expect((await send(page, { domain: "Edit", command: { type: "Undo" } })).status).toBe("Ok");
  expect(await routingOf(page, inst.device)).toEqual(routing);
  expect((await send(page, { domain: "Edit", command: { type: "Undo" } })).status).toBe("Ok");
  await expect.poll(() => routingOf(page, inst.device)).toEqual({ midi_out: null, midi_channel: 1, audio_send: null, audio_return: null });
  expect(errors).toEqual([]);
});

test("external devices panel (light theme)", async ({ page }) => {
  test.skip(!shots, "set EXTERNAL_SHOTS=<dir> to capture the light theme");
  await open(page, "light");
  const inst = await insert(page, "Midi", "Hardware Synth", "ExternalInstrument");
  await expect(inst.card.getByTestId("hardware-routing")).toBeVisible();
  await page.screenshot({ path: `${shots}/external-web-light.png` });
});
