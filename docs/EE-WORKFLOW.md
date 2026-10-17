# The `ee/` submodule workflow

This repository is the complete open-source DAW. The official builds add a proprietary part
("EE") that lives in a separate, private repository, checked out here as the **`ee/`
submodule**. `.gitmodules` marks it `update = none`, so a public clone (even
`git clone --recursive`) leaves `ee/` empty, and everything here builds and tests without it.
Contributors without access to the private repository can ignore this page.

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
| `push [--draft] [--force] [--title T] [--body B]` | Push EE `x` (if it has commits), commit the pointer (`ee: point at x`), push OSS `x`, open or update both PRs with cross-links. With no OSS changes the OSS branch stays local: EE-only. A rejection caused only by the bot's pointer commits is resolved by replacing them. |
| `status` | Branches, pointers, PRs, check counts, and what is out of line. |
| `update` | Merge `main` into OSS `x` and EE `main` into EE `x`; a gitlink conflict resolves to `ee/`'s HEAD. |
| `ship [x] [--admin] [--force] [--timeout <min>] [--takeover]` | Wait for both PRs' checks, take the global ship lock, merge main into OSS `x`, merge the EE PR, point OSS `x` at EE `main`, wait for the required checks, merge the OSS PR, make sure OSS `main` points at EE `main`. |
| `sync [--dry-run]` | The reconcile the private-side bot runs (needs write access to both repos). |
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

Keep `ee/` on a branch while you edit it. A plain `git submodule update` checks it out
detached at the pointer; `just feat ee` puts it back on `x`.

## Automation

The private repository runs the bots and the full-product CI, so no private code or log ever
reaches this repository's Actions:

- **Sync** (on every push there, on `feat push`/`ship`, and hourly): moves OSS `main` to EE
  `main` (unless a ship holds the lock), keeps each open OSS PR's pointer on its EE branch tip
  (or EE `main` once that merged), and posts the **`ee/pointer`** status on OSS PRs. It only
  makes fast-forward API commits that touch nothing but `ee`, retried when the branch moved.
- **Full build**: each non-draft OSS PR head gets the **`ee/build`** status: the full product
  built from that PR (with EE `main` when the PR leaves `ee` alone). Fork PRs are not built
  automatically.
- **EE CI** builds each EE PR against OSS branch `x` (or OSS `main`).
- Releases and the dev deploy are built on the private side from OSS `main` + the `ee` commit
  it points at; GitHub Releases are still published here.

## Concurrency notes

- Only `feat ship` merges paired PRs, one ship at a time (a lock ref in the private repo; a
  lock older than an hour is considered stale). The sync bot leaves the shipping branch and
  OSS `main` alone while the lock is held.
- Squash merges and `main` pointer bumps make the gitlink conflict on open paired PRs; that is
  expected. `feat update` / `feat ship` resolve it (to the EE branch tip, then EE `main`).
- If a ship stops after the EE PR merged (e.g. the OSS checks failed), it keeps the lock; fix
  and re-run `just feat ship x` to finish (OSS `x` is pointed at EE `main`).
