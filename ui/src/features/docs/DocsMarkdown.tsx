// Renders parsed docs Markdown (markdown.ts) as React elements: no HTML strings. Internal
// links go through the docs router (`DocsNav`); shortcuts come from the keymap (shortcuts.ts).
import clsx from "clsx";
import { AlertTriangle, Info, Lightbulb } from "lucide-react";
import { createContext, Fragment, useContext, type MouseEvent, type ReactNode } from "react";
import { actionLabel, groupActions, groupLabel, MOUSE_GESTURES } from "@/features/keymap/groups";
import { imageUrl } from "./content";
import type { Block, CalloutKind, Inline, Platform } from "./markdown";
import type { DocPage } from "./model";
import { docsLinkTarget } from "./route";
import { chordLabel, docAction, DOC_ACTIONS, keysLabel, scopeLabel, useKeyPlatform } from "./shortcuts";
import { useT, type MessageKey } from "@/i18n";

/** The docs router, as the content's links need it. */
export interface DocsNav {
  href(path: string, anchor?: string): string;
  go(path: string, anchor?: string): void;
}

export const DocsNavContext = createContext<DocsNav>({
  href: (path, anchor) => `/docs/${path}${anchor ? `#${anchor}` : ""}`,
  go: () => undefined,
});

const PLATFORM_LABEL: Record<Platform, MessageKey> = { desktop: "docs.platform.desktop", web: "docs.platform.web", mobile: "docs.platform.mobile" };
const PLATFORM_ONLY: Record<Platform, MessageKey> = {
  desktop: "docs.platform.desktop_only",
  web: "docs.platform.web_only",
  mobile: "docs.platform.mobile_only",
};

export function PlatformBadge({ platform }: { platform: Platform }) {
  const t = useT();
  return (
    <span className={clsx("eth-docs-badge", `eth-docs-badge--${platform}`)} title={t(PLATFORM_ONLY[platform])}>
      {t(PLATFORM_LABEL[platform])}
    </span>
  );
}

function Keys({ chords }: { chords: ReadonlyArray<string> }) {
  const t = useT();
  const keys = useKeyPlatform((s) => s.keys);
  return (
    <span className="eth-docs-keys">
      {chords.map((c, i) => (
        <Fragment key={c}>
          {i > 0 && <span className="eth-docs-keys__or"> {t("docs.keys_or")} </span>}
          <kbd className="eth-docs-kbd">{chordLabel(c, keys)}</kbd>
        </Fragment>
      ))}
    </span>
  );
}

function KeyChord({ chord }: { chord: string }) {
  const keys = useKeyPlatform((s) => s.keys);
  return <kbd className="eth-docs-kbd">{keysLabel(chord, keys)}</kbd>;
}

function ActionKeys({ id }: { id: string }) {
  const t = useT();
  const action = docAction(id);
  if (!action) return <code>{id}</code>;
  if (action.chords.length > 0) return <Keys chords={action.chords} />;
  if (action.ableton?.length)
    return (
      <span className="eth-docs-keys">
        <Keys chords={action.ableton} /> <span className="eth-docs-keys--none">({t("docs.ableton_preset")})</span>
      </span>
    );
  return <span className="eth-docs-keys eth-docs-keys--none">{t("docs.no_shortcut")}</span>;
}

function Link({ href, children }: { href: string; children: ReactNode }) {
  const nav = useContext(DocsNavContext);
  const target = docsLinkTarget(href);
  if (!target)
    return (
      <a href={href} target={/^https?:/.test(href) ? "_blank" : undefined} rel="noreferrer">
        {children}
      </a>
    );
  const onClick = (e: MouseEvent) => {
    if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
    e.preventDefault();
    nav.go(target.path, target.anchor);
  };
  return (
    <a href={nav.href(target.path, target.anchor)} onClick={onClick}>
      {children}
    </a>
  );
}

export function Inlines({ nodes }: { nodes: ReadonlyArray<Inline> }) {
  return (
    <>
      {nodes.map((n, i) => {
        switch (n.t) {
          case "text":
            return <Fragment key={i}>{n.v}</Fragment>;
          case "strong":
            return (
              <strong key={i}>
                <Inlines nodes={n.c} />
              </strong>
            );
          case "em":
            return (
              <em key={i}>
                <Inlines nodes={n.c} />
              </em>
            );
          case "code":
            return <code key={i}>{n.v}</code>;
          case "link":
            return (
              <Link key={i} href={n.href}>
                <Inlines nodes={n.c} />
              </Link>
            );
          case "action":
            return <ActionKeys key={i} id={n.id} />;
          case "key":
            return <KeyChord key={i} chord={n.chord} />;
          case "badge":
            return <PlatformBadge key={i} platform={n.platform} />;
        }
      })}
    </>
  );
}

const CALLOUT: Record<CalloutKind, { label: MessageKey; icon: ReactNode }> = {
  tip: { label: "docs.callout.tip", icon: <Lightbulb aria-hidden /> },
  note: { label: "docs.callout.note", icon: <Info aria-hidden /> },
  warning: { label: "docs.callout.warning", icon: <AlertTriangle aria-hidden /> },
};

