import { useCallback, useEffect, useState, type MouseEvent } from "react";
import { ChevronDown, Ellipsis, Save } from "lucide-react";
import type { Device, PresetInfo } from "@/generated";
import { useT } from "@/i18n";
import { Button, IconButton, Popover, TextInput, openContextMenu, type Placement } from "@/kit";
import { cmd, useTransport, useTransportEvent } from "@/transport";
import { groupPresets, loadPresetCommand, presetDeviceOf, useCurrentPresets } from "./model";
import { PresetDialogs, type PresetDialog } from "./PresetDialogs";

export interface PresetMenuProps {
  device: Device;
}

/** Rough panel size (search + list + footer): the side is picked up front, before the list loads. */
const PANEL_HEIGHT = 440;
const PANEL_WIDTH = 300;

/** Below the trigger unless there's more room above; aligned to the side with room. */
function placeFor(r: DOMRect): Placement {
  const below = window.innerHeight - r.bottom;
  const side = below >= PANEL_HEIGHT || below >= r.top ? "bottom" : "top";
  const align = window.innerWidth - r.left >= PANEL_WIDTH ? "start" : "end";
  return `${side}-${align}`;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

/**
 * The preset button of a device header (mounted by `DeviceView` for every device): shows
 * the preset loaded in this session, opens a searchable list (factory, then user presets)
 * where a click loads a preset (one undo step), and "Save preset…". User presets have a
 * context menu (also on their "…" button): Rename…, Delete….
 */
export function PresetMenu({ device }: PresetMenuProps) {
  // A missing device (placeholder) has no presets.
  if (device.kind.type === "Missing") return null;
  return <DevicePresetMenu device={device} />;
}

function DevicePresetMenu({ device }: PresetMenuProps) {
  const t = useT();
  const transport = useTransport();
  const current = useCurrentPresets((s) => s.current[device.id]);
  const setCurrent = useCurrentPresets((s) => s.set);
  const [open, setOpen] = useState(false);
  const [text, setText] = useState("");
  const [presets, setPresets] = useState<PresetInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [dialog, setDialog] = useState<PresetDialog | null>(null);
  const [placement, setPlacement] = useState<Placement>("bottom-start");
  const kind = presetDeviceOf(device);
  const kindKey = JSON.stringify(kind);

  const refresh = useCallback(() => {
    transport
      .send(
        cmd("Preset", {
          type: "List",
          device: JSON.parse(kindKey),
          text: null,
        }),
      )
      .then((r) => {
        if (r.type === "Presets") setPresets(r.presets);
      })
      .catch((e: unknown) => setError(message(e)));
  }, [transport, kindKey]);

  // List when opened, and again whenever the user presets change (any client).
  useEffect(() => {
    if (open || dialog) refresh();
  }, [open, dialog, refresh]);
  useTransportEvent((e) => {
    if (e.type === "Preset" && e.event.type === "Changed" && (open || dialog)) refresh();
  });
  const load = (p: PresetInfo, close: () => void) => {
    setError(null);
    transport
      .send(loadPresetCommand(device.id, p.preset))
      .then(() => {
        setCurrent(device.id, p.preset, p.name);
        close();
      })
      .catch((e: unknown) => setError(message(e)));
  };

  const setOpenState = (o: boolean) => {
    setOpen(o);
    if (!o) setText("");
  };
  const openDialog = (d: PresetDialog) => {
    setOpenState(false);
    setDialog(d);
  };

  const userMenu = (e: MouseEvent, p: PresetInfo) =>
    openContextMenu(e, [
      {
        label: t("presets.menu.rename"),
        onSelect: () => openDialog({ type: "rename", preset: p }),
      },
      "separator",
      {
        label: t("presets.menu.delete"),
        danger: true,
        onSelect: () => openDialog({ type: "delete", preset: p }),
      },
    ]);

  const q = text.trim().toLowerCase();
  const shown = (presets ?? []).filter(
    (p) => !q || p.name.toLowerCase().includes(q) || p.meta.tags.some((tag) => tag.includes(q)) || (p.meta.author ?? "").toLowerCase().includes(q),
  );
  const { factory, user } = groupPresets(shown);

  const row = (p: PresetInfo, close: () => void) => {
    const active = current && current.ref.source === p.preset.source && current.ref.id === p.preset.id;
    return (
      <li key={`${p.preset.source}:${p.preset.id}`} className="eth-presets__row" onContextMenu={p.preset.source === "User" ? (e) => userMenu(e, p) : undefined}>
        <button
          type="button"
          className="eth-presets__item"
          aria-current={active || undefined}
          title={p.meta.description ?? undefined}
          onClick={() => load(p, close)}
        >
          <span className={p.preset.source === "User" ? "eth-presets__name eth-user-text" : "eth-presets__name"}>{p.name}</span>
          {p.meta.tags.length > 0 && (
            <span className={p.preset.source === "User" ? "eth-presets__tags eth-user-text" : "eth-presets__tags"}>{p.meta.tags.join(" · ")}</span>
          )}
        </button>
        {p.preset.source === "User" && (
          <IconButton
            size="sm"
            tone="ghost"
            className="eth-presets__more"
            label={t("presets.menu.more", { name: p.name })}
            icon={<Ellipsis />}
            onClick={(e) => userMenu(e, p)}
          />
        )}
      </li>
    );
  };

  return (
    <>
      <Popover
        open={open}
        onOpenChange={setOpenState}
        placement={placement}
        aria-label={t("presets.menu.label", { name: device.name })}
        className="eth-presets"
        trigger={(tp) => (
          <Button
            {...tp}
            onClick={(e) => {
              if (!open) setPlacement(placeFor(e.currentTarget.getBoundingClientRect()));
              tp.onClick();
            }}
            size="sm"
            tone="ghost"
            className="eth-presets__trigger"
            aria-label={t("presets.menu.label", { name: device.name })}
            title={current ? t("presets.menu.current", { name: current.name }) : t("presets.menu.title")}
            onPointerDown={(e) => e.stopPropagation()}
          >
            <span className={current?.ref.source === "User" ? "eth-presets__current eth-user-text" : "eth-presets__current"}>
              {current?.name ?? t("presets.menu.button")}
            </span>
            <ChevronDown aria-hidden />
          </Button>
        )}
      >
        {(close) => (
          <div className="eth-presets__panel">
            <TextInput
              autoFocus
              size="sm"
              className="eth-presets__search"
              placeholder={t("presets.menu.search")}
              aria-label={t("presets.menu.search")}
              value={text}
              onChange={(e) => setText(e.target.value)}
            />
            <div className="eth-presets__scroll">
              {presets === null ? (
                <p className="eth-presets__empty">{t("presets.menu.loading")}</p>
              ) : shown.length === 0 ? (
                <p className="eth-presets__empty">{q ? t("presets.menu.no_match") : t("presets.menu.none")}</p>
              ) : (
                <>
                  {factory.length > 0 && (
                    <section aria-label={t("presets.menu.factory_label")}>
                      <h3 className="eth-presets__group">{t("presets.menu.factory")}</h3>
                      <ul className="eth-presets__list">{factory.map((p) => row(p, close))}</ul>
                    </section>
                  )}
                  {user.length > 0 && (
                    <section aria-label={t("presets.menu.user_label")}>
                      <h3 className="eth-presets__group">{t("presets.menu.user")}</h3>
                      <ul className="eth-presets__list">{user.map((p) => row(p, close))}</ul>
                    </section>
                  )}
                </>
              )}
            </div>
            {error && (
              <p className="eth-presets__error" role="alert">
                {error}
              </p>
            )}
            <footer className="eth-presets__footer">
              <Button
                size="sm"
                tone="default"
                onClick={() => {
                  close();
                  openDialog({ type: "save" });
                }}
              >
                <Save aria-hidden />
                {t("presets.menu.save")}
              </Button>
            </footer>
          </div>
        )}
      </Popover>
      <PresetDialogs device={device} dialog={dialog} presets={presets ?? []} onClose={() => setDialog(null)} />
    </>
  );
}
