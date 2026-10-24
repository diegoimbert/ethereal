/**
 * The project actions, once: the project menu (`ProjectMenuPanel`) and the command palette
 * (`ui/src/app/shell/commands.ts`) both list these, so the two show the same actions under the
 * same labels, and every one of them is a palette command, so the keymap can bind it.
 *
 * The menu decides its layout (groups, submenus, the scale picker, the in-place delete
 * confirmation); the labels and what an action does live here.
 */
import { notify } from "@/features/notifications";
import { openSettings } from "@/features/audio-settings";
import { errorMessage } from "@/features/transport-bar/engine";
import { useExportAudio } from "@/features/export";
import { canReveal, reveal } from "@/features/reveal";
import { openSnapshotPrompt, openVersions } from "@/features/versions/store";
import { useTemplateDialog } from "@/features/templates/model";
import { cmd, type EngineTransport } from "@/transport";
import { closeAndBin, duplicateProject, importProject, saveProject } from "./actions";
import { createDraftProject } from "./drafts";
import { useExportProject } from "./exportProjectStore";
import { guardLeave } from "./leaveGuard";
import { useProjectScreen } from "./screenStore";
import { t } from "@/i18n";
import { contributions } from "@/ext";

export interface ProjectCommand {
  id: string;
  /** The palette label, and the menu's (top level). */
  label: string;
  /** The label inside a menu submenu that names the rest ("export to another DAW" ▸ "Ableton Live Set (.als)…"). */
  short?: string;
  group: string;
  keywords?: string;
  /** Tooltip in the menu. */
  title?: string;
  /** The keymap action this is (its chord is the hint). */
  action?: string;
  run(): void;
}

/** Run a project action; a failure shows as an error toast. */
function attempt(action: () => Promise<unknown>): void {
  action().catch((e: unknown) => notify("Error", errorMessage(e)));
}

/** Move the open project to the bin (switching to another one first); the toast restores it. */
export function binOpenProject(transport: EngineTransport, id: string, name: string): void {
  // Restorable: no "keep or discard?" for an untitled one either.
  attempt(() => guardLeave(() => closeAndBin(transport, id, name), t("project.leave.delete"), { untitled: false }));
}

/** The extensions' project commands (`@/ext` "project.commands") for `project` (`null`: app-level ones). */
function contributed(transport: EngineTransport, project: { id: string; name: string } | null): ProjectCommand[] {
  return contributions("project.commands").flatMap((f) => [...f({ transport, project, attempt, guardLeave })]);
}

/** The actions available with no project open (and with one). */
export function appProjectCommands(transport: EngineTransport): ProjectCommand[] {
  return [
    {
      // The settings window, on the page shown last (like the top bar's gear).
      id: "app:settings",
      group: "Appearance",
      label: t("shell.command.settings"),
      keywords: "settings preferences options configure gear",
      run: () => openSettings(),
    },
    {
      id: "project:new",
      group: "Project",
      label: t("project.new_project"),
      keywords: "create start empty song",
      run: () => attempt(() => guardLeave(() => createDraftProject(transport), t("project.leave.create"))),
    },
    {
      id: "project:open",
      group: "Project",
      label: t("project.cmd.open"),
      keywords: "projects recent switch load",
      run: () => useProjectScreen.getState().show(),
    },
    // Desktop: the projects folder in the OS file manager.
    ...(canReveal(transport)
      ? [
          {
            id: "project:show-projects-folder",
            group: "Project",
            label: t("project.cmd.show_projects_folder"),
            keywords: "projects folder finder explorer file manager reveal open disk",
            run: () => void reveal(transport, { type: "Projects" }),
          },
        ]
      : []),
    ...contributed(transport, null),
  ];
}

/** The open project's actions, by id (see the module doc). */
export function openProjectCommands(transport: EngineTransport, project: { id: string; name: string }): ProjectCommand[] {
  const { id, name } = project;
  return [
    {
      id: "project:save",
      group: "Project",
      label: t("project.cmd.save"),
      action: "project.save",
      keywords: "save project store",
      run: () => attempt(() => saveProject(transport)),
    },
    {
      id: "project:save-as",
      group: "Project",
      label: t("project.cmd.save_as"),
      keywords: "save copy new name project",
      run: () => useProjectScreen.getState().show("saveAs"),
    },
    {
      id: "project:duplicate",
      group: "Project",
      label: t("project.cmd.duplicate"),
      keywords: "copy clone project",
      run: () => attempt(() => duplicateProject(transport, id, name)),
    },
    {
      id: "project:rename",
      group: "Project",
      label: t("project.cmd.rename"),
      keywords: "name title project",
      run: () => useProjectScreen.getState().rename(),
    },
    ...(canReveal(transport)
      ? [
          {
            id: "project:show-folder",
            group: "Project",
            label: t("project.cmd.show_project_folder"),
            keywords: "project folder finder explorer file manager reveal disk",
            run: () => void reveal(transport, { type: "Project", id }),
          },
        ]
      : []),
    {
      id: "template:save-project",
      group: "Templates",
      label: t("project.cmd.save_template"),
      keywords: "template project default new save",
      run: () => useTemplateDialog.getState().open({ type: "save-project", name }),
    },
    {
      id: "project:import",
      group: "Project",
      label: t("project.cmd.import"),
      keywords: "import bundle .ether file upload open project",
      run: () =>
        attempt(async () => {
          const summary = await importProject(transport);
          if (summary) await guardLeave(() => transport.send(cmd("Project", { type: "Open", id: summary.id })));
        }),
    },
    {
      id: "project:export",
      group: "Project",
      label: t("project.cmd.export"),
      keywords: "export bundle .ether file download backup share project",
      run: () => useExportProject.getState().show(id, name),
    },
    {
      id: "project:export-audio",
      group: "Project",
      label: t("project.cmd.export_audio"),
      action: "project.exportAudio",
      keywords: "export render bounce mixdown wav mp3 flac audio file stems",
      run: () => useExportAudio.getState().show(),
    },
    {
      id: "project:versions",
      group: "Project",
      label: t("project.cmd.versions"),
      keywords: "versions history backups autosave restore snapshots recover",
      run: () => openVersions(),
    },
    // A named version now (the keymap's `project.snapshot`).
    {
      id: "project:snapshot",
      group: "Project",
      label: t("project.cmd.snapshot"),
      action: "project.snapshot",
      keywords: "snapshot version named backup checkpoint save point",
      run: () => openSnapshotPrompt(),
    },
    {
      id: "project:delete",
      group: "Project",
      label: t("project.cmd.delete"),
      keywords: "delete remove trash bin project",
      run: () => binOpenProject(transport, id, name),
    },
    ...contributed(transport, project),
  ];
}

/** `commands` by id (the menu picks its items from it). */
export function byId<C extends { id: string }>(commands: ReadonlyArray<C>): Record<string, C> {
  return Object.fromEntries(commands.map((c) => [c.id, c]));
}
