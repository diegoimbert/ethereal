import { openAiChat } from "@/features/ai-chat";
import { openAudioSettings, openSettings } from "@/features/audio-settings";
import { captureMidi } from "@/features/capture";
import { focusChat } from "@/features/collab/social";
import { useCollabStore } from "@/features/collab/store";
import type { DeviceDescriptor, Project } from "@/generated";
import { addTrack, selectTrackEntity } from "@/features/arrangement/actions";
import { arrangementView } from "@/features/arrangement/uiStore";
import { openImportDialog } from "@/features/import";
import { openKeymapEditor, printCheatSheet } from "@/features/keymap";
import { useProjectScreen } from "@/features/project/screenStore";
import { placementAfter, tracksToSave, useTemplateDialog } from "@/features/templates";
import { useArrangementUi } from "@/features/arrangement/uiStore";
import { mediaRefCommands } from "@/features/media-refs";
import { nameCurrentCheckpoint } from "@/features/undo-history";
import { shareCommands } from "@/features/share/commands";
import { joinPaletteCommands } from "@/features/share/join";
import { tracksOrdered, useProjectStore } from "@/state";
import { getTheme, setTheme } from "@/theme";
import { cmd, type EngineTransport } from "@/transport";
import { canInsert, deviceTargetTrack, insertDeviceCommand } from "./deviceInsert";
import { useShellStore, type PaneSide } from "./shellStore";
import { LEFT_TABS } from "./tabs";

/** One command palette entry. */
export interface PaletteCommand {
  id: string;
  label: string;
  /** Section shown next to the label. */
  group: string;
  /**
   * Shortcut hint. The palette shows the keymap's chord for the command instead (keymap):
   * a hint here is the command's default chord in the keymap unless it is a built-in action.
   */
  shortcut?: string;
  /** The keymap action this command is (`ui/src/features/keymap/registry.ts`), if any. */
  action?: string;
  /** `false`: not bindable in the keymap (per-project entries). */
  bindable?: boolean;
  /** Extra words that match (not shown). */
  keywords?: string;
  run(): void;
}

const PANE_NAMES: Record<PaneSide, string> = { left: "browser", right: "inspector", bottom: "editor drawer" };

/**
 * The commands available now, built from the current app state (the palette calls it when
 * it opens). `transport` null: engine commands are left out.
 */
