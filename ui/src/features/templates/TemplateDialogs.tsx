import "./templates.css";
import { useState, type FormEvent, type MouseEvent, type ReactNode } from "react";
import { MoreHorizontal } from "lucide-react";
import type { TemplateInfo, TemplateKind } from "@/generated";
import { selectTrackEntity } from "@/features/arrangement/actions";
import { parseTags } from "@/features/presets/model";
import { useEngineCommands } from "@/features/transport-bar/engine";
import { NO_AUTOCORRECT, Button, Dialog, IconButton, TextInput } from "@/kit";
import { cmd } from "@/transport";
import { findByName, groupTemplates, insertTemplate, matchesQuery, message, templateMenu, useTemplateDialog, type TemplateDialog } from "./model";
import { useTemplates } from "./useTemplates";
import { Trans, useT } from "@/i18n";

/** The "…" button opening a row's menu at the button. */
function MoreButton({ template, onMenu }: { template: TemplateInfo; onMenu(e: MouseEvent): void }) {
  const t = useT();
  return (
    <IconButton
      size="sm"
      tone="ghost"
      className="eth-templates__more"
      label={t("project.actions.more", { name: template.name })}
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
  const t = useT();
  const { factory, user } = groupTemplates(templates);
  const row = (tpl: TemplateInfo) => (
    <li key={tpl.id} className="eth-templates__row" onContextMenu={onMenu ? (e) => onMenu(e, tpl) : undefined}>
      <button
        type="button"
        className="eth-templates__item"
        aria-current={selected === tpl.id || undefined}
        title={tpl.meta.description ?? undefined}
        onClick={() => onPick(tpl)}
      >
        {/* Factory templates' names, tags and descriptions are the engine's content: lowercase like all the app's text, in English (not translated yet). */}
        <span className={tpl.factory ? "eth-templates__name" : "eth-templates__name eth-user-text"} data-i18n-exempt={tpl.factory || undefined}>
          {tpl.name}
        </span>
        {tpl.default && <span className="eth-templates__badge">{t("common.default")}</span>}
        {tpl.meta.tags.length > 0 && (
          <span className={tpl.factory ? "eth-templates__tags" : "eth-templates__tags eth-user-text"} data-i18n-exempt={tpl.factory || undefined}>
            {tpl.meta.tags.join(" · ")}
          </span>
        )}
      </button>
      {onMenu && (!tpl.factory || tpl.kind === "Project") && <MoreButton template={tpl} onMenu={(e) => onMenu(e, tpl)} />}
    </li>
  );
  return (
    <div className="eth-templates__scroll" role="group" aria-label={label}>
      {leading && <ul className="eth-templates__list">{leading}</ul>}
      {factory.length > 0 && (
        <>
          <p className="eth-templates__group">{t("templates.list.factory")}</p>
          <ul className="eth-templates__list">{factory.map(row)}</ul>
        </>
      )}
      {user.length > 0 && (
        <>
          <p className="eth-templates__group">{t("templates.list.yours")}</p>
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
  const t = useT();
  const { transport } = useEngineCommands();
  const { templates, error: listError } = useTemplates("Tracks", true);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const shown = (templates ?? []).filter((tpl) => matchesQuery(tpl, query));
  const pick = (tpl: TemplateInfo) => {
    if (!transport || busy) return;
    setBusy(true);
    insertTemplate(transport, tpl.id, dialog.placement)
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
      title={t("templates.insert.title")}
      className="eth-template-dialog"
      footer={
        <Button tone="ghost" onClick={onClose}>
          {t("project.actions.cancel")}
        </Button>
      }
    >
      <div className="eth-template-dialog__form">
        <TextInput
          autoFocus
          {...NO_AUTOCORRECT}
          aria-label={t("templates.insert.search")}
          placeholder={t("templates.insert.search")}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        {templates && shown.length === 0 && <p className="eth-template-dialog__hint">{t("templates.insert.no_match")}</p>}
        <TemplateList label={t("templates.insert.list")} templates={shown} onPick={pick} onMenu={(e, tpl) => templateMenu(e, tpl)} />
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
  const t = useT();
  const { transport } = useEngineCommands();
  const kind: TemplateKind = dialog.type === "save-tracks" || dialog.type === "save-project" ? KIND_OF[dialog.type] : dialog.template.kind;
  const { templates } = useTemplates(kind, true);
  const [name, setName] = useState(
    dialog.type === "save-tracks" || dialog.type === "save-project" ? dialog.name : dialog.type === "rename" ? dialog.template.name : "",
  );
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
    if (dialog.type === "save-project")
      return run(() => transport.send(cmd("Template", { type: "SaveProject", name: trimmed, meta, overwrite: clash !== undefined })));
    if (clash) return;
    run(() => transport.send(cmd("Template", { type: "Rename", template: dialog.template.id, name: trimmed })));
  };

  const templateName = "template" in dialog ? <span className={dialog.template.factory ? undefined : "eth-user-text"}>{dialog.template.name}</span> : null;
  const title =
    dialog.type === "save-tracks" ? (
      t("templates.save_tracks.title", { count: dialog.tracks.length })
    ) : dialog.type === "save-project" ? (
      t("templates.save_project.title")
    ) : dialog.type === "rename" ? (
      <Trans k="templates.rename.title" values={{ name: templateName }} />
    ) : (
      <Trans k="project.actions.delete_named" values={{ name: templateName }} />
    );
  const action = saving
    ? clash
      ? t("templates.action.replace")
      : t("templates.action.save")
    : dialog.type === "rename"
      ? t("project.actions.rename")
      : t("project.actions.delete");
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
            {t("project.actions.cancel")}
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
        <p className="eth-template-dialog__text">{t("templates.delete.body")}</p>
      ) : (
        <form className="eth-template-dialog__form" onSubmit={submit}>
          {dialog.type === "save-tracks" && <p className="eth-template-dialog__hint">{t("templates.save_tracks.hint")}</p>}
          {dialog.type === "save-project" && <p className="eth-template-dialog__hint">{t("templates.save_project.hint")}</p>}
          <label className="eth-template-dialog__field">
            <span className="eth-template-dialog__label">{t("templates.field.name")}</span>
            <TextInput
              autoFocus
              value={name}
              invalid={dialog.type === "rename" && clash !== undefined}
              aria-label={t("templates.field.name_label")}
              placeholder={t("templates.field.name_placeholder")}
              maxLength={80}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          {saving && (
            <label className="eth-template-dialog__field">
              <span className="eth-template-dialog__label">{t("templates.field.tags")}</span>
              <TextInput
                value={tags}
                aria-label={t("templates.field.tags_label")}
                placeholder={t("templates.field.tags_placeholder")}
                onChange={(e) => setTags(e.target.value)}
              />
            </label>
          )}
          {clash && (
            <p className="eth-template-dialog__hint" role="status">
              <Trans k={saving ? "templates.clash.replace" : "templates.clash.exists"} values={{ name: <span className="eth-user-text">{clash.name}</span> }} />
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
