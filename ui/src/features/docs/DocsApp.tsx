// The user docs (#665): a left nav of sections and pages (a drawer on phones), the page with
// its on-page contents, prev / next, and search. The same view runs standalone at `/docs`
// (the URL router) and over the desktop app (a memory router, `DocsRoot`).
import "./docs.css";
import clsx from "clsx";
import { ArrowLeft, ArrowRight, BookOpen, ChevronRight, Menu, Search, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from "react";
import { pageIsTranslated, useDocs } from "./content";
import { Blocks, DocsNavContext, Inlines, PlatformBadge } from "./DocsMarkdown";
import { neighbours, resolveView, type Docs, type DocPage, type DocSection } from "./model";
import type { DocsRouter } from "./router";
import { buildIndex, queryTerms, search, type SearchResult } from "./search";
import { setKeyPlatform, useKeyPlatform } from "./shortcuts";
import { getLocale, useT } from "@/i18n";
import { setTabTitle, tabTitle } from "@/features/document-title/title";

export interface DocsAppProps {
  router: DocsRouter;
  /** Over the app: a close button (and Escape) instead of the "open ethereal" link. */
  onClose?: () => void;
  /** The docs (tests pass fixtures). */
  docs?: Docs;
}

export function DocsApp({ router: base, onClose, docs: given }: DocsAppProps) {
  const t = useT();
  const bundled = useDocs();
  const docs = given ?? bundled;
  const [drawer, setDrawer] = useState(false);
  // Going somewhere closes the drawer (phones).
  const router = useMemo<DocsRouter>(
    () => ({
      loc: base.loc,
      href: base.href,
      go: (path, anchor) => {
        setDrawer(false);
        base.go(path, anchor);
      },
    }),
    [base],
  );
  const view = resolveView(docs, router.loc.path);
  const scroller = useRef<HTMLDivElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const current = view.kind === "page" ? view.page : null;

  // The page title (standalone), the scroll position (top, or the anchor) on navigation.
  useEffect(() => {
    if (!onClose) setTabTitle(tabTitle(current ? t("docs.page_tab_title", { page: current.title }) : t("docs.tab_title")));
  }, [current, onClose, t]);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const target = router.loc.anchor ? el.querySelector<HTMLElement>(`#${CSS.escape(router.loc.anchor)}`) : null;
    if (target?.scrollIntoView) target.scrollIntoView({ block: "start" });
    else el.scrollTop = 0;
  }, [router.loc.path, router.loc.anchor]);

  // "/" (or Mod+K) searches; Escape closes the drawer, then the docs (over the app).
  const onKey = (e: KeyboardEvent | ReactKeyboardEvent) => {
    const typing = e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;
    if ((e.key === "/" && !typing) || ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k")) {
      e.preventDefault();
      searchRef.current?.focus();
      searchRef.current?.select();
    } else if (e.key === "Escape" && !typing) {
      if (drawer) setDrawer(false);
      else onClose?.();
    }
  };
  const keyHandler = useRef(onKey);
  useEffect(() => {
    keyHandler.current = onKey;
  });
  // Standalone, the page is the docs: keys anywhere count (the focus is often on the body).
  useEffect(() => {
    if (onClose) return;
    const listener = (e: KeyboardEvent) => keyHandler.current(e);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, [onClose]);
  // Over the app: the keys go to the docs, and the editor's shortcuts stay out (Space still
  // plays: the transport listens before anyone, by design).
  const onKeyDown = onClose
    ? (e: ReactKeyboardEvent) => {
        onKey(e);
        e.stopPropagation();
      }
    : undefined;

  return (
    <DocsNavContext.Provider value={router}>
      <div
        className={clsx("eth-docs", onClose && "eth-docs--overlay", drawer && "eth-docs--drawer-open")}
        data-testid="docs"
        onKeyDown={onKeyDown}
        {...(onClose ? { role: "dialog", "aria-modal": true, "aria-label": t("docs.dialog") } : {})}
      >
        <header className="eth-docs__top">
          <button type="button" className="eth-docs__menu" aria-label={t("docs.menu")} aria-expanded={drawer} onClick={() => setDrawer((d) => !d)}>
            <Menu aria-hidden />
          </button>
          <a
            className="eth-docs__brand"
            href={router.href("")}
            onClick={(e) => {
              e.preventDefault();
              router.go("");
            }}
          >
            <BookOpen aria-hidden />
            <span>{t("docs.brand")}</span>
            <span className="eth-docs__brand-docs">{t("docs.brand_docs")}</span>
          </a>
          <DocsSearch docs={docs} router={router} inputRef={searchRef} />
          <KeysToggle />
          {onClose ? (
            <button type="button" className="eth-docs__close" aria-label={t("docs.close")} onClick={onClose} data-testid="docs-close">
              <X aria-hidden />
            </button>
          ) : (
            <a className="eth-docs__app" href="/">
              {t("docs.open_app")}
            </a>
          )}
        </header>
        <div className="eth-docs__body">
          <div className="eth-docs__backdrop" aria-hidden onClick={() => setDrawer(false)} />
          <nav className="eth-docs__nav" aria-label={t("docs.sections")} data-testid="docs-nav">
            <SectionNav docs={docs} current={current} router={router} />
          </nav>
          <div className="eth-docs__scroll" ref={scroller} tabIndex={-1}>
            {view.kind === "home" && <Home docs={docs} router={router} />}
            {view.kind === "page" && <PageView docs={docs} page={view.page} router={router} scroller={scroller} />}
            {view.kind === "missing" && <Missing docs={docs} router={router} />}
          </div>
        </div>
      </div>
    </DocsNavContext.Provider>
  );
}

