import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Device, MissingPlugin, PluginDescriptor } from "@/generated";
import { useContextMenuStore, type ContextMenuItem } from "@/kit";
import { useEditorStore, useProjectStore, useSelectionStore } from "@/state";
import { cmd, MockTransport, newId, TransportProvider, type EngineTransport } from "@/transport";
import { libraryPath } from "@/transport/mock/roadmap/mediaReferences";
import type { ChatSession, LoopEvent } from "@/features/ai-chat/providers";
import { reloadAiSettings, useAiSettings } from "@/features/ai-chat/settings";
import { applyMediaRefEvent, MissingMediaNotice, openRelink, RelinkDialog, resetMediaRefs } from "@/features/media-refs";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { PluginDeviceControls } from "@/features/plugins";
import { DeviceChain } from "@/features/devices";
import { WarpEditor } from "@/features/warp";
import { MOCK_MISSING_PLUGIN_ID } from "@/transport/mock/roadmap/resolver";
import { AI_CONSENT_STORAGE, AiResponseError, buildAiPayload, parseAiResponse, runAi } from "./ai";
import { isMissingPlugin } from "./catalog";
import { resolveClipEntries, resolvePluginEntries, resolverCommands, withResolveClipEntries } from "./menus";
import { ResolverDialog } from "./ResolverDialog";
import { applyResolution, applyResolveEvent, askAi, openResolver, resetResolver, resolutions, setResolverTransport, useResolver } from "./store";

let mock: MockTransport | undefined;

beforeAll(() => {
  const proto = HTMLElement.prototype as unknown as Record<string, unknown>;
  proto.setPointerCapture ??= () => {};
  proto.releasePointerCapture ??= () => {};
  proto.hasPointerCapture ??= () => false;
});

beforeEach(() => {
  localStorage.clear();
  reloadAiSettings();
  resetResolver();
  resetMediaRefs();
});

afterEach(() => {
  cleanup();
  mock?.dispose();
  mock = undefined;
  setResolverTransport(null);
  useProjectStore.getState().reset();
  useSelectionStore.getState().selectTrack(null);
  usePluginStore.getState().reset();
  useContextMenuStore.setState({ menu: null });
});

const project = () => useProjectStore.getState().project!;

/** The mock engine with its demo project, plus a device of an uninstalled plugin. */
async function setup(ui: React.ReactNode = null): Promise<{ device: Device; t: MockTransport }> {
  const t = new MockTransport({ timers: "manual", seed: 1 });
  mock = t;
  render(
    <TransportProvider transport={t}>
      {ui}
      <ResolverDialog transport={t} />
    </TransportProvider>,
  );
  await waitFor(() => expect(useProjectStore.getState().project).not.toBeNull());
  setResolverTransport(t);
  t.onEvent(applyResolveEvent);
  const track = Object.values(project().tracks).find((x) => x.kind === "Audio")!;
  const id = t.resolver.addMissingPlugin(track.id);
  await waitFor(() => expect(project().devices[id]).toBeTruthy());
  return { device: project().devices[id]!, t };
}

/** Labels of the open context menu. */
const menuLabels = () => (useContextMenuStore.getState().menu?.items ?? []).map((e) => (e === "separator" ? "—" : e.label));
const menuItem = (label: string) =>
  (useContextMenuStore.getState().menu?.items ?? []).find((e): e is ContextMenuItem => e !== "separator" && e.label === label)!;

