import { openAiChat } from "@/features/ai-chat";
import { docsCommands } from "@/features/docs/commands";
import { newsCommands } from "@/features/news";
import { openAudioSettings, openSettings } from "@/features/audio-settings";
import { captureMidi } from "@/features/capture";
import type { DeviceDescriptor, Project } from "@/generated";
import { addTrack, runClipAction, selectTrackEntity } from "@/features/arrangement/actions";
import { reverseNotesCommand, sendEdit } from "@/features/clip-editing/clipEditing";
import { itemSelection } from "@/timeline";
import { arrangementView } from "@/features/arrangement/uiStore";
import { openImportDialog } from "@/features/import";
import { openKeymapEditor, printCheatSheet, runAction } from "@/features/keymap";
import { actionLabel } from "@/features/keymap/groups";
import { registeredAction } from "@/features/keymap/registry";
import { engineText, foldForSearch, t as tr, type MessageKey } from "@/i18n";
import { useProjectScreen } from "@/features/project/screenStore";
import { placementAfter, tracksToSave, useTemplateDialog } from "@/features/templates";
import { useArrangementUi } from "@/features/arrangement/uiStore";
import { mediaRefCommands } from "@/features/media-refs";
import { canLearn } from "@/features/midi-learn/host";
import { useMidiLearnStore } from "@/features/midi-learn/store";
import { resolverCommands } from "@/features/import-resolver";
import { appProjectCommands, openProjectCommands } from "@/features/project/projectCommands";
import { playOrStopHere, playOrStopToStart } from "@/features/time-edits/marker";
import { refuseWhileLoading } from "@/features/transport-bar/loading";
import { refuseRecordWhileRendering } from "@/features/transport-bar/rendering";
import { nameCurrentCheckpoint } from "@/features/undo-history";
import { TOGGLE_ACTION as KEYBOARD_MIDI_ACTION, toggleKeyboardMidi, useKeyboardMidi } from "@/features/keyboard-midi";
import { useEngineServerShown } from "@/features/remote";
import { debugPanelCommands } from "@/features/debug";
import { sendUserInsert } from "@/features/devices/userInsert";
import { tracksOrdered, useEditorStore, useProjectStore } from "@/state";
import { getTheme, setTheme } from "@/theme";
import { cmd, type EngineTransport } from "@/transport";
import { canInsert } from "@/features/devices/chainUtils";
import { deviceTargetTrack, insertDeviceCommand } from "./deviceInsert";
import { drawerCanFullscreen, useShellStore, type PaneSide } from "@/state/shell";
import { railTabs } from "./tabs";
import { contributions } from "@/ext";

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
  /** The part of `label` that is user content (a track name…): shown in its own case (`eth-user-text`). */
  userText?: string;
  run(): void;
}

/** Editor actions the palette lists: each runs where its chord would. */
export const EDITOR_PALETTE_ACTIONS = [
  "edit.copy",
  "edit.cut",
  "edit.paste",
  "edit.duplicate",
  "edit.delete",
  "edit.selectAll",
  "edit.split",
  "clip.consolidate",
  "track.group",
  "track.ungroup",
  "pianoRoll.quantize",
] as const;

/** "Pin …" / "Unpin …" per pane. */
const PIN_LABELS: Record<PaneSide, { pin: MessageKey; unpin: MessageKey }> = {
  left: { pin: "shell.command.pin_left", unpin: "shell.command.unpin_left" },
  right: { pin: "shell.command.pin_right", unpin: "shell.command.unpin_right" },
  bottom: { pin: "shell.command.pin_bottom", unpin: "shell.command.unpin_bottom" },
};

/**
 * The commands available now, built from the current app state (the palette calls it when
 * it opens). `transport` null: engine commands are left out.
 */
