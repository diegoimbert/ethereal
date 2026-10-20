// The pseudo-locale check (docs/LOCALIZATION.md): renders the localized surfaces in `en-XA`
// and fails on any text a person would read that didn't go through the catalogs. Each area's
// localization PR adds its surfaces here (with its paths in localized.ts).
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, it } from "vitest";
import { NumberField, Select, Toast, ToastStack, useConfirm } from "@/kit";
import { FloatingWindow } from "@/kit/FloatingWindow";
import { renderWithMock, resetStores } from "@/features/transport-bar/testUtils";
import { AudioSettingsDialog, openSettings, useAudioSettings } from "@/features/audio-settings";
import type { SettingsTab } from "@/features/audio-settings/store";
import { DocsApp } from "@/features/docs/DocsApp";
import { buildDocs } from "@/features/docs/model";
import { useMemoryRouter } from "@/features/docs/router";
import { EngineErrorScreen } from "@/features/startup/EngineErrorScreen";
import { OtherTabScreen } from "@/features/startup/OtherTabScreen";
import { StorageSettings, useStorageStatus } from "@/features/storage";
import { DeviceChain } from "@/features/devices";
import { DeviceLibrary } from "@/features/devices/DeviceLibrary";
import { PluginFoldersPanel } from "@/features/plugins/PluginFolders";
import { FailedPluginsDialog } from "@/features/plugins/FailedPlugins";
import { usePluginStore } from "@/features/plugins/pluginStore";
import { ArrangementView } from "@/features/arrangement/ArrangementView";
import { Toolbar } from "@/features/arrangement/Toolbar";
import { ArrangementContext, type ArrangementContextValue } from "@/features/arrangement/context";
import { UngroupConfirmDialog, useUngroupConfirm } from "@/features/groups";
import { MetronomeSettings } from "@/features/tempo/MetronomeSettings";
import { timeSelectionMenu } from "@/features/time-edits/actions";
import { ProjectMenu } from "@/features/project";
import { useExportProject } from "@/features/project/exportProjectStore";
import { useProjectScreen } from "@/features/project/screenStore";
import { useTemplateDialog } from "@/features/templates/model";
import { openSnapshotPrompt, openVersions, useSnapshotPrompt, useVersionsDialog, VersionsRoot } from "@/features/versions";
import { RemoteEngineSettings, useEngineServerShown } from "@/features/remote";
import { MissingDeviceControls, ResolverDialog, resetResolver, useResolver } from "@/features/import-resolver";
import { ExportDialog } from "@/features/export";
import { useExportAudio } from "@/features/export/store";
import { MissingMediaStatus } from "@/features/media-refs/MediaRefsRoot";
import { RelinkDialog } from "@/features/media-refs/RelinkDialog";
import { applyMediaRefEvent, openRelink, resetMediaRefs } from "@/features/media-refs/store";
import { ImportList } from "@/features/import/ImportStatus";
import { useImportStore } from "@/features/import/importStore";
import type { Device } from "@/generated";
import { useProjectStore } from "@/state";
import { TransportContext, type EngineTransport } from "@/transport";
import { MockTransport } from "@/transport/mock";
import { useContext, type ReactNode } from "react";
import { Browser, FolderBrowser } from "@/features/browser";
import { FileFilterSettings } from "@/features/browser/settings/FileFilterSettings";
import { AiChatPanel } from "@/features/ai-chat/AiChatPanel";
import { reloadAiSettings, useAiSettings } from "@/features/ai-chat/settings";
import { DebugPanelsRoot, DebugPanelsSettings } from "@/features/debug/DebugPanelsRoot";
import { NewsList } from "@/features/news/NewsDialog";
import { resetNews, useNews } from "@/features/news/store";
import { openDebugPanel, registerDebugPanel, resetDebugPanels } from "@/features/debug/registry";
import { TourRoot } from "@/features/tutorials";
import { askExit, nextStep, resetTour, startTour } from "@/features/tutorials/store";
import { PLATFORM_ENVS } from "@/features/tutorials/targets";
import { TOURS } from "@/features/tutorials/tours";
import { textOf } from "@/features/tutorials/types";
import { DrumRackTab } from "@/features/drum-rack/DrumRackView";
import { sliceSampleEntries } from "@/features/drum-rack/menus";
import { GrooveControls, GroovePanel, grooveMenuItems } from "@/features/groove";
import { MpeSettingsFields } from "@/features/mpe/MpeSettingsFields";
import { DEFAULT_MPE } from "@/domain/mpe";
import { PresetDialogs, type PresetDialog } from "@/features/presets/PresetDialogs";
import { PresetMenu } from "@/features/presets/PresetMenu";
import { kindText, noteKindText } from "@/features/expression/labels";
import { confirmClearLane, confirmClearNotes, confirmRemoveLane } from "@/features/expression/confirm";
import { channelOptions, midiChannels, midiOptions, missingParts } from "@/features/external/routing";
import { addModulatorEntries } from "@/features/modulation/kinds";
import { sourceName } from "@/features/modulation/model";
import { groupEntries } from "@/features/racks";
import { TAP_ERROR_TEXT, tapPoints } from "@/features/sidechain/routing";
import type { DeviceDescriptor, ModulatorDescriptor, PresetInfo } from "@/generated";
import { useSelectionStore } from "@/state";
import { TransportBar } from "@/features/transport-bar";
import { CommandPalette } from "@/app/shell/CommandPalette";
import { Inspector } from "@/app/shell/Inspector";
import { PanelBoundary } from "@/app/PanelBoundary";
import { BUILTIN_ACTIONS, KeymapRoot, openKeymapEditor } from "@/features/keymap";
import { CheatSheetContent } from "@/features/keymap/CheatSheet";
import { DesktopBandFrame } from "@/features/desktop-band";
import { EngineLogSettings } from "@/features/engine-health";
import { useShellStore } from "@/state/shell";
import { PianoRoll } from "@/features/piano-roll";
import { RecordingControls } from "@/features/recording";
import { MidiLearnPanel } from "@/features/midi-learn";
import { WarpEditor } from "@/features/warp";
import { CaptureButton } from "@/features/capture";
import { KeyboardMidiButton } from "@/features/keyboard-midi";
import { trackTakeEntries } from "@/features/comping/actions";
import { freezeTrackEntries } from "@/features/freeze/menus";
import { useEditorStore } from "@/state";
import { cmd } from "@/transport";
import { PSEUDO_CLOSE, PSEUDO_OPEN } from "./pseudo";
import { resetI18nForTests, setLocale, t } from "./runtime";
import { isText } from "./tooling/text";