describe("resolver with the mock engine", () => {
  it("proposes the deterministic match and applies it in one undo step", async () => {
    const { device, t } = await setup();
    expect(isMissingPlugin(device, [])).toBe(true);
    await act(() => openResolver({ device: device.id }, t));
    const s = useResolver.getState();
    expect(s.open).toBe(true);
    expect(s.plugins).toHaveLength(1);
    expect(s.focus).toBe(`p:Clap:${MOCK_MISSING_PLUGIN_ID}`);
    const item = s.plugins[0]!;
    expect(item.candidates[0]).toMatchObject({ name: "Mock Mega", rule: "SameFamily" });
    expect(item.apply).toBe(true);
    const dialog = await screen.findByRole("dialog", { name: "resolve “Mock Mega 1”" });
    expect(within(dialog).getByText("same family")).toBeTruthy();
    expect(within(dialog).getByText("85%")).toBeTruthy();

    const steps = () => (t as unknown as { undoStack: unknown[] }).undoStack.length;
    const before = steps();
    fireEvent.click(within(dialog).getByRole("button", { name: "apply 1 change" }));
    await waitFor(() => expect(useResolver.getState().open).toBe(false));
    const d = project().devices[device.id]!;
    expect(d.kind.type === "Plugin" && d.kind.plugin.plugin_id).toBe("dev.ethereal.mock.mega");
    expect(d.name).toBe("Mock Mega");
    expect(d.params).toEqual({ 7: 0.5 });
    expect(steps()).toBe(before + 1);

    await act(() => t.send(cmd("Edit", { type: "Undo" })));
    const back = project().devices[device.id]!;
    expect(back.kind.type === "Plugin" && back.kind.plugin.plugin_id).toBe(MOCK_MISSING_PLUGIN_ID);
  });

  it("finds a missing audio file in the library and relinks it with the plugin, in one step", async () => {
    const { device, t } = await setup();
    const id = newId();
    await act(() =>
      t.send(cmd("Media", { type: "Import", id, source: { type: "Location", location: { type: "Library", id: "library" }, path: "Drums/Kick.wav" } })),
    );
    // The referenced sample goes away.
    t.mediaRefs.setOffline(libraryPath("Drums/Kick.wav"));
    act(() => applyMediaRefEvent({ type: "Media", event: { type: "Missing", media: id } }));
    await act(() => openResolver({ media: id }, t));
    const s = useResolver.getState();
    expect(s.focus).toBe(`m:${id}`);
    expect(s.plugins).toHaveLength(1);
    const m = s.media.find((x) => x.missing.media === id)!;
    expect(m.candidates[0]).toMatchObject({ display: "Library/Drums/Kick.wav", confidence: 1, reason: "same content" });
    expect(m.apply).toBe(true);
    const dialog = await screen.findByRole("dialog", { name: "resolve “Kick.wav”" });
    fireEvent.click(within(dialog).getByRole("button", { name: /show all 2 missing items/ }));
    // Leaving the plugin missing: nothing is sent for it.
    act(() => useResolver.setState((st) => ({ plugins: st.plugins.map((p) => ({ ...p, apply: false })) })));
    expect(resolutions(useResolver.getState()).plugins).toEqual([]);
    act(() => useResolver.setState((st) => ({ plugins: st.plugins.map((p) => ({ ...p, apply: true })) })));
    const steps = () => (t as unknown as { undoStack: unknown[] }).undoStack.length;
    const before = steps();
    fireEvent.click(within(dialog).getByRole("button", { name: "apply 2 changes" }));
    await waitFor(() => expect(useResolver.getState().open).toBe(false));
    expect(steps()).toBe(before + 1);
    expect(project().media[id]!.location).toEqual({ type: "External", path: libraryPath("Drums/Kick.wav") });
    const d = project().devices[device.id]!;
    expect(d.kind.type === "Plugin" && d.kind.plugin.plugin_id).toBe("dev.ethereal.mock.mega");
  });
});

