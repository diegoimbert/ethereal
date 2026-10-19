# News: announcements to users

The maintainers can tell users something (a release, a survey, a heads-up) without shipping a
build. A few seconds after launch the app asks the news service for the items meant for it,
drops the ones the user already closed, and shows the newest one in a modal. Closing it (the ×
or close button, Escape, a click on the backdrop, or the page's own button) dismisses it for
good. If there are several, **next** dismisses the current one and shows the next; the ones not
seen yet wait for the next launch. **news…** (the desktop Help menu, the project menu, or the
command palette's Help group) lists the current items again, dismissed ones included.

Every build asks the same public service, a build made from source included: nothing private is
needed to show news. Users turn it off in **settings › privacy › check for news**: nothing is
requested then.

## What is sent and kept

- **The request** (`GET /v1/news?version=&platform=&os=`): the app version, `desktop` or `web`,
  and the OS family (`macos`, `windows`, `linux`, `ios`, `android`, `other`), so the service
  can pick the items meant for this app. No install id, no cookie (`credentials: "omit"`), no
  project data.
- **The service** keeps nothing about who asked: no IP address, no read counts, no request
  logs.
- **On the device**: the ids of the items the user closed and the toggle
  (`localStorage["eth-news"]`).
- Development builds and automated browsers (`navigator.webdriver`) ask nothing unless
  `localStorage["eth-news-url"]` is set.

## The app side

`ui/src/features/news`:

| File | What |
|---|---|
| `config.ts` | the endpoint: `DEFAULT_NEWS_URL` (`https://news.ethereal.ws`), or `localStorage["eth-news-url"]` (`""` turns it off, a URL turns it on even in dev); the request's query |
| `store.ts` | the fetch (8 s timeout; a failure is only logged with `console.info`, never shown at launch), dismissals, the toggle |
| `messages.ts` | the frame's sandbox and the two messages a page may post |
| `NewsDialog.tsx` | the viewer (one item at a time, in a sandboxed frame) and the **news…** list |
| `NewsRoot.tsx` | mounted by the app shell (desktop and mobile layouts): the startup check, and the toggle in settings › privacy |

To point a dev build at another instance, in the devtools:

```js
localStorage.setItem("eth-news-url", "http://127.0.0.1:8791");
location.reload();
```

(The dev server sends no CSP; under the production CSP only the default origin can be framed.)

## Security model

The item page is treated as untrusted:

- **Sandboxed frame, opaque origin.** The app loads `html_url` (on the news service's origin) in
  `<iframe sandbox="allow-scripts allow-popups allow-popups-to-escape-sandbox">`, without
  `allow-same-origin`, and with `referrerpolicy="no-referrer"`: the page runs in a unique
  origin. It can't read the app's DOM, `localStorage`, cookies or IndexedDB, can't navigate the
  app, and can't call the desktop shell. The service sends the same `sandbox` in the page's
  Content-Security-Policy, so even opened on its own the page has no origin.
- **No IPC for the frame (desktop).** Tauri answers IPC only from the app's own origin: every
  capability (`apps/desktop/src-tauri/capabilities/*.json`) lists the `main` window and no
  `remote` URLs, and `withGlobalTauri` is off. A test (`ui/src/features/news/news.test.tsx`,
  "isolation from the app") fails if that changes.
- **Framing allowed for the news service only.** `frame-src` in the app's CSP (desktop
  `apps/desktop/src-tauri/tauri.conf.json`, web `scripts/release/web/_headers`) is the news
  service's origin and nothing else (the same test checks both). A build that uses another
  service needs that CSP changed too.
- **Links leave the app.** On desktop the main window has a new-window handler
  (`create_main_window` in `apps/desktop/src-tauri/src/lib.rs`): a popup or `target="_blank"`
  link with an `http(s)` or `mailto` URL opens in the system browser.
- **Messages.** The page may post `{ type: "ethereal-news", action: "close" }` (close and
  dismiss) or `{ type: "ethereal-news", action: "resize", height }` (the frame's height,
  80-640 px). The app reads only these, only from that frame's window (`event.source`), with
  origin `null` or the service's. Anything else is ignored.

## The service API

The service is run by the maintainers. Its public, read-only part:

| Route | What |
|---|---|
| `GET /v1/news?version=&platform=&os=` | `{ items: [{ id, title, published_at, html_url }] }`, newest first; `Cache-Control: public, max-age=300`. CORS for the app's origins (the web app's domains, `tauri://localhost`, `http://tauri.localhost`, `http://localhost:*`, `http://127.0.0.1:*`); native requests send no `Origin`. |
| `GET /v1/news/<id>/content` | the item page (`text/html`, the sandboxing CSP, `Cross-Origin-Resource-Policy: cross-origin`) |
| `GET /v1/news/<id>/assets/<path>` | its files |

The app keeps only items whose `html_url` is `http(s)`, and orders them by `published_at`.
Another service that answers the same routes works with `localStorage["eth-news-url"]` (dev) or
a build with its URL in `DEFAULT_NEWS_URL` and in both CSPs.
