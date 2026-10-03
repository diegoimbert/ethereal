import "./templates.css";
import { useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { MoreHorizontal } from "lucide-react";
import type { TemplateInfo, TemplateKind } from "@/generated";
import { selectTrackEntity } from "@/features/arrangement/actions";
import { parseTags } from "@/features/presets/model";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { Button, Dialog, IconButton, TextInput } from "@/kit";
import { cmd } from "@/transport";
import { findByName, groupTemplates, insertTemplate, matchesQuery, message, templateMenu, useTemplateDialog, type TemplateDialog } from "./model";
import { useTemplates } from "./useTemplates";

/** The "…" button opening a row's menu at the button. */
function MoreButton({ template, onMenu }: { template: TemplateInfo; onMenu(e: MouseEvent): void }) {
  return (
    <IconButton
      size="sm"
      tone="ghost"
      className="eth-templates__more"
      label={`More actions for ${template.name}`}
      icon={<MoreHorizontal aria-hidden />}
      onClick={(e) => {
        e.stopPropagation();
        const r = e.currentTarget.getBoundingClientRect();
        onMenu({ clientX: r.left, clientY: r.bottom, preventDefault: () => undefined, stopPropagation: () => undefined } as MouseEvent);
      }}
    />
  );
}

export interface TemplateListProps {
  templates: ReadonlyArray<TemplateInfo>;
  /** Highlighted row (`aria-current`). */
  selected?: string | null;
  onPick(t: TemplateInfo): void;
  onMenu?(e: MouseEvent, t: TemplateInfo): void;
  /** Rows before the groups (e.g. "Empty project"). */
  leading?: ReactNode;
  label: string;
}

/** Factory then user templates, each row a button (+ a menu on user rows). */
export function TemplateList({ templates, selected, onPick, onMenu, leading, label }: TemplateListProps) {
  const { factory, user } = groupTemplates(templates);
  const row = (t: TemplateInfo) => (
    <li key={t.id} className="eth-templates__row" onContextMenu={onMenu ? (e) => onMenu(e, t) : undefined}>
      <button type="button" className="eth-templates__item" aria-current={selected === t.id || undefined} title={t.meta.description ?? undefined} onClick={() => onPick(t)}>
        <span className="eth-templates__name">{t.name}</span>
        {t.default && <span className="eth-templates__badge">Default</span>}
        {t.meta.tags.length > 0 && <span className="eth-templates__tags">{t.meta.tags.join(" · ")}</span>}
      </button>
      {onMenu && (!t.factory || t.kind === "Project") && <MoreButton template={t} onMenu={(e) => onMenu(e, t)} />}
    </li>
  );
  return (
    <div className="eth-templates__scroll" role="group" aria-label={label}>
      {leading && <ul className="eth-templates__list">{leading}</ul>}
      {factory.length > 0 && (
        <>
          <p className="eth-templates__group">Factory</p>
          <ul className="eth-templates__list">{factory.map(row)}</ul>
        </>
      )}
      {user.length > 0 && (
        <>
          <p className="eth-templates__group">Your templates</p>
          <ul className="eth-templates__list">{user.map(row)}</ul>
        </>
      )}
    </div>
  );
}

/**
 * The template dialogs (save tracks / project, insert, rename, delete), driven by
 * `useTemplateDialog`. Mounted once (by the project menu).
 */
export function TemplateDialogs() {
  const dialog = useTemplateDialog((s) => s.dialog);
  const close = useTemplateDialog((s) => s.close);
  const key = dialog ? `${dialog.type}:${"template" in dialog ? dialog.template.id : ""}` : "none";
  if (!dialog) return <Dialog open={false} onClose={close} title="" />;
  if (dialog.type === "insert") return <InsertDialog key={key} dialog={dialog} onClose={close} />;
  return <EditDialog key={key} dialog={dialog} onClose={close} />;
}

function InsertDialog({ dialog, onClose }: { dialog: Extract<TemplateDialog, { type: "insert" }>; onClose(): void }) {
  const { transport } = useEngineCommands();
  const { templates, error: listError } = useTemplates("Tracks", true);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const shown = (templates ?? []).filter((t) => matchesQuery(t, query));
  const pick = (t: TemplateInfo) => {
    if (!transport || busy) return;
    setBusy(true);
    insertTemplate(transport, t.id, dialog.placement)
      .then((track) => {
        selectTrackEntity(track);
        onClose();
      })
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };
  return (
    <Dialog
      open
      onClose={onClose}
      title="Insert track template"
      className="eth-template-dialog"
      footer={
        <Button tone="ghost" onClick={onClose}>
          Cancel
        </Button>
      }
    >
      <div className="eth-template-dialog__form">
        <TextInput autoFocus aria-label="Search templates" placeholder="Search templates" value={query} onChange={(e) => setQuery(e.target.value)} />
        {templates && shown.length === 0 && <p className="eth-template-dialog__hint">No templates match.</p>}
        <TemplateList label="Track templates" templates={shown} onPick={pick} onMenu={(e, t) => templateMenu(e, t)} />
        {(error ?? listError) && (
          <p className="eth-template-dialog__error" role="alert">
            {error ?? listError}
          </p>
        )}
      </div>
    </Dialog>
  );
}

const KIND_OF: Record<"save-tracks" | "save-project", TemplateKind> = { "save-tracks": "Tracks", "save-project": "Project" };

function EditDialog({ dialog, onClose }: { dialog: Exclude<TemplateDialog, { type: "insert" }>; onClose(): void }) {
  const { transport } = useEngineCommands();
  const kind: TemplateKind = dialog.type === "save-tracks" || dialog.type === "save-project" ? KIND_OF[dialog.type] : dialog.template.kind;
  const { templates } = useTemplates(kind, true);
  const [name, setName] = useState(dialog.type === "save-tracks" || dialog.type === "save-project" ? dialog.name : dialog.type === "rename" ? dialog.template.name : "");
  const [tags, setTags] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const saving = dialog.type === "save-tracks" || dialog.type === "save-project";
  const trimmed = name.trim();
  const clash = dialog.type === "delete" ? undefined : findByName(templates ?? [], kind, trimmed, dialog.type === "rename" ? dialog.template.id : undefined);

  const run = (body: () => Promise<unknown>) => {
    if (!transport || busy) return;
    setBusy(true);
    setError(null);
    body()
      .then(onClose)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (!transport) return;
    if (dialog.type === "delete") return run(() => transport.send(cmd("Template", { type: "Delete", template: dialog.template.id })));
    if (!trimmed) return;
    const meta = { tags: parseTags(tags), author: null, description: null };
    if (dialog.type === "save-tracks")
      return run(() => transport.send(cmd("Template", { type: "SaveTracks", tracks: dialog.tracks, name: trimmed, meta, overwrite: clash !== undefined })));
    if (dialog.type === "save-project") return run(() => transport.send(cmd("Template", { type: "SaveProject", name: trimmed, meta, overwrite: clash !== undefined })));
    if (clash) return;
    run(() => transport.send(cmd("Template", { type: "Rename", template: dialog.template.id, name: trimmed })));
  };

  const title =
    dialog.type === "save-tracks"
      ? dialog.tracks.length > 1
        ? `Save ${dialog.tracks.length} tracks as a template`
        : "Save track as a template"
      : dialog.type === "save-project"
        ? "Save project as a template"
        : dialog.type === "rename"
          ? `Rename “${dialog.template.name}”`
          : `Delete “${dialog.template.name}”?`;
  const action = saving ? (clash ? "Replace" : "Save") : dialog.type === "rename" ? "Rename" : "Delete";
  const danger = dialog.type === "delete" || (saving && clash !== undefined);

  return (
    <Dialog
      open
      onClose={onClose}
      title={title}
      className="eth-template-dialog"
      footer={
        <>
          <Button tone="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            tone={danger ? "danger" : "accent"}
            disabled={busy || !transport || (dialog.type !== "delete" && (!trimmed || (dialog.type === "rename" && clash !== undefined)))}
            onClick={() => submit()}
          >
            {action}
          </Button>
        </>
      }
    >
      {dialog.type === "delete" ? (
        <p className="eth-template-dialog__text">The template is removed from your user library. This can’t be undone.</p>
      ) : (
        <form className="eth-template-dialog__form" onSubmit={submit}>
          {dialog.type === "save-tracks" && (
            <p className="eth-template-dialog__hint">Devices, racks, modulation, sends between these tracks and their automation are saved. Clips are not.</p>
          )}
          {dialog.type === "save-project" && <p className="eth-template-dialog__hint">The whole project is saved, with its samples. New projects can start from it.</p>}
          <label className="eth-template-dialog__field">
            <span className="eth-template-dialog__label">Name</span>
            <TextInput
              autoFocus
              value={name}
              invalid={dialog.type === "rename" && clash !== undefined}
              aria-label="Template name"
              placeholder="My template"
              maxLength={80}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          {saving && (
            <label className="eth-template-dialog__field">
              <span className="eth-template-dialog__label">Tags</span>
              <TextInput value={tags} aria-label="Template tags" placeholder="vocal, live" onChange={(e) => setTags(e.target.value)} />
            </label>
          )}
          {clash && (
            <p className="eth-template-dialog__hint" role="status">
              {saving ? `“${clash.name}” exists: saving replaces it.` : `A template named “${clash.name}” already exists.`}
            </p>
          )}
          <button type="submit" hidden />
        </form>
      )}
      {error && (
        <p className="eth-template-dialog__error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