describe("AI suggestions", () => {
  const missing: MissingPlugin = {
    devices: ["d1"],
    format: "Vst3",
    plugin_id: "0123",
    name: "Old Synth 2",
    vendor: "Acme",
    version: "2.0",
    has_state: true,
    params: [
      { id: 1, value: 0.25, name: "Cutoff" },
      { id: 2, value: 0.5, name: null },
    ],
    candidates: [],
  };
  const installed: PluginDescriptor[] = [
    {
      format: "Clap",
      id: "com.acme.newsynth",
      name: "New Synth",
      vendor: "Acme",
      version: "3.0",
      description: "",
      features: [],
      category: "Instrument",
      path: "/x.clap",
      sidechain_inputs: 0,
    },
    {
      format: "Clap",
      id: "com.other.verb",
      name: "Verb",
      vendor: "Other",
      version: "1",
      description: "",
      features: [],
      category: "AudioEffect",
      path: "/y.clap",
      sidechain_inputs: 0,
    },
  ];

  it("sends only plugin metadata", () => {
    const { payload } = buildAiPayload([missing], installed);
    expect(payload.items[0]).toEqual({
      key: "p1",
      format: "Vst3",
      id: "0123",
      name: "Old Synth 2",
      vendor: "Acme",
      version: "2.0",
      param_count: 2,
      params: [
        { id: 1, name: "Cutoff", value: 0.25 },
        { id: 2, value: 0.5 },
      ],
      candidates: ["c1", "c2"],
    });
    expect(payload.candidates[0]).toEqual({ key: "c1", format: "Clap", name: "New Synth", vendor: "Acme", version: "3.0" });
    // No paths, descriptions or states.
    expect(JSON.stringify(payload)).not.toMatch(/x\.clap|path|state/);
  });

  it("validates replies strictly", () => {
    const { payload } = buildAiPayload([missing], installed);
    const ok = parseAiResponse(
      '```json\n{"matches":[{"item":"p1","candidate":"c1","confidence":0.7,"reason":"successor","params":[{"from":1,"to":9}]}]}\n```',
      payload,
    );
    expect(ok.suggestions).toEqual([{ item: "p1", candidate: "c1", confidence: 0.7, reason: "successor", params: [{ from: 1, to: 9 }] }]);
    expect(ok.rejected).toEqual([]);
    expect(() => parseAiResponse("Sure! Here is the json: {}", payload)).toThrow(AiResponseError);
    expect(() => parseAiResponse('{"matches":[],"extra":1}', payload)).toThrow(AiResponseError);
    expect(() => parseAiResponse("[]", payload)).toThrow(AiResponseError);
    const bad = parseAiResponse(
      JSON.stringify({
        matches: [
          { item: "p9", candidate: "c1", confidence: 0.5 },
          { item: "p1", candidate: "c7", confidence: 0.5 },
          { item: "p1", candidate: "c1", confidence: 1.5 },
          { item: "p1", candidate: "c1", confidence: 0.5, params: [{ from: 42, to: 1 }] },
          { item: "p1", candidate: "c1", confidence: 0.5, why: "x" },
          { item: "p1", candidate: null, confidence: 0.1 },
          { item: "p1", candidate: "c1", confidence: 0.9 },
        ],
      }),
      payload,
    );
    expect(bad.rejected).toHaveLength(6);
    expect(bad.suggestions).toEqual([{ item: "p1", candidate: null, confidence: 0.1, reason: "", params: [] }]);
  });

  /** A fake provider session answering `text` (counts the requests). */
  function fake(text: string) {
    const calls: string[][] = [];
    const session = (): ChatSession => ({
      history: [],
      restore: () => {},
      addUser: (parts) => calls.push(parts),
      step: async (_c, _s, onEvent: (e: LoopEvent) => void) => {
        onEvent({ type: "text_start" });
        onEvent({ type: "text", delta: text });
        return { calls: [], stop: "end_turn", raw: null };
      },
      commit: () => {},
      addToolResults: () => {},
      addNote: () => {},
    });
    return { calls, session };
  }

  it("runs in batches through the provider abstraction", async () => {
    const f = fake('{"matches":[{"item":"p1","candidate":"c1","confidence":0.6,"reason":"same vendor, newer synth"}]}');
    const res = await runAi(
      [missing],
      installed,
      { provider: "anthropic", conn: { baseUrl: "x", apiKey: "k", model: "m" }, signal: new AbortController().signal, session: f.session },
      "Anthropic",
    );
    expect(res.calls).toBe(1);
    const c = res.suggestions.get("Vst3:0123")!;
    expect(c).toMatchObject({ name: "New Synth", rule: "Ai", confidence: 0.6, plan: { type: "Auto" } });
    expect(c.reason).toBe("ai suggestion (Anthropic): same vendor, newer synth");
  });

  it("needs consent once and never applies a suggestion by itself", async () => {
    const { device, t } = await setup();
    await act(() => openResolver({ device: device.id }, t));
    // Unsure: drop the deterministic candidate so the AI is offered.
    act(() =>
      useResolver.setState((s) => ({
        plugins: s.plugins.map((p) => ({ ...p, missing: { ...p.missing, candidates: [] }, candidates: [], choice: null, apply: false })),
      })),
    );
    // No provider configured: no AI.
    expect(screen.queryByRole("button", { name: "ask ai" })).toBeNull();
    act(() => useAiSettings.getState().setApiKey("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123"));
    const f = fake('{"matches":[{"item":"p1","candidate":"c1","confidence":0.75,"reason":"newer version"}]}');
    const send = vi.spyOn(t, "send");
    // Asking first shows what will be sent; nothing is sent before consent.
    fireEvent.click(await screen.findByRole("button", { name: "ask ai" }));
    expect(useResolver.getState().ai.status).toBe("consent");
    fireEvent.click(screen.getByRole("button", { name: "show what will be sent" }));
    expect(screen.getByTestId("resolver-ai-payload")).toHaveTextContent('"name": "Mock Mega 1"');
    expect(f.calls).toHaveLength(0);
    await act(() => askAi({ session: f.session, consent: true }));
    expect(localStorage.getItem(AI_CONSENT_STORAGE)).toBe("yes");
    expect(f.calls).toHaveLength(1);
    const item = useResolver.getState().plugins[0]!;
    expect(item.candidates[0]).toMatchObject({ rule: "Ai", name: "Mock Mega" });
    expect(item.choice).toBe(0);
    expect(item.apply).toBe(false);
    expect(screen.getByText("ai suggestion")).toBeTruthy();
    expect(screen.getByText("1 ai call")).toBeTruthy();
    // Nothing was applied: no Resolve::Apply went out, and "Apply" has nothing to send.
    expect(send.mock.calls.some(([c]) => c.domain === "Resolve" && c.command.type === "Apply")).toBe(false);
    expect(await act(() => applyResolution(t))).toBe(false);
    // The next request needs no consent prompt.
    await act(() => askAi({ session: f.session }));
    expect(f.calls).toHaveLength(2);
    expect(useResolver.getState().ai.calls).toBe(2);
  });
});