const TEXT_ATTRS = ["aria-label", "title", "placeholder", "aria-valuetext", "alt"];

/** Text in `root` a reader sees that isn't pseudo-translated (user content and code excepted). */
function untranslated(root: Element): string[] {
  const out: string[] = [];
  const exempt = (el: Element | null) => !!el?.closest(".eth-user-text, code, [data-i18n-exempt]");
  // A rich message's inner parts (`<b>…</b>`) carry no bracket: their words are all accented.
  const accented = (s: string) => !isText(s.replace(/[\p{L}\p{N}]*[^\p{ASCII}][\p{L}\p{N}]*/gu, " "));
  const translated = (s: string) => s.includes(PSEUDO_OPEN) || s.includes(PSEUDO_CLOSE) || !isText(s) || accented(s);
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const text = n.textContent ?? "";
    if (!exempt(n.parentElement) && !translated(text)) out.push(`text "${text.trim()}" in <${n.parentElement?.tagName.toLowerCase()}>`);
  }
  for (const el of [root, ...root.querySelectorAll("*")]) {
    if (exempt(el)) continue;
    for (const a of TEXT_ATTRS) {
      const v = el.getAttribute(a);
      if (v && !translated(v)) out.push(`${a}="${v}" on <${el.tagName.toLowerCase()}>`);
    }
  }
  return out;
}

beforeAll(async () => {
  await setLocale("en-XA", { persist: false });
});
afterAll(() => resetI18nForTests());
afterEach(() => {
  resetStores();
  act(() => useAudioSettings.getState().close());
});

/** The inspector on the mock project's first audio clip. */
function AudioClipInspector() {
  const id = useProjectStore((s) => Object.values(s.project?.clips ?? {}).find((c) => c.content.type === "Audio")?.id);
  return id ? <Inspector target={{ kind: "clips", ids: [id] }} /> : null;
}