export function buildCommands(transport: EngineTransport | null, devices: ReadonlyArray<DeviceDescriptor>): PaletteCommand[] {
  const out: PaletteCommand[] = [];
  const shell = useShellStore.getState();
  const { project, transport: state } = useProjectStore.getState();
  const send = (c: Parameters<EngineTransport["send"]>[0]) => void transport?.send(c).catch((e: unknown) => console.warn("[ethereal] command failed:", e));

  if (transport && project) {
    out.push(
      {
        id: "track:midi",
        group: "Tracks",
        label: tr("shell.action.track_add_midi"),
        keywords: "new create",
        run: () => void addTrack(transport, "Midi"),
      },
      {
        id: "track:audio",
        group: "Tracks",
        label: tr("shell.action.track_add_audio"),
        keywords: "new create",
        run: () => void addTrack(transport, "Audio"),
      },
      {
        id: "track:return",
        group: "Tracks",
        label: tr("shell.command.add_return_track"),
        keywords: "new create send bus fx aux",
        run: () => void addTrack(transport, "Return"),
      },
      {
        id: "import:audio",
        group: "Tracks",
        label: tr("shell.action.file_import"),
        action: "file.import",
        keywords: "import file upload sample wav mp3 flac aiff open add",
        run: () => void openImportDialog(transport),
      },
      // Relink missing samples, collect referenced ones.
      ...mediaRefCommands(transport, project),
      // Missing plugins and audio files (any time, not only after an import).
      ...resolverCommands(transport, project),
      // templates: save / insert templates, new project from a template.
      ...templateCommands(project),
    );
    const target = deviceTargetTrack(project);
    for (const d of devices) {
      if (!canInsert(d, target)) continue;
      out.push({
        id: `device:${d.name}`,
        group: "Devices",
        label: tr("shell.command.add_device", { name: engineText(d.name) }),
        keywords: `device insert ${d.name} ${d.category} ${target?.name ?? ""}`,
        run: () => {
          const p = useProjectStore.getState().project;
          const t = p ? deviceTargetTrack(p) : undefined;
          // A device added from the palette opens its window; a failure says so.
          if (p && t) void insertDeviceCommand(transport, p, t, d).then((c) => c && sendUserInsert(transport, c, d.name));
        },
      });
    }
    out.push(
      {
        id: "transport:play",
        group: "Transport",
        label: state?.playing ? tr("shell.command.stop_to_start") : tr("shell.command.play"),
        keywords: "play stop start pause toggle",
        action: "transport.play",
        run: () => {
          if (!refuseWhileLoading()) playOrStopToStart(transport);
        },
      },
      {
        id: "transport:stopToSelection",
        group: "Transport",
        // Stopped, both play; named apart so the two rows differ.
        label: state?.playing ? tr("shell.command.stop_here") : tr("shell.command.play_stop_here"),
        keywords: "play stop pause here stay",
        action: "transport.stopToSelection",
        run: () => {
          if (!refuseWhileLoading()) playOrStopHere(transport);
        },
      },
      {
        id: "transport:record",
        group: "Transport",
        label: state?.recording ? tr("shell.command.stop_recording") : tr("shell.action.transport_record"),
        keywords: "record arm",
        run: () => {
          if (refuseWhileLoading() || (!state?.recording && refuseRecordWhileRendering())) return;
          send(
            cmd("Recording", {
              type: "SetRecording",
              enabled: !state?.recording,
            }),
          );
        },
      },
      {
        // capture-midi: the command replies InvalidState when nothing was played.
        id: "transport:capture",
        group: "Transport",
        label: tr("shell.command.capture_midi"),
        keywords: "capture midi record recent played notes take clip",
        run: () => void captureMidi(transport).catch((e: unknown) => console.warn("[ethereal] capture failed:", e)),
      },
      {
        // keyboard-midi: play notes from the computer keyboard.
        id: "transport:computerKeyboard",
        group: "Transport",
        label: useKeyboardMidi.getState().enabled ? tr("shell.command.computer_keyboard_off") : tr("shell.command.computer_keyboard_on"),
        keywords: "computer midi keyboard qwerty play notes piano typing",
        action: KEYBOARD_MIDI_ACTION,
        run: toggleKeyboardMidi,
      },
      // midi-learn: MIDI mode also outlives the MIDI tab (turn it off from anywhere).
      ...(canLearn(transport)
        ? [
            {
              id: "transport:midiMode",
              group: "Transport",
              label: useMidiLearnStore.getState().enabled ? tr("shell.command.midi_mode_off") : tr("shell.command.midi_mode_on"),
              keywords: "midi learn mapping controller map knob",
              run: () => {
                const s = useMidiLearnStore.getState();
                s.setEnabled(!s.enabled);
              },
            },
          ]
        : []),
      {
        id: "transport:loop",
        group: "Transport",
        label: state?.loop_enabled ? tr("shell.command.loop_off") : tr("shell.command.loop_on"),
        keywords: "loop cycle toggle",
        run: () =>
          send(
            cmd("Transport", {
              type: "SetLoopEnabled",
              enabled: !state?.loop_enabled,
            }),
          ),
      },
      {
        id: "transport:metronome",
        group: "Transport",
        label: state?.metronome ? tr("shell.command.metronome_off") : tr("shell.command.metronome_on"),
        keywords: "metronome click toggle",
        run: () =>
          send(
            cmd("Transport", {
              type: "SetMetronome",
              enabled: !state?.metronome,
            }),
          ),
      },
      {
        id: "edit:undo",
        group: "Edit",
        label: tr("shell.action.edit_undo"),
        action: "edit.undo",
        run: () => send(cmd("Edit", { type: "Undo" })),
      },
      {
        id: "edit:redo",
        group: "Edit",
        label: tr("shell.action.edit_redo"),
        action: "edit.redo",
        run: () => send(cmd("Edit", { type: "Redo" })),
      },
      // Reverse: the selected notes when notes are selected in the piano roll, else the
      // selected clips (audio on / off, MIDI mirrored); the clip.reverse shortcut.
      {
        id: "edit:reverse",
        group: "Edit",
        label: tr("shell.command.reverse"),
        action: "clip.reverse",
        keywords: "backwards mirror flip retrograde clip notes midi audio",
        run: () => {
          const notes = [...itemSelection.getState().selected.note].map((id) => project.notes[id]).filter((n) => !!n);
          if (notes.length > 0 && useEditorStore.getState().clip) void sendEdit(transport, reverseNotesCommand(notes));
          else void runClipAction(transport, "reverse");
        },
      },
      // The editors' own actions (the chords and context menus have them), run by the
      // same key handlers in the editor focused last (`runAction`); labels from the keymap.
      ...EDITOR_PALETTE_ACTIONS.flatMap((id): PaletteCommand[] => {
        const a = registeredAction(id);
        return a ? [{ id: `editor:${id}`, group: "Edit", label: actionLabel(a), action: id, keywords: a.keywords, run: () => void runAction(id) }] : [];
      }),
      // undo-history: name the current step from anywhere (opens the History tab).
      {
        id: "history:checkpoint",
        group: "Edit",
        label: tr("shell.command.checkpoint"),
        keywords: "history undo checkpoint mark bookmark snapshot",
        run: () => nameCurrentCheckpoint(),
      },
    );

    for (const t of tracksOrdered(project)) {
      out.push({
        id: `goto:${t.id}`,
        group: "Go to",
        label: tr("shell.command.go_to_track", { name: t.name }),
        userText: t.name,
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
        label: tr("shell.action.view_zoom_to_fit"),
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

  // The project menu's actions, under its labels (`projectCommands.ts`).
  if (transport) out.push(...appProjectCommands(transport));
  if (transport && project) out.push(...openProjectCommands(transport, { id: project.id, name: project.settings.name }));
  // ai-chat.
  out.push({
    id: "ai:ask",
    group: "AI",
    label: tr("shell.tab.ai"),
    keywords: "ai assistant claude chat agent llm prompt generate alpha",
    action: "ai:ask",
    run: () => openAiChat(),
  });
  for (const t of railTabs()) {
    if (t.id === "ai") continue; // "Ask AI" above
    if (t.contributed?.isVisible && !t.contributed.isVisible()) continue;
    out.push({
      id: `panel:${t.id}`,
      group: "Panels",
      label: tr("shell.command.open_panel", { panel: t.label() }),
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
    label: shell.bottom.open ? tr("shell.command.drawer_close") : tr("shell.command.drawer_open"),
    keywords: "piano roll warp automation editor toggle",
    run: () => useShellStore.getState().setOpen("bottom", !useShellStore.getState().bottom.open),
  });
  if (shell.bottom.open && !shell.bottom.collapsed && drawerCanFullscreen(shell.bottom.tab)) {
    out.push({
      id: "panel:drawer-fullscreen",
      group: "Panels",
      label: shell.drawerFullscreen ? tr("shell.command.drawer_exit_fullscreen") : tr("shell.command.drawer_fullscreen"),
      keywords: "maximize expand piano roll beat editor full screen",
      run: () => useShellStore.getState().setDrawerFullscreen(!useShellStore.getState().drawerFullscreen),
    });
  }
  for (const side of ["left", "right", "bottom"] as const) {
    const pinned = shell[side].pinned;
    out.push({
      id: `pin:${side}`,
      group: "Panels",
      label: tr(pinned ? PIN_LABELS[side].unpin : PIN_LABELS[side].pin),
      keywords: "pin unpin dock float",
      run: () => useShellStore.getState().setPinned(side, !useShellStore.getState()[side].pinned),
    });
  }
  out.push({
    id: "audio-settings",
    group: "Appearance",
    label: tr("shell.command.audio_settings"),
    keywords: "audio device output input microphone sample rate buffer latency driver preferences",
    run: () => openAudioSettings(),
  });
  out.push(
    {
      id: "keymap:open",
      group: "Appearance",
      label: tr("shell.action.keymap_open"),
      keywords: "keymap hotkeys keys bindings shortcuts rebind preset ableton",
      run: () => openKeymapEditor(),
    },
    {
      id: "keymap:print",
      group: "Appearance",
      label: tr("shell.command.print_shortcuts"),
      keywords: "keymap cheat sheet hotkeys print",
      run: () => printCheatSheet(),
    },
  );
  // The engine server form is hidden until asked for (ether-server isn't released).
  out.push({
    id: "settings:engine-server",
    group: "Appearance",
    label: tr("shell.command.engine_server"),
    keywords: "engine server remote ether-server headless studio machine connect advanced",
    run: () => {
      useEngineServerShown.getState().show();
      openSettings("advanced");
    },
  });
  out.push({
    id: "input-settings",
    group: "Appearance",
    label: tr("shell.command.input_settings"),
    keywords: "input mouse wheel scroll zoom trackpad sensitivity invert reverse direction middle button back forward preferences",
    run: () => openSettings("input"),
  });
  const dark = getTheme() === "dark";
  out.push({
    id: "theme",
    group: "Appearance",
    label: dark ? tr("shell.command.theme_light") : tr("shell.command.theme_dark"),
    keywords: "theme dark light mode appearance",
    run: () => setTheme(dark ? "light" : "dark"),
  });
  // Extensions (`@/ext` "palette.commands").
  for (const f of contributions("palette.commands")) out.push(...f({ transport, project, devices }));
  // docs: the user documentation.
  out.push(...docsCommands());
  // news: "news…" (Help; the desktop Help menu's `app:news`).
  out.push(...newsCommands());
  // "Debug: <title>" per registered debug panel.
  out.push(...debugPanelCommands());
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
      id: "template:insert",
      group: "Templates",
      label: tr("shell.command.template_insert"),
      keywords: "template track add insert chain preset",
      run: () => dialogs.open({ type: "insert", placement: first ? placementAfter(project, first) : { parent: null, before: null } }),
    },
    {
      id: "template:new-project",
      group: "Templates",
      label: tr("shell.command.template_new_project"),
      keywords: "template project new create start",
      run: () => useProjectScreen.getState().showNew(),
    },
  ];
  if (tracks.length) {
    out.unshift({
      id: "template:save-tracks",
      group: "Templates",
      label:
        tracks.length > 1
          ? tr("shell.command.template_save_tracks", { count: tracks.length })
          : tr("shell.command.template_save_track", { name: first?.name ?? "" }),
      userText: tracks.length > 1 ? undefined : first?.name,
      keywords: "template track save chain",
      run: () => dialogs.open({ type: "save-tracks", tracks, name: tracks.length === 1 ? (first?.name ?? "") : "" }),
    });
  }
  return out;
}

/**
 * Match score of `query` in `text` (higher is better), or -1: every query character must
 * appear in order (case and accents ignored: "creer" finds "créer"). Contiguous runs, word starts and an early match score higher.
 */
export function fuzzyScore(query: string, text: string): number {
  const q = foldForSearch(query).trim();
  if (!q) return 0;
  const t = foldForSearch(text);
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