describe("Resolve… entry points", () => {
  it("device header of a missing plugin", async () => {
    const { device } = await setup(<DeviceChain />);
    act(() => useSelectionStore.getState().selectTrack(device.track));
    const card = await screen.findByRole("region", { name: "Mock Mega 1" });
    fireEvent.contextMenu(card.querySelector(".eth-device__header")!);
    expect(menuLabels()[0]).toBe("resolve…");
    await act(async () => menuItem("resolve…").onSelect());
    await waitFor(() => expect(useResolver.getState().focus).toBe(`p:Clap:${MOCK_MISSING_PLUGIN_ID}`));
    // An installed plugin's header has no such entry.
    const installed = structuredClone(device);
    if (installed.kind.type === "Plugin") installed.kind.plugin.plugin_id = "dev.ethereal.mock.mega";
    expect(resolvePluginEntries(installed, mock)).toEqual([]);
    expect(resolverCommands(mock!, project()).map((c) => c.label)).toEqual(["resolve missing devices and samples…"]);
  });

  it("missing-plugin badge", async () => {
    const { device } = await setup();
    act(() => usePluginStore.getState().setPlugins([]));
    cleanup();
    render(
      <TransportProvider transport={mock!}>
        <PluginDeviceControls device={device} />
      </TransportProvider>,
    );
    fireEvent.contextMenu(screen.getByText("missing · bypassed"));
    expect(menuLabels()).toEqual(["resolve…"]);
  });

  it("missing audio clip in the arranger menu and the audio editor", async () => {
    await setup(<WarpEditor />);
    const clip = Object.values(project().clips).find((c) => c.content.type === "Audio")!;
    const media = clip.content.type === "Audio" ? clip.content.media : "";
    expect(resolveClipEntries(clip)).toEqual([]);
    act(() => applyMediaRefEvent({ type: "Media", event: { type: "Missing", media } }));
    const labels = withResolveClipEntries([{ label: "rename", onSelect: () => {} }, "separator", { label: "delete", onSelect: () => {} }], clip).map((e) =>
      e === "separator" ? "—" : e.label,
    );
    expect(labels).toEqual(["rename", "—", "resolve…", "—", "delete"]);
    act(() => useEditorStore.getState().openClip(clip.id));
    fireEvent.contextMenu(await screen.findByTestId("warp-editor"));
    expect(menuLabels()).toEqual(["resolve…"]);
    await act(async () => menuItem("resolve…").onSelect());
    // The mock engine still reads this (project) sample: the dialog opens on everything.
    await waitFor(() => expect(useResolver.getState().open).toBe(true));
  });

  it("missing-sample notice and Relink rows", async () => {
    const { t } = await setup(
      <>
        <MissingMediaNotice />
      </>,
    );
    const clip = Object.values(project().clips).find((c) => c.content.type === "Audio")!;
    const media = clip.content.type === "Audio" ? clip.content.media : "";
    act(() => applyMediaRefEvent({ type: "Media", event: { type: "Missing", media } }));
    fireEvent.contextMenu(screen.getByTestId("missing-media-notice"));
    expect(menuLabels()).toEqual(["resolve…"]);
    act(() => openRelink());
    render(<RelinkDialog transport={t as unknown as EngineTransport} />);
    fireEvent.contextMenu(await screen.findByTestId("relink-row"));
    expect(menuLabels()).toEqual(["resolve…"]);
  });
});