export function buildCommands(transport: EngineTransport | null, devices: ReadonlyArray<DeviceDescriptor>): PaletteCommand[] {
  const out: PaletteCommand[] = [];
  const shell = useShellStore.getState();
  const { project, transport: state } = useProjectStore.getState();
  const send = (c: Parameters<EngineTransport["send"]>[0]) =>
    void transport?.send(c).catch((e: unknown) => console.warn("[ethereal] command failed:", e));

  if (transport && project) {
    out.push(
      { id: "track:midi", group: "Tracks", label: "Add MIDI track", keywords: "new create", run: () => void addTrack(transport, "Midi") },
      { id: "track:audio", group: "Tracks", label: "Add audio track", keywords: "new create", run: () => void addTrack(transport, "Audio") },
      {
        id: "import:audio",
        group: "Tracks",
        label: "Import audio…",
        shortcut: "⌘I",
        keywords: "import file upload sample wav mp3 flac aiff open add",
        run: () => void openImportDialog(transport),
      },
      // media-references: relink missing samples, collect referenced ones.
      ...mediaRefCommands(transport, project),
      // templates: save / insert templates, new project from a template.
      ...templateCommands(project),
    );
    const target = deviceTargetTrack(project);
    for (const d of devices) {
      if (!canInsert(d, target)) continue;
      out.push({
        id: `device:${d.name}`,
        group: "Devices",
        label: `Add ${d.name}`,
        keywords: `device insert ${d.category} ${target?.name ?? ""}`,
        run: () => {
          const p = useProjectStore.getState().project;
          const t = p ? deviceTargetTrack(p) : undefined;
          if (p && t) void insertDeviceCommand(transport, p, t, d).then((c) => c && send(c));
        },
      });
    }
    out.push(
      {
        id: "transport:play",
        group: "Transport",
        label: state?.playing ? "Stop" : "Play",
        keywords: "play stop start pause toggle",
        shortcut: "Space",
        run: () => send(cmd("Transport", { type: "TogglePlay" })),
      },
      {
        id: "transport:record",
        group: "Transport",
        label: state?.recording ? "Stop recording" : "Record",
        keywords: "record arm",
        run: () => send(cmd("Recording", { type: "SetRecording", enabled: !state?.recording })),
      },
      {
        // capture-midi: the command replies InvalidState when nothing was played.
        id: "transport:capture",
        group: "Transport",
        label: "Capture MIDI",
        keywords: "capture midi record recent played notes take clip",
        run: () => void captureMidi(transport).catch((e: unknown) => console.warn("[ethereal] capture failed:", e)),
      },
      {
        id: "transport:loop",
        group: "Transport",
        label: state?.loop_enabled ? "Turn loop off" : "Turn loop on",
        keywords: "loop cycle toggle",
        run: () => send(cmd("Transport", { type: "SetLoopEnabled", enabled: !state?.loop_enabled })),
      },
      {
        id: "transport:metronome",
        group: "Transport",
        label: state?.metronome ? "Turn metronome off" : "Turn metronome on",
        keywords: "metronome click toggle",
        run: () => send(cmd("Transport", { type: "SetMetronome", enabled: !state?.metronome })),
      },
      { id: "edit:undo", group: "Edit", label: "Undo", shortcut: "⌘Z", run: () => send(cmd("Edit", { type: "Undo" })) },
      { id: "edit:redo", group: "Edit", label: "Redo", shortcut: "⇧⌘Z", run: () => send(cmd("Edit", { type: "Redo" })) },
      // undo-history: name the current step from anywhere (opens the History tab).
      {
        id: "history:checkpoint",
        group: "Edit",
        label: "Name checkpoint…",
        keywords: "history undo checkpoint mark bookmark snapshot",
        run: () => nameCurrentCheckpoint(),
      },
    );

    for (const t of tracksOrdered(project)) {
      out.push({
        id: `goto:${t.id}`,
        group: "Go to",
        label: `Go to track ${t.name}`,
        keywords: "select jump track",
        bindable: false,
        run: () => selectTrackEntity(t.id),
      });
    }
    const clips = Object.values(project.clips);
    if (clips.length) {
      out.push({
        id: "view:fit",
        group: "View",
        label: "Zoom to fit",
        keywords: "zoom fit all show whole song",
        run: () => {
          const all = Object.values(useProjectStore.getState().project?.clips ?? {});
          if (!all.length) return;
          const start = Math.min(...all.map((c) => c.start));
          const end = Math.max(...all.map((c) => c.start + c.length));
          arrangementView.getState().zoomToRange({ start, end }, 24);
        },
      });
    }
  }

  // join-flow: "Join shared project…" (paste an invite link).
  out.push(...joinPaletteCommands());

  const inSession = useCollabStore.getState().status.type === "Online";
  if (inSession) {
    out.push({
      id: "chat:focus",
      group: "Chat",
      label: "Chat: Focus input",
      keywords: "chat message collab session talk send",
      shortcut: "⇧⌘M",
      run: () => focusChat(),
    });
  }
  // ai-chat.
  out.push({
    id: "ai:ask",
    group: "AI",
    label: "Ask AI",
    keywords: "ai assistant claude chat agent llm prompt generate",
    shortcut: "⇧⌘A",
    run: () => openAiChat(),
  });
  for (const t of LEFT_TABS) {
    if (t.session && !inSession) continue;
    if (t.id === "ai") continue; // "Ask AI" above
    out.push({
      id: `panel:${t.id}`,
      group: "Panels",
      label: `Open ${t.label}`,
      keywords: "show panel browser rail",
      run: () => {
        const s = useShellStore.getState();
        if (!s.left.open || s.left.tab !== t.id) s.toggleLeft(t.id);
      },
    });
  }
  out.push({
    id: "panel:drawer",
    group: "Panels",
    label: shell.bottom.open ? "Close editor drawer" : "Open editor drawer",
    keywords: "piano roll warp automation editor toggle",
    run: () => useShellStore.getState().setOpen("bottom", !useShellStore.getState().bottom.open),
  });
  for (const side of ["left", "right", "bottom"] as const) {
    const pinned = shell[side].pinned;
    out.push({
      id: `pin:${side}`,
      group: "Panels",
      label: `${pinned ? "Unpin" : "Pin"} ${PANE_NAMES[side]}`,
      keywords: "pin unpin dock float",
      run: () => useShellStore.getState().setPinned(side, !useShellStore.getState()[side].pinned),
    });
  }
  // base-115: Share, Stop sharing, Leave, sharing settings.
  out.push(...shareCommands(transport, !!project));
  out.push({
    id: "audio-settings",
    group: "Appearance",
    label: "Audio settings…",
    keywords: "audio device output input microphone sample rate buffer latency driver preferences",
    run: () => openAudioSettings(),
  });
  out.push(
    {
      id: "keymap:open",
      group: "Appearance",
      label: "Keyboard shortcuts…",
      keywords: "keymap hotkeys keys bindings shortcuts rebind preset ableton",
      run: () => openKeymapEditor(),
    },
    {
      id: "keymap:print",
      group: "Appearance",
      label: "Print keyboard shortcuts",
      keywords: "keymap cheat sheet hotkeys print",
      run: () => printCheatSheet(),
    },
  );
  out.push({
    id: "input-settings",
    group: "Appearance",
    label: "Mouse and wheel settings…",
    keywords: "input mouse wheel scroll zoom trackpad sensitivity invert reverse direction middle button back forward preferences",
    run: () => openSettings("input"),
  });
  const dark = getTheme() === "dark";
  out.push({
    id: "theme",
    group: "Appearance",
    label: dark ? "Switch to light theme" : "Switch to dark theme",
    keywords: "theme dark light mode appearance",
    run: () => setTheme(dark ? "light" : "dark"),
  });
  return out;
}

