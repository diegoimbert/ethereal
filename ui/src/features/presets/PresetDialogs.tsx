import { useState, type FormEvent } from "react";
import type { Device, PresetInfo } from "@/generated";
import { Button, Dialog, TextInput } from "@/kit";
import { cmd, useTransport } from "@/transport";
import { findByName, parseTags, useCurrentPresets } from "./model";

export type PresetDialog = { type: "save" } | { type: "rename"; preset: PresetInfo } | { type: "delete"; preset: PresetInfo };

export interface PresetDialogsProps {
  device: Device;
  dialog: PresetDialog | null;
  /** The device type's presets (to spot name clashes before sending). */
  presets: ReadonlyArray<PresetInfo>;
  onClose(): void;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * Save (name + tags; an existing user preset of that name is replaced after a second
 * click on "Replace"), Rename and Delete dialogs of the preset menu.
 */
export function PresetDialogs({ device, dialog, presets, onClose }: PresetDialogsProps) {
  const transport = useTransport();
  const current = useCurrentPresets((s) => s.current[device.id]);
  const setCurrent = useCurrentPresets((s) => s.set);
  const replace = useCurrentPresets((s) => s.replace);
  const [name, setName] = useState("");
  const [tags, setTags] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  // Fresh fields whenever a (different) dialog opens (state adjusted during render).
  const key = dialog ? `${dialog.type}:${"preset" in dialog ? dialog.preset.preset.id : ""}` : null;
  const [openedKey, setOpenedKey] = useState<string | null>(null);
  if (key !== openedKey) {
    setOpenedKey(key);
    if (dialog) {
      setError(null);
      setBusy(false);
      if (dialog.type === "save") {
        setName(current?.ref.source === "User" ? current.name : "");
        const info = current && presets.find((p) => p.preset.source === current.ref.source && p.preset.id === current.ref.id);
        setTags(info?.meta.tags.join(", ") ?? "");
      } else {
        setName(dialog.preset.name);
      }
    }
  }

  const trimmed = name.trim();
  const clash =
    dialog?.type === "save" ? findByName(presets, trimmed) : dialog?.type === "rename" ? findByName(presets, trimmed, dialog.preset.preset) : undefined;

  const run = (send: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    send()
      .then(onClose)
      .catch((e: unknown) => setError(message(e)))
      .finally(() => setBusy(false));
  };

  const submit = (e?: FormEvent) => {
    e?.preventDefault();
    if (!dialog || busy) return;
    if (dialog.type === "delete") {
      const p = dialog.preset;
      return run(async () => {
        await transport.send(cmd("Preset", { type: "Delete", preset: p.preset }));
        replace(p.preset, null);
      });
    }
    if (!trimmed) return;
    if (dialog.type === "save") {
      return run(async () => {
        const r = await transport.send(
          cmd("Preset", {
            type: "Save",
            device: device.id,
            name: trimmed,
            meta: { tags: parseTags(tags), author: null, description: null },
            overwrite: clash !== undefined,
          }),
        );
        if (r.type === "Preset") setCurrent(device.id, r.preset.preset, r.preset.name);
      });
    }
    if (clash) return;
    const p = dialog.preset;
    run(async () => {
      const r = await transport.send(cmd("Preset", { type: "Rename", preset: p.preset, name: trimmed }));
      if (r.type === "Preset") replace(p.preset, { ref: r.preset.preset, name: r.preset.name });
    });
  };

  const title =
    dialog?.type === "save"
      ? `Save preset for ${device.name}`
      : dialog?.type === "rename"
        ? `Rename “${dialog.preset.name}”`
        : dialog?.type === "delete"
          ? `Delete “${dialog.preset.name}”?`
          : "";
  const action = dialog?.type === "save" ? (clash ? "Replace" : "Save") : dialog?.type === "rename" ? "Rename" : "Delete";

  return (
    <Dialog
      open={dialog !== null}
      onClose={onClose}
      title={title}
      className="eth-preset-dialog"
      footer={
        <>
          <Button tone="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            tone={dialog?.type === "delete" || (dialog?.type === "save" && clash) ? "danger" : "accent"}
            disabled={busy || (dialog?.type !== "delete" && (!trimmed || (dialog?.type === "rename" && clash !== undefined)))}
            onClick={() => submit()}
          >
            {action}
          </Button>
        </>
      }
    >
      {dialog?.type === "delete" ? (
        <p className="eth-preset-dialog__text">The preset file is removed from your user library. This can’t be undone.</p>
      ) : (
        <form className="eth-preset-dialog__form" onSubmit={submit}>
          <label className="eth-preset-dialog__field">
            <span className="eth-preset-dialog__label">Name</span>
            <TextInput
              autoFocus
              value={name}
              invalid={dialog?.type === "rename" && clash !== undefined}
              aria-label="Preset name"
              placeholder="My preset"
              maxLength={80}
              onChange={(e) => setName(e.target.value)}
            />
          </label>
          {dialog?.type === "save" && (
            <label className="eth-preset-dialog__field">
              <span className="eth-preset-dialog__label">Tags</span>
              <TextInput value={tags} aria-label="Preset tags" placeholder="pad, warm" onChange={(e) => setTags(e.target.value)} />
            </label>
          )}
          {clash && (
            <p className="eth-preset-dialog__hint" role="status">
              {dialog?.type === "save" ? `“${clash.name}” exists: saving replaces it.` : `A preset named “${clash.name}” already exists.`}
            </p>
          )}
          {/* Enter submits. */}
          <button type="submit" hidden />
        </form>
      )}
      {error && (
        <p className="eth-preset-dialog__error" role="alert">
          {error}
        </p>
      )}
    </Dialog>
  );
}
