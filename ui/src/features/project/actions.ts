// Project actions shared by the project screen and the command palette (base-114): save,
// save as, duplicate, export / import a `.ether` bundle, delete (or close and delete the open
// project). Every file operation is engine-side (`Command::Project`); the desktop passes the
// OS dialog's path to the engine, the web build downloads / uploads the bundle.
import type { ProjectSummary, ReplyValue } from "@/generated";
import { fetchDownload, saveFile } from "@/features/export/download";
import { notify } from "@/features/notifications";
import { withUpload, type UploadSource } from "@/features/remote/upload";
import { useProjectStore } from "@/state";
import { cmd, newProjectId, type EngineTransport } from "@/transport";
import { copyName, sortProjects, uniqueName } from "./projectNames";

/** The desktop transport's bundle dialogs (`TauriTransport`): absolute engine-machine paths. */
export interface BundleFileHost {
  pickBundleSavePath(defaultName: string): Promise<string | null>;
  pickBundleFile(): Promise<string | null>;
}

export function isBundleFileHost(t: EngineTransport | null | undefined): t is EngineTransport & BundleFileHost {
  const h = t as Partial<BundleFileHost> | null | undefined;
  return typeof h?.pickBundleSavePath === "function" && typeof h.pickBundleFile === "function";
}

/** File types offered by the web import picker. */
export const BUNDLE_ACCEPT = ".ether,application/zip,application/json";

const projects = () => useProjectStore.getState().projects;
const current = () => useProjectStore.getState().project;

export function saveProject(t: EngineTransport): Promise<ReplyValue> {
  return t.send(cmd("Project", { type: "Save" }));
}

/** Save the open project as a new one (media copied) and switch to it. */
export function saveProjectAs(t: EngineTransport, name: string): Promise<ReplyValue> {
  return t.send(
    cmd("Project", {
      type: "SaveAs",
      new_id: newProjectId(),
      name: uniqueName(name, projects()),
    }),
  );
}

/** Copy a stored project (the open one: its current state) without opening the copy. */
export async function duplicateProject(t: EngineTransport, id: string, name: string): Promise<ProjectSummary | null> {
  const copy = copyName(name, projects());
  const reply = await t.send(
    cmd("Project", {
      type: "Duplicate",
      id,
      new_id: newProjectId(),
      name: copy,
    }),
  );
  if (reply.type !== "Saved") return null;
  notify("Info", `Duplicated as “${reply.project.name}”`);
  return reply.project;
}

/**
 * Export a project as a `.ether` bundle: the desktop asks where (OS save dialog) and the engine
 * writes the file; the web build pulls the bytes and saves them as a download. `false` when
 * the dialog was dismissed.
 */
export async function exportProject(t: EngineTransport, id: string, name: string): Promise<boolean> {
  if (isBundleFileHost(t)) {
    const path = await t.pickBundleSavePath(`${name}.ether`);
    if (!path) return false;
    await t.send(cmd("Project", { type: "ExportBundle", id, path }));
    notify("Info", `Exported “${name}” to ${path}`);
    return true;
  }
  const reply = await t.send(cmd("Project", { type: "ExportBundle", id, path: null }));
  if (reply.type !== "Bundle") throw new Error(`unexpected reply ${reply.type}`);
  try {
    saveFile(reply.download.name, reply.download.mime, await fetchDownload(t, reply.download));
  } finally {
    void t.send(cmd("Export", { type: "Release", token: reply.download.token })).catch(() => undefined);
  }
  notify("Info", `Exported “${name}” as ${reply.download.name}`);
  return true;
}

/** The browser's file picker for one bundle. Resolves `null` when dismissed. */
export function pickBundleUpload(doc: Document = document): Promise<File | null> {
  return new Promise((resolve) => {
    const input = doc.createElement("input");
    input.type = "file";
    input.accept = BUNDLE_ACCEPT;
    input.hidden = true;
    input.setAttribute("data-testid", "project-import-input");
    let done = false;
    const finish = (file: File | null) => {
      if (done) return;
      done = true;
      input.remove();
      resolve(file);
    };
    input.addEventListener("change", () => finish(input.files?.[0] ?? null));
    input.addEventListener("cancel", () => finish(null));
    doc.body.appendChild(input);
    input.click();
  });
}

/**
 * Import a `.ether` bundle as a new stored project (not opened): the desktop's OS open
 * dialog, else the browser's file picker (uploaded). `file` skips the picker. A name already
 * in use gets a number (`Song 2`). `null` when nothing was picked.
 */
export async function importProject(t: EngineTransport, file?: UploadSource): Promise<ProjectSummary | null> {
  let reply: ReplyValue;
  if (!file && isBundleFileHost(t)) {
    const path = await t.pickBundleFile();
    if (!path) return null;
    reply = await t.send(
      cmd("Project", {
        type: "ImportBundle",
        new_id: newProjectId(),
        source: { type: "Path", path },
        name: null,
      }),
    );
  } else {
    const picked = file ?? (await pickBundleUpload());
    if (!picked) return null;
    reply = await withUpload(t, picked, undefined, {}, (upload) =>
      t.send(
        cmd("Project", {
          type: "ImportBundle",
          new_id: newProjectId(),
          source: { type: "Upload", upload },
          name: null,
        }),
      ),
    );
  }
  if (reply.type !== "Saved") throw new Error(`unexpected reply ${reply.type}`);
  let summary = reply.project;
  const others = projects().filter((p) => p.id !== summary.id);
  const unique = uniqueName(summary.name, others);
  if (unique !== summary.name) {
    await t.send(cmd("Project", { type: "Rename", id: summary.id, name: unique }));
    summary = { ...summary, name: unique };
  }
  return summary;
}

/** Open a stored project. */
export function openProject(t: EngineTransport, id: string): Promise<ReplyValue> {
  return t.send(cmd("Project", { type: "Open", id }));
}

/** Delete a stored project that is not open (the engine refuses the open one). */
export function deleteProject(t: EngineTransport, id: string): Promise<ReplyValue> {
  return t.send(cmd("Project", { type: "Delete", id }));
}

/**
 * The open project can't be deleted while open (`Project::Delete` refuses it): open the most
 * recently saved other project (or create an "Untitled" one), then delete it.
 */
export async function closeAndDelete(t: EngineTransport, id: string): Promise<void> {
  const next = sortProjects(projects()).find((p) => p.id !== id);
  if (next) await openProject(t, next.id);
  else {
    const others = projects().filter((p) => p.id !== id);
    await t.send(
      cmd("Project", {
        type: "Create",
        id: newProjectId(),
        name: uniqueName("Untitled", others),
      }),
    );
  }
  if (current()?.id === id) throw new Error("the project is still open");
  await deleteProject(t, id);
}
