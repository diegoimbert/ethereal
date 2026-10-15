# Contributing to Ethereal

Thanks for your interest in Ethereal. The [README](README.md) is the project overview; this file covers how
to set up a checkout, what CI expects, and how to get a change merged. When the two disagree,
the README and the docs in [`docs/`](docs) win.

By participating you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- **Report a bug.** Use the [bug template](https://github.com/diegoimbert/ethereal/issues/new?template=bug.yml)
  in the issue tracker. Include what you did, what you expected, what happened instead, the
  platform (desktop, web, `ether-server`) and the version or commit. Never paste secrets or
  private sample paths.
- **Request a feature.** Use the [feature template](https://github.com/diegoimbert/ethereal/issues/new?template=feature.yml).
  Say what the change is, what problem it solves and how we'll know it works.
- **Write code.** Rust (engine, host, tools) or TypeScript/React (UI).
- **Write docs.** User-facing pages live in [`docs-site/content/`](docs-site/content) and are
  bundled into the app at `/docs`; the developer docs are the Markdown files in [`docs/`](docs).
  The page format and style rules are in [`docs-site/README.md`](docs-site/README.md).
- **Translate.** The UI ships in English and French; a language is one catalog file plus one
  line of code. See [`docs/LOCALIZATION.md`](docs/LOCALIZATION.md). It also covers translating
  the `/docs` pages.
- **Work on the design system.** Colors, shapes, typography, spacing and motion come from one
  file, `ui/src/theme/tokens.ts`. See [`docs/DESIGN-SYSTEM.md`](docs/DESIGN-SYSTEM.md).

## Before you start

- **Search first.** Look through open issues and pull requests; comment on an existing issue
  rather than opening a duplicate.
- **Open an issue before large work.** Anything beyond a small fix or a clearly-scoped bug is
  worth an issue first, so we can agree on the approach and on where the change belongs. The
  engine and the document model have real-time and format-compatibility constraints (see
  [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`docs/CONTRACTS.md`](docs/CONTRACTS.md));
  a short design note saves a lot of rework.
- **Keep a pull request focused.** One change per pull request. Unrelated cleanups make review
  harder and are better as their own pull request.
- **Do not report security problems in public.** See [Security](#security) below.
- **Sign the CLA.** The first time you open a pull request you'll be asked to (below).

## Contributor License Agreement

Ethereal is licensed under the **GNU Affero General Public License, version 3 or later**
(AGPL-3.0-or-later), and the Maintainer also offers it under other, including commercial,
terms (see [`LICENSING.md`](LICENSING.md)). To make that dual licensing possible, every
contribution must be covered by the [Contributor License Agreement](CLA.md) (**CLA**). You
keep the copyright to your contribution; the CLA only grants the permissions needed to
distribute Ethereal under the AGPL and, separately, under other terms.

To sign, open (or update) a pull request and reply on it with exactly:

> I have read the CLA Document and I hereby sign the CLA

A bot records your GitHub account and marks the pull request's **CLA** check as passed. You
only need to sign once. If the check does not update, comment `recheck`. The `CLA` check is
required before a pull request can merge. If you contribute on behalf of an employer or
another entity, read [CLA.md](CLA.md) §4 first.

## Development setup

The toolchain table (Rust, wasm-bindgen, Node, pnpm, just, Tauri, the C/C++ toolchain and
platform packages) is in the README under [Toolchain setup](README.md#toolchain-setup). In
short:

```sh
pnpm install
just check-all   # fmt, clippy, wasm32 check, tsc, eslint, licenses, generated-file freshness
just test-all    # cargo test + vitest
```

`cargo` picks up the pinned toolchain from `rust-toolchain.toml`, which installs `rustfmt`,
`clippy` and the `wasm32-unknown-unknown` target on the first run.

Two gates in `just check-all` need tools installed once per machine:

```sh
cargo install cargo-deny
cargo install cargo-bundle-licenses --version 4.2.0
```

### Everyday commands

Run the app without a full build, or against a real engine:

```sh
just dev-ui        # standalone UI against the in-memory MockTransport (no Rust needed)
just dev-web       # browser host (COOP/COEP headers for SharedArrayBuffer)
just dev-desktop   # Tauri desktop app, fast dev core
```

`just --list` shows every recipe, and the README's
[Everyday commands](README.md#everyday-commands-just---list) table explains them.

### Never hardcode ports or paths

Several checkouts (worktrees, parallel agents) run dev servers and tests at the same time
without clashing. Always go through the `just` recipes; each derives a per-instance id and
port from the checkout directory (`scripts/dev-env.mjs` is the single source of truth). Do not
hardcode a port or an app path. See the README's
[Running multiple dev instances](README.md#running-multiple-dev-instances) for the details.

## What CI runs

CI (`.github/workflows/ci.yml`) must pass before a pull request merges. The local equivalents:

| Gate | Local command |
|---|---|
| Rust formatting | `cargo fmt --all --check` (fix with `just fmt`) |
| Rust lints, every feature and none | `just check-features` |
| Rust tests | `just test-all` |
| wasm32 build of the engine crates | `just check-wasm` |
| TypeScript typecheck | `pnpm -r typecheck` |
| ESLint | `pnpm -r lint` |
| Prettier | `pnpm format:check` (fix with `just fmt` or `pnpm format`) |
| Generated TS types fresh | `just check-types-fresh` |
| Third-party notices fresh | `just check-notices-fresh` |
| Dependency licenses allowed | `just check-licenses` |
| Filename case clashes | `just check-case-clashes` |

`just check-all` runs every check except the tests; `just test-all` runs the tests. Run both
before you push, or at least the ones your change touches.

### Generated files

Some files are generated and must not be edited by hand. Change the source, run the recipe and
commit the result; CI fails if the output is stale.

- `ui/src/generated/` — TypeScript protocol types, generated from Rust with `just gen-types`.
  The protocol types are the Rust definitions in `crates/ether-protocol`; never hand-write them.
- `ui/src/theme/tokens.css` — generated from `tokens.ts` with `just gen-tokens`.
- `THIRD-PARTY-NOTICES.md` — regenerated with `just notices`.

### Cargo features

By default the Rust crates build a **dev core** that leaves out the plugin hosts, the sandbox
and Signalsmith complex warp; releases and the gates build every feature (`full`). When you
change feature-gated code, build it with its feature (or `full`) before pushing:

```sh
just dev-desktop-full                  # everything, like a release
cargo test -p ether-native --features full
```

The feature map is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md#cargo-features).

## Code guidelines

### Rust

- Formatting is `rustfmt` (`rustfmt.toml`: edition 2024, `max_width = 100`). `clippy` runs
  with `-D warnings`; the workspace lint configuration is in the root `Cargo.toml`.
- The audio path has hard real-time rules: no allocation or free, no locks shared with non-RT
  threads, no syscalls, I/O or logging, bounded runtime. See
  [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) ("Real-time rules").
- Keep the engine core (`ether-core`) free of threads, the filesystem, the clock and the
  device, so it keeps compiling to `wasm32`. The engine-side crates are checked for that in CI.
- Protocol and document-model *types* are contracts shared with the UI and the file format.
  Discuss changes to them in the issue before you write code; prefer additive changes.

### TypeScript and the UI

- Formatting is Prettier (`.prettierrc.json`: `printWidth` 160); linting is ESLint
  (`ui/eslint.config.js`). `tsc` must pass.
- Never hardcode a color or size in a feature: use the design tokens in
  `ui/src/theme/tokens.ts` ([`docs/DESIGN-SYSTEM.md`](docs/DESIGN-SYSTEM.md)).
- Never hand-write protocol types: import them from `ui/src/generated/`.
- The UI talks to the engine through the `EngineTransport` interface
  (`ui/src/transport/`), which has Tauri, WebAssembly and mock implementations. File handling
  is engine-side: the UI never reads or writes files and never sends paths.

### Documentation

- **User docs** (`docs-site/content/`): follow [`docs-site/README.md`](docs-site/README.md) for
  the front-matter, the Markdown subset and the writing rules (second person, plain words,
  lowercase headings and UI names, no em dashes). Describe what the app does now; verify it
  against the code. After editing, refresh the coverage report as the README explains. A
  French page exists for most pages; when you change an English page, change
  `docs-site/content-fr/` the same way (`docs/LOCALIZATION.md`).
- **Developer docs** (`docs/`): keep them consistent with the code they describe and link
  related docs rather than duplicating them.

## Tests

- Rust: `cargo test -p <crate>`, or `just test-all` for the workspace with every feature.
  Engine tests render offline and never need an audio device (`ETHER_AUDIO=null`, which CI
  sets).
- UI/web: `pnpm --filter @ethereal/ui test` (vitest), or `just test-all`.
- End to end: `just e2e-install` once per machine, then `just e2e-web` (Playwright against the
  real wasm engine) or `just e2e-native` (native smoke test on the null audio backend).
- Add a test with a fix or a new behaviour where it is testable. Snapshot tests use `insta`
  and `vitest`; property tests use `proptest`.

## Adding a dependency

Every dependency is distributed in the releases, so its license must be on the allow-list in
[`deny.toml`](deny.toml): permissive licenses, or ones compatible with both the AGPL and
relicensing. A copyleft dependency (GPL/AGPL) cannot come in, because it could not be
relicensed. Rust dependencies are declared centrally in the root
`Cargo.toml` `[workspace.dependencies]`; a crate opts in with `foo.workspace = true`. If you
add one, run `just check-licenses` and `just notices` and commit the regenerated
`THIRD-PARTY-NOTICES.md`. If a contribution includes third-party material, identify it and its
license in the pull request ([CLA.md](CLA.md) §4).

## Commit and pull request

- Branch off `main` and open a pull request against `main`.
- Write a clear commit message: a short summary line, then a blank line and a body explaining
  the why if it isn't obvious.
- Fill in the pull request template — **what and why**, and **how it was checked** (tests, e2e,
  screenshots for UI changes, anything still owed). Link the issue with `Fixes #<n>` so merging
  closes it.
- Make sure CI is green and the `CLA` check passes. A maintainer will review; keep the branch
  up to date and respond to review comments with new commits (avoid force-pushing during
  review unless asked).

## Security

Report vulnerabilities privately, never in a public issue, pull request or discussion. Use
GitHub's [private vulnerability reporting](https://github.com/diegoimbert/ethereal/security/advisories/new);
see [`SECURITY.md`](SECURITY.md) for what to include and what to expect.

## Community and support

Be kind and constructive: the [Code of Conduct](CODE_OF_CONDUCT.md) applies to every space in
the project. For help, open an issue or start a discussion rather than emailing a maintainer
directly. Bugs and feature requests belong in the
[issue tracker](https://github.com/diegoimbert/ethereal/issues).

## License

By contributing, you agree that your contributions are licensed under the
**AGPL-3.0-or-later** and are covered by the [CLA](CLA.md).