/** A list item whose content is one paragraph renders it inline (a tight list). */
function ListItem({ blocks, page }: { blocks: Block[]; page: DocPage }) {
  const [first, ...rest] = blocks;
  if (first?.t === "p")
    return (
      <li>
        <Inlines nodes={first.c} />
        {rest.length > 0 && <Blocks blocks={rest} page={page} />}
      </li>
    );
  return (
    <li>
      <Blocks blocks={blocks} page={page} />
    </li>
  );
}

export function Blocks({ blocks, page }: { blocks: ReadonlyArray<Block>; page: DocPage }) {
  const t = useT();
  return (
    <>
      {blocks.map((b, i) => {
        switch (b.t) {
          case "heading": {
            const H = `h${b.level}` as "h2" | "h3" | "h4";
            return (
              <H key={i} id={b.id} className="eth-docs-heading">
                <Inlines nodes={b.c} />
                <a className="eth-docs-heading__anchor" href={`#${b.id}`} aria-label={t("docs.link_section")}>
                  #
                </a>
              </H>
            );
          }
          case "p":
            return (
              <p key={i}>
                <Inlines nodes={b.c} />
              </p>
            );
          case "list": {
            const L = b.ordered ? "ol" : "ul";
            return (
              <L key={i}>
                {b.items.map((item, j) => (
                  <ListItem key={j} blocks={item} page={page} />
                ))}
              </L>
            );
          }
          case "callout":
            return (
              <aside key={i} className={clsx("eth-docs-callout", `eth-docs-callout--${b.kind}`)} role="note" aria-label={t(CALLOUT[b.kind].label)}>
                <span className="eth-docs-callout__icon">{CALLOUT[b.kind].icon}</span>
                <div className="eth-docs-callout__body">
                  <Blocks blocks={b.blocks} page={page} />
                </div>
              </aside>
            );
          case "table":
            return (
              <div key={i} className="eth-docs-table">
                <table>
                  <thead>
                    <tr>
                      {b.head.map((c, j) => (
                        <th key={j}>
                          <Inlines nodes={c} />
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {b.rows.map((row, r) => (
                      <tr key={r}>
                        {row.map((c, j) => (
                          <td key={j}>
                            <Inlines nodes={c} />
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            );
          case "code":
            return (
              <pre key={i} className="eth-docs-code">
                <code>{b.v}</code>
              </pre>
            );
          case "image": {
            const url = imageUrl(page, b.src);
            if (!url) return null;
            return (
              <figure key={i} className="eth-docs-figure">
                <img src={url} alt={b.alt} loading="lazy" />
                {b.caption && <figcaption>{b.caption}</figcaption>}
              </figure>
            );
          }
          case "shortcuts":
            return <ShortcutTables key={i} />;
        }
      })}
    </>
  );
}

/** The generated shortcuts reference: every keymap action with a default chord, by group. */
export function ShortcutTables() {
  const t = useT();
  const keys = useKeyPlatform((s) => s.keys);
  const bound = DOC_ACTIONS.filter((a) => a.chords.length > 0 || (a.ableton?.length ?? 0) > 0);
  return (
    <div className="eth-docs-shortcuts" data-testid="docs-shortcuts">
      {groupActions(bound).map(({ group, actions }) => (
        <section key={group} className="eth-docs-shortcuts__group">
          <h3 className="eth-docs-heading" id={`keys-${group.toLowerCase().replace(/\s+/g, "-")}`}>
            {groupLabel(group)}
          </h3>
          <div className="eth-docs-table">
            <table>
              <thead>
                <tr>
                  <th>{t("docs.table.action")}</th>
                  <th>{t("docs.table.keys")}</th>
                  <th>{t("docs.table.where")}</th>
                </tr>
              </thead>
              <tbody>
                {actions.map((a) => (
                  <tr key={a.id} data-action={a.id}>
                    <td>{actionLabel(a)}</td>
                    <td>
                      {a.chords.length > 0 ? <Keys chords={a.chords} /> : <span className="eth-docs-keys--none">{t("docs.table.none")}</span>}
                      {(a.ableton?.length ?? 0) > 0 && (
                        <span className="eth-docs-shortcuts__ableton">
                          {" "}
                          {t("docs.table.ableton", { keys: a.ableton!.map((c) => chordLabel(c, keys)).join(", ") })}
                        </span>
                      )}
                    </td>
                    <td>{scopeLabel(a) || t("docs.table.everywhere")}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </section>
      ))}
      <section className="eth-docs-shortcuts__group">
        <h3 className="eth-docs-heading" id="keys-mouse">
          {t("docs.table.mouse")}
        </h3>
        <div className="eth-docs-table">
          <table>
            <tbody>
              {MOUSE_GESTURES.map((g) => (
                <tr key={g.label}>
                  <td>{t(g.label)}</td>
                  <td>
                    <kbd className="eth-docs-kbd">{keysLabel(g.keys, keys)}</kbd> {t("docs.table.drag")}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
