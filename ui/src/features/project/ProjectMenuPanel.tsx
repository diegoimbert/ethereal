/**
 * The project menu (the top bar's menu button, owner request), in groups: New project /
 * Open project…; Save as template…; the project scale; Import project… / Export project…
 * (a dialog: pack media or not) / Export audio…; Versions… / Save snapshot…;
 * Delete project… (confirmed in place). Extensions add commands and submenus
 * (`@/ext` "project.commands", "project.menu").
 */
import clsx from "clsx";
import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { menuItems, menuKeyDown, useFocusFirstItem } from "@/kit";
import { ChevronRight } from "lucide-react";
import { contributions, useContributions, type ProjectMenuEntry } from "@/ext";
import { type EngineCommands } from "@/features/transport-bar/engine";
import { ProjectScale } from "@/features/scale/ProjectScale";
import { useProjectStore } from "@/state";
import { openResolver, resolveMissingLabel, useMissingCount } from "@/features/import-resolver";
import { appProjectCommands, byId, deleteOpenProject, openProjectCommands } from "./projectCommands";
import { docsCommands } from "@/features/docs/commands";
import { Trans, useT } from "@/i18n";

/** Which submenu of the project menu is open (one at a time), shared by its items. */
const OpenSub = createContext<{ open: string | null; setOpen(label: string | null): void }>({ open: null, setOpen: () => {} });

function Item({ children, danger, title, onClick }: { children: ReactNode; danger?: boolean; title?: string; onClick(): void }) {
  const sub = useContext(OpenSub);
  return (
    <button
      type="button"
      role="menuitem"
      title={title}
      className={clsx("eth-menu__item", danger && "eth-menu__item--danger")}
      onClick={onClick}
      // Reaching a plain item of the top level closes an open submenu.
      onMouseEnter={(e) => isTopLevel(e.currentTarget) && sub.setOpen(null)}
      onFocus={(e) => isTopLevel(e.currentTarget) && sub.setOpen(null)}
    >
      <span className="eth-menu__label">{children}</span>
    </button>
  );
}

const isTopLevel = (el: HTMLElement) => !el.closest(".eth-project-menu__submenu");

const Separator = () => <div className="eth-menu__separator" role="separator" />;

/** Grace period for the pointer to cross from an item to its submenu. */
const SUBMENU_CLOSE_MS = 150;

/**
 * A menu item that opens `children` as a submenu at its side: on hover, a click, Enter, Space
 * or →. Inside it ↑/↓ move, ← or Escape close it back to its item. One submenu at a time.
 */
function SubMenu({ label, tour, children }: { label: string; tour?: string; children: ReactNode }) {
  const sub = useContext(OpenSub);
  const open = sub.open === label;
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const panel = useRef<HTMLDivElement>(null);
  const item = useRef<HTMLButtonElement>(null);
  useEffect(() => () => clearTimeout(timer.current), []);
  const show = () => {
    clearTimeout(timer.current);
    sub.setOpen(label);
  };
  const hide = () => {
    clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      if (!panel.current?.contains(document.activeElement)) sub.setOpen(null);
    }, SUBMENU_CLOSE_MS);
  };
  const openAndFocus = () => {
    show();
    requestAnimationFrame(() => menuItems(panel.current)[0]?.focus());
  };
  return (
    <div className="eth-project-menu__sub" data-tour={tour} onMouseEnter={show} onMouseLeave={hide}>
      <button
        ref={item}
        type="button"
        role="menuitem"
        aria-haspopup="menu"
        aria-expanded={open}
        className={clsx("eth-menu__item", open && "eth-menu__item--open")}
        onClick={(e) => {
          // A keyboard click (Enter / Space) also moves focus into the submenu.
          if (e.detail === 0) openAndFocus();
          else sub.setOpen(open ? null : label);
        }}
        onKeyDown={(e) => {
          if (e.key !== "ArrowRight") return;
          e.preventDefault();
          openAndFocus();
        }}
      >
        <span className="eth-menu__label">{label}</span>
        <ChevronRight className="eth-project-menu__chevron" aria-hidden />
      </button>
      {open && (
        <div
          ref={panel}
          className="eth-menu eth-project-menu__submenu"
          role="menu"
          aria-label={label}
          onKeyDown={(e) => {
            if (menuKeyDown(e, panel.current)) return;
            if (e.key !== "ArrowLeft" && e.key !== "Escape") return;
            e.preventDefault();
            e.stopPropagation();
            sub.setOpen(null);
            item.current?.focus();
          }}
        >
          {children}
        </div>
      )}
    </div>
  );
}

