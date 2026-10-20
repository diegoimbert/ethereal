// Grouping and pointer gestures for the shortcut lists (the keymap editor, the cheat sheet and
// the user docs' generated shortcuts page). Pure: no stores.
// eth-allow-case-file: the group ids are English identifiers, shown through their messages.
import { t, type MessageKey } from "@/i18n";
import { SCOPE_LABELS, type KeymapAction } from "./registry";

/** Group order in the editor and the cheat sheet; other groups follow, by name. */
export const GROUP_ORDER = ["Transport", "Edit", "Arrangement", "Time selection", "Piano roll", "Automation", "View", "App"];

/**
 * The shown name of a group id (keymap groups and command palette groups; the ids stay
 * English and stable). A group without a message shows its id.
 */
const GROUP_LABELS: Record<string, MessageKey> = {
  Transport: "shell.group.transport",
  Edit: "shell.group.edit",
  Arrangement: "shell.group.arrangement",
  "Time selection": "shell.group.time_selection",
  "Piano roll": "shell.group.piano_roll",
  Automation: "shell.group.automation",
  View: "shell.group.view",
  App: "shell.group.app",
  Collaboration: "shell.group.collaboration",
  Collab: "shell.group.collaboration",
  Session: "shell.group.session",
  Chat: "shell.group.chat",
  AI: "shell.group.ai",
  Tracks: "shell.group.tracks",
  Devices: "shell.group.devices",
  "Go to": "shell.group.go_to",
  Panels: "shell.group.panels",
  Appearance: "shell.group.appearance",
  Templates: "shell.group.templates",
  Project: "shell.group.project",
  Share: "shell.group.share",
  Help: "shell.group.help",
  Debug: "shell.group.debug",
};

/** Whether `group` is a group id (shown through its message, so its case is not the UI's). */
export const isGroupId = (group: string): boolean => Object.hasOwn(GROUP_LABELS, group);

/** A group's name in the current language. */
export function groupLabel(group: string): string {
  const key = Object.hasOwn(GROUP_LABELS, group) ? GROUP_LABELS[group] : undefined;
  return key ? t(key) : group;
}

/** An action's name in the current language. */
export function actionLabel(action: Pick<KeymapAction, "id" | "label" | "labelKey">): string {
  return action.labelKey ? t(action.labelKey) : (action.label ?? action.id);
}

/** Where an action works, in the current language (null: everywhere). */
export function scopesLabel(action: KeymapAction): string | null {
  return action.scopes.includes("global") ? null : action.scopes.map((s) => t(SCOPE_LABELS[s])).join(", ");
}

/** Actions grouped for display (groups in `GROUP_ORDER`, then by name; rows in registry order). */
export function groupActions(actions: ReadonlyArray<KeymapAction>): Array<{ group: string; actions: KeymapAction[] }> {
  const groups = new Map<string, KeymapAction[]>();
  for (const a of actions) {
    const list = groups.get(a.group) ?? [];
    list.push(a);
    groups.set(a.group, list);
  }
  const rank = (g: string) => {
    const i = GROUP_ORDER.indexOf(g);
    return i < 0 ? GROUP_ORDER.length : i;
  };
  return [...groups.entries()].sort(([a], [b]) => rank(a) - rank(b) || a.localeCompare(b)).map(([group, list]) => ({ group, actions: list }));
}

/**
 * Pointer gestures with modifiers in the arrangement (not rebindable: the keymap only holds
 * key chords). `keys` are modifiers in chord syntax. Ctrl+Shift also works on macOS, where
 * Ctrl-click is often a right click.
 */
/**
 * Modifiers of a point drag (automation lanes and clip envelopes; the tempo and expression lanes
 * share ⌥). The drag tooltip lists them with `hint` (hints come from here, not the views).
 * ⇧ + ⌥: ⇧'s whole increments win for the value, the time stays fine and off the grid.
 * `label` and `hint` are message keys.
 */
export const POINT_DRAG_GESTURES = {
  increments: { label: "shell.keymap.point_increments", keys: "Shift", hint: "shell.keymap.point_increments_hint" },
  precise: { label: "shell.keymap.point_precise", keys: "Alt", hint: "shell.keymap.point_precise_hint" },
  axis: { label: "shell.keymap.point_axis", keys: "Mod", hint: "shell.keymap.point_axis_hint" },
} as const satisfies Record<string, { label: MessageKey; keys: string; hint: MessageKey }>;

export const MOUSE_GESTURES: ReadonlyArray<{ label: MessageKey; keys: string }> = [
  { label: "shell.keymap.mouse_no_snap", keys: "Alt" },
  { label: "shell.keymap.mouse_copy", keys: "Mod" },
  { label: "shell.keymap.mouse_slip", keys: "Mod+Shift" },
  ...Object.values(POINT_DRAG_GESTURES).map(({ label, keys }) => ({ label, keys })),
];