/** templates: palette entries (save the project or the selected tracks, insert, new project). */
function templateCommands(project: Project): PaletteCommand[] {
  const dialogs = useTemplateDialog.getState();
  const selected = useArrangementUi.getState().selectedTracks;
  const first = [...selected].map((id) => project.tracks[id]).find((t) => t && t.kind !== "Master");
  const tracks = first ? tracksToSave(first.id, selected, project) : [];
  const out: PaletteCommand[] = [
    {
      id: "template:save-project",
      group: "Templates",
      label: "Save project as template…",
      keywords: "template project default new save",
      run: () => dialogs.open({ type: "save-project", name: project.settings.name }),
    },
    {
      id: "template:insert",
      group: "Templates",
      label: "Insert track template…",
      keywords: "template track add insert chain preset",
      run: () => dialogs.open({ type: "insert", placement: first ? placementAfter(project, first) : { parent: null, before: null } }),
    },
    {
      id: "template:new-project",
      group: "Templates",
      label: "New project from template…",
      keywords: "template project new create start",
      run: () => useProjectScreen.getState().showNew(),
    },
  ];
  if (tracks.length) {
    out.splice(1, 0, {
      id: "template:save-tracks",
      group: "Templates",
      label: tracks.length > 1 ? `Save ${tracks.length} tracks as template…` : `Save track “${first?.name ?? ""}” as template…`,
      keywords: "template track save chain",
      run: () => dialogs.open({ type: "save-tracks", tracks, name: tracks.length === 1 ? (first?.name ?? "") : "" }),
    });
  }
  return out;
}

/**
 * Match score of `query` in `text` (higher is better), or -1: every query character must
 * appear in order. Contiguous runs, word starts and an early match score higher.
 */
export function fuzzyScore(query: string, text: string): number {
  const q = query.toLowerCase().trim();
  if (!q) return 0;
  const t = text.toLowerCase();
  const sub = t.indexOf(q);
  if (sub >= 0) return 1000 - sub + (sub === 0 || t[sub - 1] === " " ? 200 : 0);
  let score = 0;
  let at = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === " ") continue;
    const i = t.indexOf(ch, at);
    if (i < 0) return -1;
    run = i === at ? run + 1 : 0;
    score += 10 + run * 5 + (i === 0 || t[i - 1] === " " ? 15 : 0) - Math.min(9, i - at);
    at = i + 1;
  }
  return score;
}