// ---- Nav -------------------------------------------------------------------------------

function SectionNav({ docs, current, router }: { docs: Docs; current: DocPage | null; router: DocsRouter }) {
  // Sections start open; the reader can fold them. The current page's section is always open.
  const [closed, setClosed] = useState<ReadonlySet<string>>(() => new Set());
  return (
    <ul className="eth-docs-nav">
      {docs.sections.map((s) => {
        const open = !closed.has(s.slug) || current?.section === s.slug;
        return (
          <li key={s.slug} className="eth-docs-nav__section">
            <button
              type="button"
              className="eth-docs-nav__heading"
              aria-expanded={open}
              onClick={() =>
                setClosed((c) => {
                  const next = new Set(c);
                  if (open) next.add(s.slug);
                  else next.delete(s.slug);
                  return next;
                })
              }
            >
              <ChevronRight aria-hidden className="eth-docs-nav__chevron" />
              {s.title}
            </button>
            {open && (
              <ul className="eth-docs-nav__pages">
                {s.pages.map((p) => (
                  <li key={p.path}>
                    <PageLink router={router} page={p} className="eth-docs-nav__page" current={current?.path === p.path}>
                      {p.title}
                    </PageLink>
                  </li>
                ))}
              </ul>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function PageLink({
  router,
  page,
  anchor = "",
  className,
  current,
  children,
}: {
  router: DocsRouter;
  page: { path: string };
  anchor?: string;
  className?: string;
  current?: boolean;
  children: ReactNode;
}) {
  return (
    <a
      className={className}
      href={router.href(page.path, anchor)}
      aria-current={current ? "page" : undefined}
      onClick={(e) => {
        if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
        e.preventDefault();
        router.go(page.path, anchor);
      }}
    >
      {children}
    </a>
  );
}

// ---- Keys toggle -----------------------------------------------------------------------

function KeysToggle() {
  const t = useT();
  const keys = useKeyPlatform((s) => s.keys);
  return (
    <div className="eth-docs__keys" role="radiogroup" aria-label={t("docs.keys_for")}>
      {(["mac", "pc"] as const).map((k) => (
        <button key={k} type="button" role="radio" aria-checked={keys === k} className="eth-docs__keys-option" onClick={() => setKeyPlatform(k)}>
          {k === "mac" ? t("docs.keys_mac") : t("docs.keys_pc")}
        </button>
      ))}
    </div>
  );
}

// ---- Search ----------------------------------------------------------------------------

function DocsSearch({ docs, router, inputRef }: { docs: Docs; router: DocsRouter; inputRef: React.RefObject<HTMLInputElement | null> }) {
  const t = useT();
  const index = useMemo(() => buildIndex(docs), [docs]);
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  // The results close a moment after the input loses focus (a click on one lands first); typing
  // or focusing again within that moment keeps them open.
  const closing = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const keepOpen = () => {
    clearTimeout(closing.current);
    setOpen(true);
  };
  useEffect(() => () => clearTimeout(closing.current), []);
  const results = useMemo(() => search(index, query), [index, query]);
  const terms = useMemo(() => queryTerms(query), [query]);

  const pick = useCallback(
    (r: SearchResult) => {
      setOpen(false);
      setQuery("");
      inputRef.current?.blur();
      router.go(r.entry.path, r.entry.anchor);
    },
    [router, inputRef],
  );

  return (
    <div className="eth-docs-search" data-testid="docs-search">
      <Search aria-hidden className="eth-docs-search__icon" />
      <input
        ref={inputRef}
        type="search"
        className="eth-docs-search__input"
        placeholder={t("docs.search")}
        aria-label={t("docs.search")}
        aria-expanded={open && query !== ""}
        aria-controls="eth-docs-search-results"
        role="combobox"
        value={query}
        onChange={(e) => {
          setQuery(e.target.value);
          setActive(0);
          keepOpen();
        }}
        onFocus={keepOpen}
        onBlur={() => {
          closing.current = setTimeout(() => setOpen(false), 150);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") {
            e.preventDefault();
            setActive((a) => Math.min(a + 1, results.length - 1));
          } else if (e.key === "ArrowUp") {
            e.preventDefault();
            setActive((a) => Math.max(a - 1, 0));
          } else if (e.key === "Enter" && results[active]) {
            e.preventDefault();
            pick(results[active]);
          } else if (e.key === "Escape") {
            e.stopPropagation();
            if (query) setQuery("");
            else inputRef.current?.blur();
          }
        }}
      />
      {open && query.trim() !== "" && (
        <ul className="eth-docs-search__results" id="eth-docs-search-results" role="listbox" aria-label={t("docs.search_results")}>
          {results.length === 0 && <li className="eth-docs-search__empty">{t("docs.nothing_found")}</li>}
          {results.map((r, i) => (
            <li
              key={`${r.entry.path}#${r.entry.anchor}`}
              role="option"
              aria-selected={i === active}
              className={clsx("eth-docs-search__result", i === active && "eth-docs-search__result--active")}
              onMouseDown={(e) => {
                e.preventDefault();
                pick(r);
              }}
              onMouseEnter={() => setActive(i)}
            >
              <span className="eth-docs-search__where">
                {r.entry.sectionTitle} › {r.entry.pageTitle}
                {r.entry.heading && ` › ${r.entry.heading}`}
              </span>
              <span className="eth-docs-search__snippet">
                <Highlight text={r.snippet} terms={terms} />
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Highlight({ text, terms }: { text: string; terms: string[] }) {
  if (terms.length === 0) return <>{text}</>;
  const re = new RegExp(`(${terms.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "gi");
  return (
    <>
      {text.split(re).map((part, i) =>
        i % 2 === 1 ? (
          <mark key={i} className="eth-docs-search__mark">
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  );
}

// ---- Pages -----------------------------------------------------------------------------

function Home({ docs, router }: { docs: Docs; router: DocsRouter }) {
  const t = useT();
  const first = docs.pages[0];
  return (
    <main className="eth-docs__main eth-docs-home" data-testid="docs-home">
      <article className="eth-docs__article">
        <h1 className="eth-docs__title">{t("docs.home_title")}</h1>
        <p className="eth-docs-lead">{t("docs.home_lead")}</p>
        {first && (
          <PageLink router={router} page={first} className="eth-docs-home__start">
            <span className="eth-docs-home__start-label">{t("docs.start_here")}</span>
            <span className="eth-docs-home__start-title">{first.title}</span>
            <ArrowRight aria-hidden />
          </PageLink>
        )}
        <div className="eth-docs-home__sections">
          {docs.sections.map((s) => (
            <SectionCard key={s.slug} section={s} router={router} />
          ))}
        </div>
      </article>
    </main>
  );
}

function SectionCard({ section, router }: { section: DocSection; router: DocsRouter }) {
  return (
    <section className="eth-docs-home__section">
      <h2 className="eth-docs-home__section-title">{section.title}</h2>
      {section.summary && <p className="eth-docs-home__section-summary">{section.summary}</p>}
      <ul className="eth-docs-home__pages">
        {section.pages.map((p) => (
          <li key={p.path}>
            <PageLink router={router} page={p}>
              {p.title}
            </PageLink>
          </li>
        ))}
      </ul>
    </section>
  );
}

function PageView({ docs, page, router, scroller }: { docs: Docs; page: DocPage; router: DocsRouter; scroller: React.RefObject<HTMLDivElement | null> }) {
  const t = useT();
  const section = docs.sections.find((s) => s.slug === page.section);
  const { prev, next } = neighbours(docs, page.path);
  const toc = page.headings.filter((h) => h.level <= 3);
  const activeId = useActiveHeading(scroller, page);
  const translated = pageIsTranslated(page);
  return (
    <main className="eth-docs__main" data-testid="docs-page" data-path={page.path}>
      {/* An untranslated page reads in English: say so, and mark its language (screen readers). */}
      <article className="eth-docs__article eth-docs-prose" lang={translated ? undefined : "en"}>
        {!translated && (
          <p className="eth-docs__untranslated" lang={getLocale()} data-testid="docs-untranslated">
            {t("docs.untranslated")}
          </p>
        )}
        <p className="eth-docs__crumb">{section?.title}</p>
        <h1 className="eth-docs__title">
          {page.title}
          {page.platforms.map((p) => (
            <PlatformBadge key={p} platform={p} />
          ))}
        </h1>
        <p className="eth-docs-lead">{page.summary}</p>
        {toc.length > 2 && (
          <details className="eth-docs__toc-inline">
            <summary>{t("docs.on_this_page")}</summary>
            <Toc toc={toc} router={router} page={page} activeId={activeId} />
          </details>
        )}
        <Blocks blocks={page.blocks} page={page} />
        <nav className="eth-docs-pager" aria-label={t("docs.pager")}>
          {prev ? (
            <PageLink router={router} page={prev} className="eth-docs-pager__link eth-docs-pager__link--prev">
              <span className="eth-docs-pager__label">
                <ArrowLeft aria-hidden /> {t("docs.previous")}
              </span>
              <span className="eth-docs-pager__title">{prev.title}</span>
            </PageLink>
          ) : (
            <span />
          )}
          {next && (
            <PageLink router={router} page={next} className="eth-docs-pager__link eth-docs-pager__link--next">
              <span className="eth-docs-pager__label">
                {t("docs.next")} <ArrowRight aria-hidden />
              </span>
              <span className="eth-docs-pager__title">{next.title}</span>
            </PageLink>
          )}
        </nav>
      </article>
      {toc.length > 0 && (
        <aside className="eth-docs__toc" aria-label={t("docs.on_this_page")}>
          <p className="eth-docs__toc-title">{t("docs.on_this_page")}</p>
          <Toc toc={toc} router={router} page={page} activeId={activeId} />
        </aside>
      )}
    </main>
  );
}

function Toc({ toc, router, page, activeId }: { toc: DocPage["headings"]; router: DocsRouter; page: DocPage; activeId: string | null }) {
  return (
    <ul className="eth-docs-toc">
      {toc.map((h) => (
        <li key={h.id} className={clsx("eth-docs-toc__item", h.level === 3 && "eth-docs-toc__item--sub")}>
          <PageLink router={router} page={page} anchor={h.id} current={activeId === h.id} className="eth-docs-toc__link">
            <Inlines nodes={h.c.filter((n) => n.t !== "badge")} />
          </PageLink>
        </li>
      ))}
    </ul>
  );
}

/** The heading the reader is in (the last one above the top of the scroller). */
function useActiveHeading(scroller: React.RefObject<HTMLDivElement | null>, page: DocPage): string | null {
  const [active, setActive] = useState<string | null>(null);
  useEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const update = () => {
      const top = el.getBoundingClientRect().top;
      let id: string | null = null;
      for (const h of el.querySelectorAll<HTMLElement>(".eth-docs__article .eth-docs-heading[id]")) {
        if (h.getBoundingClientRect().top - top > el.clientHeight / 4) break;
        id = h.id;
      }
      setActive(id);
    };
    update();
    el.addEventListener("scroll", update, { passive: true });
    return () => el.removeEventListener("scroll", update);
  }, [scroller, page]);
  return active;
}

function Missing({ docs, router }: { docs: Docs; router: DocsRouter }) {
  const t = useT();
  return (
    <main className="eth-docs__main" data-testid="docs-missing">
      <article className="eth-docs__article eth-docs-prose">
        <h1 className="eth-docs__title">{t("docs.not_found")}</h1>
        <p className="eth-docs-lead">{t("docs.not_found_lead")}</p>
        <ul>
          {docs.sections.map((s) => (
            <li key={s.slug}>
              <PageLink router={router} page={s.pages[0]!}>
                {s.title}
              </PageLink>
            </li>
          ))}
        </ul>
      </article>
    </main>
  );
}