export function ProjectMenuPanel({ commands, close }: { commands: EngineCommands; close(): void }) {
  const t = useT();
  const { transport, send } = commands;
  const project = useProjectStore((s) => s.project);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [openSub, setOpenSub] = useState<string | null>(null);
  const subContext = useMemo(() => ({ open: openSub, setOpen: setOpenSub }), [openSub]);
  const ref = useRef<HTMLDivElement>(null);
  const confirmRef = useRef<HTMLDivElement>(null);
  const missing = useMissingCount();
  const entries = useContributions("project.menu");
  // Opens like the kit menus: the first item focused, then ↑/↓/Home/End.
  useFocusFirstItem(ref);
  // The delete confirmation replaces its item: keep the keyboard on it (its cancel).
  useEffect(() => {
    if (confirmDelete)
      menuItems(ref.current)
        .find((el) => confirmRef.current?.contains(el))
        ?.focus();
  }, [confirmDelete]);
  if (!transport) return null;

  // #577: the same actions and labels as the command palette (`projectCommands.ts`).
  const cmds = byId<MenuCommand>([
    ...appProjectCommands(transport),
    ...(project ? openProjectCommands(transport, { id: project.id, name: project.settings.name }) : []),
    ...docsCommands(),
    // Extensions' palette commands the menu may name (`project.menu`).
    ...contributions("palette.commands").flatMap((f) => f({ transport, project, devices: [] })),
  ]);
  /** The extensions' entries of `section`. */
  const extra = (section: ProjectMenuEntry["section"]) =>
    entries
      .filter((e) => e.section === section && (e.when?.() ?? true))
      .map((e, i) =>
        "command" in e.item ? (
          item(e.item.command)
        ) : (
          <SubMenu key={`sub-${section}-${i}`} label={e.item.submenu} tour={e.item.tour}>
            {e.item.commands.map((id) => item(id, { short: true }))}
          </SubMenu>
        ),
      );
  /** The menu item of command `id` (the menu closes, then it runs). */
  const item = (id: string, opts: { short?: boolean; danger?: boolean; onClick?(): void } = {}) => {
    const c = cmds[id];
    if (!c) return null;
    return (
      <Item
        key={id}
        title={c.title}
        danger={opts.danger}
        onClick={
          opts.onClick ??
          (() => {
            close();
            c.run();
          })
        }
      >
        {opts.short && c.short ? c.short : c.label}
      </Item>
    );
  };

  return (
    <OpenSub.Provider value={subContext}>
      <div
        ref={ref}
        className="eth-menu eth-project-menu"
        role="menu"
        data-tour="project-menu-panel"
        aria-label={t("project.menu.label")}
        onKeyDown={(e) => menuKeyDown(e, ref.current)}
      >
        {item("project:new")}
        {item("project:open")}
        {extra("start")}
        {project && (
          <>
            <Separator />
            {item("project:save")}
            {item("project:save-as")}
            {item("project:duplicate")}
            {item("project:rename")}
            {item("template:save-project")}
            <Separator />
            <div className="eth-project-menu__scale">
              <ProjectScale send={send} />
            </div>
            <Separator />
            {item("project:import")}
            {extra("import")}
            {item("project:export")}
            {extra("export")}
            {item("project:export-audio")}
            <Separator />
            {item("project:versions")}
            {item("project:snapshot")}
            {/* import-resolver: everything missing (devices, plugins, samples), only while something is. */}
            {missing > 0 && (
              <Item
                title={t("project.menu.missing_hint")}
                onClick={() => {
                  close();
                  void openResolver(undefined, transport);
                }}
              >
                {resolveMissingLabel()}
              </Item>
            )}
            <Separator />
            {confirmDelete ? (
              <div ref={confirmRef} className="eth-project-menu__confirm" role="group" aria-label={t("project.menu.confirm_delete")}>
                <span className="eth-project-menu__confirm-text">
                  <Trans k="project.menu.delete_confirm" values={{ name: <span className="eth-user-text">{project.settings.name}</span> }} />
                </span>
                <span className="eth-project-menu__confirm-actions">
                  <button type="button" role="menuitem" className="eth-menu__item" onClick={() => setConfirmDelete(false)}>
                    {t("project.actions.cancel")}
                  </button>
                  <button
                    type="button"
                    role="menuitem"
                    className="eth-menu__item eth-menu__item--danger"
                    onClick={() => {
                      close();
                      deleteOpenProject(transport, project.id);
                    }}
                  >
                    {t("project.actions.delete")}
                  </button>
                </span>
              </div>
            ) : (
              // The palette's entry asks in a dialog; here the menu asks in place.
              item("project:delete", { danger: true, onClick: () => setConfirmDelete(true) })
            )}
          </>
        )}
        <Separator />
        {/* docs (#665): the user documentation (in the app on desktop, /docs in a new tab on the web). */}
        {item("app:docs")}
        {/* #518: the example beats, on the welcome screen. */}
        {item("app:examples")}
        {extra("app")}
      </div>
    </OpenSub.Provider>
  );
}

/** What the menu needs of a command (`ProjectCommand`, the palette's docs entries). */
interface MenuCommand {
  id: string;
  label: string;
  short?: string;
  title?: string;
  run(): void;
}
