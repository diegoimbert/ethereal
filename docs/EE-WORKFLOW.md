# The `ee/` submodule workflow

This repository is the complete open-source DAW. The official builds add a proprietary part
("EE") that lives in a separate, private repository, checked out here as the **`ee/`
submodule**. `.gitmodules` marks it `update = none`, so a public clone (even
`git clone --recursive`) leaves `ee/` empty, and everything here builds and tests without it.
Contributors without access to the private repository can ignore this page.

## Building the full app

From a normal OSS checkout (your uncommitted changes included):

| Command | What it builds |
|---|---|
| `just full-desktop [args]` | the desktop app, dev build, every feature |
| `just full-build [args]` | the desktop release bundle (`tauri build --features full`) |
| `just full-server [args]` | the headless engine server, every feature |
| `just full-web [args]` | the web app's dev server |

They initialise `ee/` and its submodules when missing (that needs read access to the private
repository). While the private Rust code still overlays OSS files (`ee/overlay.mjs`), they
build in a cached copy, `target/full-tree`: this checkout with the overlay applied, refreshed
on every run by rewriting only the files whose content changed, so its cargo target stays
warm. `node scripts/full.mjs prepare` only refreshes it. Once `ee/` builds on its own, the
same commands build there in place. Extra arguments go to the underlying command (e.g.
`just full-desktop --release`).

Rules:

- **Dependency direction**: `ee/` may depend on this repo, never the reverse. Nothing outside
  `ee/` references paths inside it (CI job `ee-boundary`).
- **One checkout**: EE work happens in `ee/` of an OSS checkout or worktree, never in a separate
  clone of the private repo.
- **Pairing by branch name**: OSS branch `x` pairs with EE branch `x`.

## Invariants

1. An OSS PR either leaves the `ee` gitlink untouched, or has a paired EE branch `x` (and EE PR)
   and points `ee` at the tip of EE `x`.
2. OSS `main` points at EE `main`. OSS `x` points at EE `x` while EE `x` exists.
3. When the EE PR `x` merges, OSS `x` is re-pointed at EE `main`, then merges; an EE-only change
   gets an automatic pointer-bump commit on OSS `main`.
4. No OSS commit points at an EE commit that is not pushed, and OSS `main` only ever points at
   EE `main` commits.

## Commands (`just feat ...`, `scripts/feat.mjs`)

| Command | What it does |
|---|---|
| `start x [--ee] [--worktree <dir>]` | Branch `x` off `origin/main` (or `origin/x`), here or in a new worktree. `--ee` (or an existing EE branch `x`) also initialises `ee/` and puts it on EE branch `x`. |
| `ee` | Initialise `ee/` and put it on the current branch's name (existing EE `x`, else new off EE `main`). |
| `push [--draft] [--title T] [--body B \| --body-file F]` | Push EE `x` (if it has commits), commit the pointer (`ee: point at x`), push OSS `x`, open or update both PRs with cross-links, post `ee/pointer` and start the full build (`ee/build`). With no OSS changes the OSS branch stays local: EE-only. If a branch moved on GitHub (bot pointer commits, a ship's merge of main), that is merged in first; nothing is force-pushed except bot pointer-only commits. |
| `status` | Branches, pointers, PRs, checks, `ee/*` statuses, the ship lock, and what is out of line. |
| `update` | Merge `main` into OSS `x` and EE `main` into EE `x`; a gitlink conflict resolves to `ee/`'s HEAD. |
| `ship [x] [--admin] [--force] [--no-build] [--ignore-ee-checks] [--timeout <min>] [--takeover]` | Takes the global ship lock, brings OSS `x` (and EE `x`) up to date with `main`, waits for every check on both PRs including `ee/build`, merges the EE PR, points OSS `x` at EE `main`, merges the OSS PR (squash) and makes sure OSS `main` points at EE `main`. Repeats the update/wait round if anything moved meanwhile. `--force` ships despite failing checks, `--no-build` skips only the `ee/build` wait, `--ignore-ee-checks` reports failing EE PR checks without gating on them (an EE change merged ahead of its OSS side), `--admin` bypasses branch protection. |
| `sync [--dry-run] [--pr N] [--build]` | The reconcile the private-side bot runs (needs write access to both repos). |
| `check <pr>` | Dry-run the pointer guard for one OSS PR. |
| `hooks` | Install a pre-push hook that refuses OSS commits pointing at unpushed EE commits. |
| `tree <dir>` | A throwaway full-product tree (copy of this checkout + the EE overlay) to build locally. |

Typical paired feature:

```sh
just feat start my-thing --ee --worktree ../my-thing && cd ../my-thing
# edit both sides; commit OSS changes here and EE changes in ee/ (cd ee && git commit)
just feat push            # EE first, then the pointer + OSS; PRs opened and cross-linked
just feat status
just feat ship my-thing   # EE merges first, then OSS (squash), under the lock
```

An OSS-only change is the same without `--ee` (its PR never touches the gitlink); an EE-only
change is the same with only `ee/` commits (only an EE PR is opened).

Keep `ee/` on a branch while you edit it. A plain `git submodule update` checks it out
detached at the pointer; `just feat ee` puts it back on `x`.

## Automation

The private repository runs the bots and the full-product CI, so no private code or log ever
reaches this repository's Actions:

- **Sync** (on every push there, on `feat push`/`ship`, and hourly): moves OSS `main` to EE
  `main` (unless a ship holds the lock), keeps each open OSS PR's pointer on its EE branch tip
  (or EE `main` once that merged), merges `main` into a PR whose only conflict is the gitlink,
  and posts the **`ee/pointer`** status on OSS PRs. Its API commits touch nothing but `ee` and
  are fast-forward only, retried when the branch moved.
- **Full build**: each non-draft OSS PR head gets the **`ee/build`** status: the full product
  built from that PR (with EE `main` when the PR leaves `ee` alone). Fork PRs are not built
  automatically.
- **EE CI** builds each EE PR against OSS branch `x` (or OSS `main`).
- Releases and the dev deploy are built on the private side from OSS `main` + the `ee` commit
  it points at; GitHub Releases are still published here.

`feat push` and `feat ship` do the bot's part for their own PR themselves (pointer status,
starting the build, turning a finished build into `ee/build`), so the workflow keeps working
when the bot cannot write here.

## Concurrency notes

- Only `feat ship` merges paired PRs, one ship at a time: a lock ref in the private repo,
  renewed every few minutes while the ship waits, and considered dead after 30 minutes without
  renewal. The sync bot leaves the shipping branch and OSS `main` alone while the lock is held.
- Squash merges and `main` pointer bumps make the gitlink conflict on open paired PRs; that is
  expected. The bot, `feat update` and `feat ship` resolve it (to the EE branch tip, then EE
  `main`).
- After the EE PR merged, the OSS PR's head only changes the gitlink (to EE `main`, normally
  the very tree that was built), so the `ee/*` statuses carry over and only the required checks
  are waited for again.
- If a ship stops after the EE PR merged (e.g. the OSS merge failed), it keeps the lock; fix
  the cause and run `just feat ship x --takeover` to finish (OSS `x` is pointed at EE `main`).