describe("the pseudo-locale finds no untranslated text", () => {
  it("in the kit", () => {
    const { container } = render(
      <>
        <ToastStack>
          <Toast title={<span className="eth-user-text">a</span>} onDismiss={() => {}} />
        </ToastStack>
        <NumberField value={1} mixed onChange={() => {}} aria-label={PSEUDO_OPEN} />
        <FloatingWindow id="pseudo" title={<span className="eth-user-text">w</span>} label={PSEUDO_OPEN} onClose={() => {}}>
          <div />
        </FloatingWindow>
        <Select
          aria-label={PSEUDO_OPEN}
          value="a"
          searchable
          activeFilter={{ value: false, onChange: () => {} }}
          options={[
            { value: "a", label: "a", userText: true, active: true },
            { value: "b", label: "b", userText: true },
          ]}
          onChange={() => {}}
        />
      </>,
    );
    fireEvent.click(screen.getByRole("combobox"));
    fireEvent.change(screen.getByRole("searchbox"), { target: { value: "zzz" } });
    expect(untranslated(container.ownerDocument.body)).toEqual([]);
  });

  it("in settings > general (language, startup) and the dialog's frame", async () => {
    await renderWithMock(<AudioSettingsDialog />);
    await act(async () => openSettings("general"));
    const dialog = await screen.findByRole("dialog");
    const frame = [dialog.querySelector(".eth-dialog__header"), screen.getByRole("tablist"), dialog.querySelector(".eth-dialog__footer")];
    for (const el of [...frame, screen.getByTestId("settings-language"), screen.getByTestId("settings-general")]) {
      expect(el).not.toBeNull();
      expect(untranslated(el!)).toEqual([]);
    }
  });

  for (const tab of ["appearance", "input", "audio", "privacy"] satisfies SettingsTab[]) {
    it(`in settings > ${tab}`, async () => {
      await renderWithMock(<AudioSettingsDialog />);
      await act(async () => openSettings(tab));
      const dialog = await screen.findByRole("dialog");
      expect(untranslated(dialog)).toEqual([]);
    });
  }

  it("in the docs' chrome (the content is phase 3)", () => {
    const docs = buildDocs({
      "a/_section.md": "---\ntitle: a\norder: 1\n---\n",
      "a/one.md": "---\ntitle: one\norder: 1\nsummary: s.\nplatform: desktop\n---\n## x\n\n> [!tip]\n> y\n\n## z\n\nw\n\n## v\n\nu",
      "a/two.md": "---\ntitle: two\norder: 2\nsummary: s.\n---\nt",
    });
    function Harness() {
      const router = useMemoryRouter({ path: "a/one", anchor: "" });
      return <DocsApp router={router} docs={docs} onClose={() => {}} />;
    }
    const { container } = render(<Harness />);
    const chrome = [
      container.querySelector(".eth-docs__top"),
      ...container.querySelectorAll(".eth-docs-pager__label, .eth-docs__toc-title, .eth-docs__toc-inline > summary, .eth-docs-badge"),
    ];
    expect(chrome.length).toBeGreaterThan(3);
    for (const el of chrome) expect(untranslated(el!)).toEqual([]);
    // Landmarks and callouts: only their own names (their content is the docs').
    for (const el of container.querySelectorAll("nav, aside, [role=dialog], .eth-docs-heading__anchor")) {
      const name = el.getAttribute("aria-label");
      if (name) expect(name).toContain(PSEUDO_OPEN);
    }
  });

  it("in the startup screens and the storage settings", () => {
    for (const ui of [
      <EngineErrorScreen key="e" error="wasm failed" onRetry={() => {}} />,
      <EngineErrorScreen key="u" error={null} />,
      <OtherTabScreen key="b" reason="busy" onUseHere={() => Promise.resolve()} />,
      <OtherTabScreen key="m" reason="moved" onUseHere={() => Promise.resolve()} />,
    ]) {
      const { container, unmount } = render(ui);
      expect(untranslated(container)).toEqual([]);
      unmount();
    }
    useStorageStatus.setState({ persistence: "best-effort", usage: 5e6, quota: 2e9 });
    const { container } = render(<StorageSettings />);
    expect(untranslated(container)).toEqual([]);
  });

  it("in the device chain, the device library and the plugin settings", async () => {
    const { container, unmount } = await renderWithMock(<DeviceChain />);
    const card = await screen.findByRole("region", { name: "Synth" });
    // The chain's own controls (the header's preset, modulator and sidechain menus are other areas').
    const own = [
      ...container.querySelectorAll(".eth-devices__toolbar, .eth-devices__section-label, .eth-device__actions"),
      ...card.querySelectorAll(".eth-device__power, .eth-device__window-toggle, .eth-device__name"),
    ];
    expect(own.length).toBeGreaterThan(4);
    for (const el of own) expect(untranslated(el)).toEqual([]);
    for (const el of container.querySelectorAll("[role=list], [role=group]")) expect(el.getAttribute("aria-label")).toContain(PSEUDO_OPEN);
    unmount();

    const lib = render(<DeviceLibrary category="AudioEffect" onPick={() => {}} />);
    const chrome = lib.container.querySelectorAll(".eth-devlib__search, .eth-devlib__sources, .eth-devlib__footer, .eth-devlib__empty");
    expect(chrome.length).toBeGreaterThan(2);
    for (const el of chrome) expect(untranslated(el)).toEqual([]);
    lib.unmount();

    const web = new MockTransport({ timers: "manual", seed: 1 });
    const folders = render(<PluginFoldersPanel transport={web} />);
    expect(untranslated(folders.container)).toEqual([]);
    folders.unmount();
    act(() => usePluginStore.getState().setFailed([{ path: "/p/bad.clap", format: "Clap", message: "" }]));
    render(<FailedPluginsDialog transport={web} open onClose={() => {}} />);
    expect(untranslated(await screen.findByRole("dialog"))).toEqual([]);
    act(() => usePluginStore.getState().reset());
    web.dispose();
  });

  it("in the arrangement's toolbar and add-track menu, the metronome settings, the ungroup dialog and the time menu", async () => {
    const { container, unmount } = render(<ArrangementView />);
    expect(untranslated(container)).toEqual([]);
    unmount();
    const bar = render(
      <ArrangementContext.Provider value={{} as ArrangementContextValue}>
        <Toolbar />
      </ArrangementContext.Provider>,
    );
    fireEvent.click(bar.container.querySelector(".eth-arr-toolbar__add")!);
    expect(screen.getByRole("menu")).toBeTruthy();
    expect(untranslated(bar.container.ownerDocument.body)).toEqual([]);
    bar.unmount();
    await renderWithMock(<MetronomeSettings />);
    fireEvent.click(await screen.findByTestId("metronome-settings"));
    expect(untranslated(document.body)).toEqual([]);
    cleanup();
    const transport = {} as EngineTransport;
    act(() => useUngroupConfirm.getState().ask(transport, { group: "g", name: "Drums", losses: ["1"] }));
    render(<UngroupConfirmDialog />);
    expect(untranslated(await screen.findByRole("dialog"))).toEqual([]);
    act(() => useUngroupConfirm.getState().close());
    for (const e of timeSelectionMenu(transport)) if (e !== "separator") expect(e.label).toContain(PSEUDO_OPEN);
  });

  it("in the project screen, the project's dialogs, the templates and the versions", async () => {
    const { container } = await renderWithMock(
      <>
        <ProjectMenu />
        <VersionsRoot />
      </>,
    );
    const project = useProjectStore.getState().project!;
    // The welcome screen.
    act(() => useProjectScreen.getState().show());
    let dialog = await screen.findByRole("dialog");
    await waitFor(() => expect(dialog.querySelector(".eth-welcome__start")).not.toBeNull());
    expect(untranslated(dialog)).toEqual([]);
    // The new-project form (and its template picker).
    act(() => useProjectScreen.getState().show("new"));
    dialog = await screen.findByRole("dialog");
    expect(untranslated(dialog)).toEqual([]);
    act(() => useProjectScreen.getState().hide());
    // The top bar: the project's name is user text; the unsaved dot and safe mode.
    act(() => useProjectStore.setState({ dirty: true, safe: true, safeMode: [] }));
    expect(untranslated(container)).toEqual([]);
    act(() => useProjectStore.setState({ dirty: false, safe: false }));
    for (const open of [
      () => useExportProject.getState().show(project.id, project.settings.name),
      () => useTemplateDialog.getState().open({ type: "save-tracks", tracks: [], name: "" }),
      () => useTemplateDialog.getState().open({ type: "save-project", name: "a" }),
      // The insert list holds the factory templates, whose descriptions (row tooltips) are the engine's.
      () => useTemplateDialog.getState().open({ type: "insert", placement: { parent: null, before: null } }),
      () => openSnapshotPrompt(),
      () => openVersions(),
    ]) {
      act(open);
      const shown = await screen.findByRole("dialog");
      // Lists the engine answers (the versions, the templates).
      await act(() => new Promise((r) => setTimeout(r, 10)));
      const factory = useTemplateDialog.getState().dialog?.type === "insert";
      expect(untranslated(shown).filter((x) => !(factory && x.startsWith("title=")))).toEqual([]);
      act(() => {
        useExportProject.getState().hide();
        useTemplateDialog.getState().close();
        useSnapshotPrompt.getState().hide();
        useVersionsDialog.getState().hide();
      });
    }
  });

  it("in the engine server settings", async () => {
    useEngineServerShown.setState({ shown: true });
    const { container } = await renderWithMock(<RemoteEngineSettings />);
    fireEvent.click(within(container).getAllByRole("button").at(-1)!);
    await within(container).findByRole("alert");
    expect(untranslated(container)).toEqual([]);
    useEngineServerShown.setState({ shown: false });
  });

  it("in the import and export dialogs (resolver, import and export reports, export audio, relink, imports)", async () => {
    function WithTransport({ render: r }: { render(t: EngineTransport): ReactNode }) {
      const transport = useContext(TransportContext)?.transport;
      return transport ? <>{r(transport)}</> : null;
    }
    const { unmount } = await renderWithMock(
      <>
        <WithTransport
          render={(t) => (
            <>
              <ResolverDialog transport={t} />
              <RelinkDialog transport={t} />
            </>
          )}
        />
        <ExportDialog />
      </>,
    );
    const project = useProjectStore.getState().project!;
    const device = Object.keys(project.devices)[0]!;
    const media = Object.keys(project.media)[0]!;
    act(() =>
      useResolver.setState({
        open: true,
        search: { scanned: 3, done: true },
        plugins: [
          {
            key: "p",
            missing: {
              devices: [device],
              format: "Vst3",
              plugin_id: "x",
              name: "Old",
              vendor: "Acme",
              version: "1",
              has_state: true,
              params: [],
              candidates: [],
            },
            candidates: [
              {
                format: "Vst3",
                id: "y",
                name: "New",
                vendor: "Acme",
                version: "2",
                rule: "SameFamily",
                confidence: 0.9,
                reason: "Chosen by you",
                plan: { type: "Auto" },
                lost: ["a", "b"],
              },
            ],
            choice: 0,
            apply: false,
          },
        ],
        devices: [
          {
            key: "d",
            missing: { devices: [device], source_id: "s", description: "Verb", origin: "Live", role: "AudioEffect", params: [], candidates: [] },
            candidates: [],
            choice: null,
            apply: false,
          },
        ],
        media: [
          {
            key: "m",
            missing: { media, name: "Kick.wav", path: null, sample_rate: 48000, channels: 2, frames: 1, hashed: false },
            candidates: [],
            choice: null,
            apply: false,
            searched: true,
          },
        ],
      }),
    );
    expect(untranslated(await screen.findByTestId("resolver"))).toEqual([]);
    expect(untranslated(screen.getByRole("dialog").querySelector(".eth-dialog__footer")!)).toEqual([]);
    act(() => useResolver.setState({ open: false }));
    act(() => useExportAudio.getState().show());
    const exportDialog = await screen.findByRole("dialog");
    fireEvent.click(within(exportDialog).getAllByRole("switch")[1]!);
    expect(untranslated(exportDialog)).toEqual([]);
    act(() => useExportAudio.getState().hide());

    act(() => applyMediaRefEvent({ type: "Media", event: { type: "Missing", media } }));
    const notice = render(<MissingMediaStatus />);
    expect(untranslated(notice.container)).toEqual([]);
    notice.unmount();
    act(() => openRelink());
    expect(untranslated(await screen.findByRole("dialog"))).toEqual([]);
    unmount();
    resetMediaRefs();
    resetResolver();
  });

  it("in the import progress list and a missing device's controls", () => {
    act(() =>
      useImportStore.setState({
        items: [
          { id: "a", name: "a.wav", phase: "uploading", progress: 0.5, error: null, cancel: () => {} },
          { id: "b", name: "b.wav", phase: "queued", progress: null, error: "b.wav: x", cancel: null },
        ],
      }),
    );
    const { container, unmount } = render(<ImportList />);
    // A failed import's row shows its error (the file name and the engine's reason).
    for (const el of container.querySelectorAll('[data-phase="error"] .eth-import__text')) el.setAttribute("data-i18n-exempt", "");
    expect(untranslated(container)).toEqual([]);
    unmount();
    act(() => useImportStore.setState({ items: [] }));
    const missing = {
      id: "dev",
      name: "Verb",
      kind: { type: "Missing", missing: { description: "Verb", origin: "Live" } },
    } as unknown as Device;
    const header = render(<MissingDeviceControls device={missing} />);
    expect(untranslated(header.container)).toEqual([]);
    header.unmount();
  });

  it("in the library browser (indexed and folders) and its file filter settings", async () => {
    const { container, unmount } = await renderWithMock(<Browser scope="library" />);
    await screen.findByRole("treeitem", { name: /Library/ });
    expect(untranslated(container)).toEqual([]);
    unmount();
    const folders = await renderWithMock(<FolderBrowser />);
    await screen.findByRole("button", { name: "Drums" });
    expect(untranslated(folders.container)).toEqual([]);
    folders.unmount();
    function Filters() {
      const transport = useContext(TransportContext)?.transport;
      return transport ? <FileFilterSettings transport={transport} /> : null;
    }
    const filters = await renderWithMock(<Filters />);
    await waitFor(() => expect(screen.getByTestId("file-filter-count").textContent).toContain(PSEUDO_OPEN));
    expect(untranslated(filters.container)).toEqual([]);
  });

  it("in the AI chat (key setup, consent, conversation, settings)", async () => {
    localStorage.clear();
    reloadAiSettings();
    const setup = await renderWithMock(<AiChatPanel />);
    expect(untranslated(setup.container)).toEqual([]);
    setup.unmount();
    act(() => useAiSettings.getState().setApiKey("sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123"));
    const consent = await renderWithMock(<AiChatPanel />);
    expect(screen.getByTestId("ai-consent")).toBeInTheDocument();
    expect(untranslated(consent.container)).toEqual([]);
    fireEvent.click(screen.getByTestId("ai-consent-accept"));
    expect(screen.getByTestId("ai-input")).toBeInTheDocument();
    expect(untranslated(consent.container)).toEqual([]);
    // The toolbar's last button: the AI settings.
    fireEvent.click([...consent.container.querySelectorAll(".eth-ai__toolbar button")].at(-1)!);
    expect(screen.getByTestId("ai-settings")).toBeInTheDocument();
    expect(untranslated(consent.container)).toEqual([]);
    consent.unmount();
    localStorage.clear();
    reloadAiSettings();
  });

  it("in the news list, in each of its states", () => {
    for (const state of [{ enabled: false }, { enabled: true, items: [], loading: false, error: null }, { enabled: true, items: null, error: PSEUDO_OPEN }]) {
      act(() => useNews.setState({ ...state, listOpen: true } as never));
      const { unmount } = render(<NewsList />);
      expect(untranslated(screen.getByRole("dialog"))).toEqual([]);
      unmount();
    }
    act(() => resetNews());
  });

  it("in the debug panels", () => {
    const off = registerDebugPanel({ id: "pseudo", title: PSEUDO_OPEN, render: () => <span className="eth-user-text">p</span> });
    act(() => openDebugPanel("pseudo"));
    const { container, unmount } = render(
      <>
        <DebugPanelsSettings />
        <DebugPanelsRoot />
      </>,
    );
    expect(untranslated(container.ownerDocument.body)).toEqual([]);
    unmount();
    off();
    resetDebugPanels();
  });

  it("in the guided tours: every step's text, illustration and actions, and the bubble's frame", () => {
    for (const tour of Object.values(TOURS))
      for (const p of tour.platforms) {
        const ctx = { transport: null, env: PLATFORM_ENVS[p] };
        expect(t(tour.title)).toContain(PSEUDO_OPEN);
        for (const step of tour.steps) {
          for (const k of [step.title, step.hint, ...(step.actions ?? []).map((a) => a.label)])
            if (k) expect(t(k), `${tour.id}/${step.id}`).toContain(PSEUDO_OPEN);
          const { container, unmount } = render(
            <>
              {textOf(step.body, ctx)}
              {step.fallback !== undefined && textOf(step.fallback, ctx)}
              {step.illustration}
            </>,
          );
          expect(untranslated(container), `${tour.id}/${step.id} (${p})`).toEqual([]);
          unmount();
        }
      }
    // The bubble: header, hint, back / next, the exit confirmation.
    const tour = TOURS["record-audio"]!;
    render(<TourRoot />);
    try {
      act(() => startTour(tour, { transport: null, env: PLATFORM_ENVS.desktop }));
      act(() => nextStep());
      expect(untranslated(document.body)).toEqual([]);
      act(() => askExit(true));
      expect(untranslated(document.body)).toEqual([]);
    } finally {
      act(() => resetTour());
    }
  });

  it("in the drum rack, presets, groove, MPE, expression lanes, external routing, sidechain and the rack and modulation menus", async () => {
    const pseudo = (s: string) => expect(s).toContain(PSEUDO_OPEN);
    // The drum rack tab: no track, then an audio track (racks go on MIDI tracks).
    const rack = await renderWithMock(<DrumRackTab />);
    expect(untranslated(rack.container)).toEqual([]);
    const tracks = Object.values(useProjectStore.getState().project!.tracks);
    act(() => useSelectionStore.getState().selectTrack(tracks.find((tr) => tr.kind === "Audio")!.id));
    await waitFor(() => expect(rack.container.querySelector(".eth-drum-rack__hint")).toBeTruthy());
    expect(untranslated(rack.container)).toEqual([]);
    const midi = tracks.find((tr) => tr.kind === "Midi")!;
    const device = Object.values(useProjectStore.getState().project!.devices).find((d) => d.track === midi.id)!;
    rack.unmount();
    cleanup();
    // Groove: the project swing and the piano roll's popovers.
    const groove = await renderWithMock(
      <>
        <GroovePanel />
        <GrooveControls clip="c" selected={[]} rollStep={1} />
        <MpeSettingsFields
          track={{ ...midi, mpe: DEFAULT_MPE }}
          Row={({ label, children }) => (
            <label>
              {label}
              {children}
            </label>
          )}
        />
        <PresetMenu device={device} />
      </>,
    );
    expect(untranslated(groove.container)).toEqual([]);
    for (const title of [t("groove.quantize.title"), t("groove.humanize.title")]) {
      fireEvent.click(screen.getByTitle(title));
      expect(untranslated(document.body)).toEqual([]);
      fireEvent.keyDown(document.activeElement ?? document.body, { key: "Escape" });
    }
    cleanup();
    // The preset dialogs.
    const preset = { preset: { source: "User", id: "p" }, name: "Warm", meta: { tags: [], author: null, description: null } } as unknown as PresetInfo;
    for (const dialog of [{ type: "save" }, { type: "rename", preset }, { type: "delete", preset }] as PresetDialog[]) {
      const d = await renderWithMock(<PresetDialogs device={device} dialog={dialog} presets={[preset]} onClose={() => {}} />);
      expect(untranslated(await screen.findByRole("dialog"))).toEqual([]);
      d.unmount();
      cleanup();
    }
    // Menus, labels and confirmations built as data.
    const project = useProjectStore.getState().project;
    for (const e of grooveMenuItems("c", [], 1, () => {})) if (e !== "separator") pseudo(e.label);
    const kinds = ["Lfo", "Envelope", "EnvelopeFollower", "Steps", "Random", "Keytrack", "Velocity"].map((kind) => ({
      kind,
      name: kind,
    })) as unknown as ModulatorDescriptor[];
    for (const e of addModulatorEntries(kinds, device, () => {})) if (e !== "separator") pseudo(e.label);
    const synth = { ...device, pad: null, chain: undefined, kind: { type: "Builtin", device: { type: "Synth" } } } as Device;
    for (const category of ["Instrument", "AudioEffect", "NoteEffect"] as const)
      for (const e of groupEntries(synth, { category } as DeviceDescriptor, () => {})) if (e !== "separator") pseudo(e.label);
    if (project) pseudo(sourceName(project, { type: "Macro", rack: "r", index: 0 }));
    for (const k of [{ type: "PitchBend" }, { type: "ChannelPressure" }, { type: "Cc", controller: 1 }, { type: "Cc", controller: 20 }] as const)
      pseudo(kindText(k));
    for (const k of ["Pitch", "Pressure", "Timbre"] as const) pseudo(noteKindText(k));
    for (const ask of [() => confirmClearLane("x", 2, () => {}), () => confirmRemoveLane("x", 2, () => {}), () => confirmClearNotes("x", 1, () => {})]) {
      ask();
      const r = useConfirm.getState().request!;
      for (const text of [r.title, r.body, r.action]) pseudo(String(text));
      useConfirm.getState().close();
    }
    for (const o of [
      ...midiChannels(),
      channelOptions([], { first: 0, count: 2 }, t("external.routing.in_prefix"), "x")[0]!,
      midiOptions(null, null, null)[0]!,
    ])
      pseudo(o.label);
    for (const part of missingParts({ midi_out: "gone", midi_channel: 1, audio_send: { first: 0, count: 1 }, audio_return: { first: 0, count: 1 } }, {
      midi_outputs: [],
      audio_inputs: [],
      audio_outputs: [],
    } as never))
      pseudo(part);
    for (const key of Object.values(TAP_ERROR_TEXT)) pseudo(t(key));
    if (project) for (const o of tapPoints(project, midi.id)) pseudo(o.label);
    expect(sliceSampleEntries(null, {} as never)).toEqual([]);
  });

  it("in the shell: transport bar, command palette, keymap editor, cheat sheet, inspector, crash card, desktop band", async () => {
    const { container } = await renderWithMock(
      <>
        <TransportBar />
        <CommandPalette />
        <KeymapRoot />
        <AudioClipInspector />
      </>,
    );
    // The capture and computer keyboard buttons are their own areas'.
    const bar = screen.getByRole("toolbar");
    for (const other of bar.querySelectorAll(".eth-capture, .eth-kbmidi")) other.setAttribute("data-i18n-exempt", "");
    expect(untranslated(bar)).toEqual([]);
    // The palette's own text (the commands of other areas are theirs): its frame, empty.
    act(() => useShellStore.getState().setPalette(true));
    fireEvent.change(document.querySelector(".eth-palette__input")!, { target: { value: "zzzzzz" } });
    // Chords ("Ctrl+K") are key names, as the keyboard prints them.
    const keysExempt = () => document.querySelectorAll("kbd").forEach((k) => k.setAttribute("data-i18n-exempt", ""));
    keysExempt();
    expect(untranslated(screen.getByRole("dialog"))).toEqual([]);
    act(() => useShellStore.getState().setPalette(false));
    act(() => openKeymapEditor());
    const editor = await screen.findByTestId("keymap-editor");
    keysExempt();
    // The rows of other areas' commands and actions (the palette's, the computer keyboard's) are theirs.
    const builtin = new Set(BUILTIN_ACTIONS.map((a) => a.id));
    editor.querySelectorAll<HTMLElement>("[data-action]").forEach((row) => !builtin.has(row.dataset.action!) && row.setAttribute("data-i18n-exempt", ""));
    expect(untranslated(editor)).toEqual([]);

    const sheet = render(<CheatSheetContent />);
    keysExempt();
    sheet.container
      .querySelectorAll("dt")
      .forEach((dt) => !BUILTIN_ACTIONS.some((a) => dt.textContent!.startsWith(t(a.labelKey!))) && dt.setAttribute("data-i18n-exempt", ""));
    expect(untranslated(sheet.container)).toEqual([]);
    sheet.unmount();

    // An audio clip's fields (the device chain below is the devices' area).
    const sections = [...container.querySelectorAll(".eth-inspector__section")].filter((el) => !el.querySelector("[data-feature], .eth-device"));
    expect(sections.length).toBeGreaterThan(1);
    for (const el of sections) expect(untranslated(el)).toEqual([]);

    const Boom = () => {
      throw new Error("boom");
    };
    const error = console.error;
    console.error = () => {};
    const crash = render(
      <PanelBoundary name="inspector">
        <Boom />
      </PanelBoundary>,
    );
    console.error = error;
    expect(untranslated(crash.getByTestId("panel-crash").querySelector(".eth-crash__card")!).filter((s) => !s.includes("boom"))).toEqual([]);
    crash.unmount();

    for (const ui of [
      <DesktopBandFrame key="d" show openDeepLink={() => {}}>
        <div />
      </DesktopBandFrame>,
      <EngineLogSettings key="l" />,
    ]) {
      const { container, unmount } = render(ui);
      expect(untranslated(container)).toEqual([]);
      unmount();
    }
  });

  it("in the editors: piano roll, warp, recording controls, MIDI learn, capture, the MIDI keyboard and the take and freeze menus", async () => {
    const { container, mock } = await renderWithMock(
      <>
        <RecordingControls />
        <CaptureButton />
        <KeyboardMidiButton />
        <MidiLearnPanel />
        <WarpEditor />
        <PianoRoll />
      </>,
    );
    expect(untranslated(container)).toEqual([]);
    const track = Object.values(useProjectStore.getState().project!.tracks).find((t) => t.kind === "Midi")!;
    const clip = "01PSEUDOPIANOROLLCLIP00000";
    await mock.send(cmd("Clip", { type: "CreateMidi", id: clip, track: track.id, start: 0, length: 8, name: "Test" }));
    await mock.send(cmd("Note", { type: "Add", clip, notes: [{ id: "01PSEUDOPIANOROLLNOTE00000", pitch: 60, velocity: 0.8, start: 1, duration: 1 }] }));
    act(() => useEditorStore.getState().openClip(clip));
    await screen.findByTestId("piano-roll");
    fireEvent.click(screen.getByTestId("scale-trigger"));
    await screen.findByTestId("scale-popover");
    // The groove popovers and the expression lanes in the toolbar and under the notes are their own areas'.
    const others = /quantize settings|randomize timing|"lane"|"velocity"/;
    expect(untranslated(document.body).filter((s) => !others.test(s))).toEqual([]);
    const t = useProjectStore.getState().project!.tracks[track.id]!;
    for (const e of [...trackTakeEntries(mock, track.id), ...freezeTrackEntries(mock, t, [])]) if (e !== "separator") expect(e.label).toContain(PSEUDO_OPEN);
  });

  it("catches what it should", () => {
    const { container } = render(
      <div>
        <span>{`${PSEUDO_OPEN}ok`}</span>
        <span>hard-coded</span>
        <span className="eth-user-text">my track</span>
        <button aria-label="Close" />
        <span>-6 dB · 44.1 kHz · MIDI</span>
      </div>,
    );
    expect(untranslated(container)).toEqual(['text "hard-coded" in <span>', 'aria-label="Close" on <button>']);
  });
});
