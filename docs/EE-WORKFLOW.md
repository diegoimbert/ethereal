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
| `start x [--ee] [--worktree <dir>]` | Branch `x` off `origin/main` (or `origin/x`), here or in a new worktree (outside this checkout). `--ee` (or an existing EE branch `x`) also initialises `ee/` and puts it on EE branch `x`. |
| `ee` | Initialise `ee/` and put it on the current branch's name (existing EE `x`, else new off EE `main`). `start`, `ee`, `update` and `tree` also check out the submodules nested in `ee/` (libvpx). |
| `push [--draft] [--title T] [--body B \| --body-file F] [--ee-title T] [--ee-body B \| --ee-body-file F]` | Push EE `x` (if it has commits), commit the pointer (`ee: point at x`), push OSS `x`, open or update both PRs with cross-links (a given title/body is applied to open PRs too; `--ee-*` set the EE PR's, which default to the OSS ones), post `ee/pointer`. No full build (pull requests build nothing, see below). With no OSS changes the OSS branch stays local: EE-only. If a branch moved on GitHub (bot pointer commits, a ship's merge of main), that is merged in first; nothing is force-pushed except bot pointer-only commits. |
| `status` | Branches, pointers, PRs, checks, `ee/*` statuses, the ship lock, and what is out of line. |
| `update` | Merge `main` into OSS `x` and EE `main` into EE `x`; a gitlink conflict resolves to `ee/`'s HEAD. |
| `ship [x] [--admin] [--force] [--build] [--ignore-ee-checks] [--timeout <min>] [--takeover] [--strict] [--log F]` | Brings OSS `x` (and EE `x`) up to date with `main`, and waits for the gating checks (`CLA`, `What changed`, `Policy`, `ee/pointer`; see below), without the lock (a cancelled run is re-run once). Then it takes the global ship lock and brings both up to date again. If `main` came in cleanly and touches none of the PR's files (or nothing changed), it merges on the checks that passed: the EE PR, then OSS `x` pointed at EE `main`, then the OSS PR (squash), and makes sure OSS `main` points at EE `main`. Otherwise (the PR got new commits, or `main` changed files the PR changes; with `--strict` or `FEAT_SHIP_STRICT=1`, any tree change) it lets the lock go and waits for the checks on the new heads first. Logs to a file of its own as well (printed first; `--log F` picks it). `--force` ships despite failing checks, `--build` also waits for an `ee/build` started by hand (`feat sync --pr N --build`), `--ignore-ee-checks` reports failing EE PR checks without gating on them (an EE change merged ahead of its OSS side), `--admin` bypasses branch protection. A PR that changes `.github/workflows` cannot be merged through the API without the token's `workflow` scope: `ship` then pushes the same squash commit to `main` over SSH and closes the PR. |
| `queue [--prune]` | Who holds the ship lock and what every running ship waits for (its checks, the full build, the lock), with ages. `--prune` deletes the tickets of ships that died. |
| `sync [--dry-run] [--pr N] [--build]` | The reconcile the private-side bot runs (needs write access to both repos). |
| `check <pr>` | Dry-run the pointer guard for one OSS PR. |
| `hooks` | Install a pre-push hook that refuses OSS commits pointing at unpushed EE commits. |
| `tree <dir>` | A throwaway full-product tree (copy of this checkout, `ee/` included, + the EE Rust overlay) to build locally. The full UI needs no tree: it is `ee/app`, a package over `ui/` through the extension points (docs/EXTENSIONS.md). |

Typical paired feature:

```sh
just feat start my-thing --ee --worktree ../my-thing && cd ../my-thing
# edit both sides; commit OSS changes here and EE changes in ee/ (cd ee && git commit)
just feat push            # EE first, then the pointer + OSS; PRs opened and cross-linked
just feat status
just feat ship my-thing   # EE merges first, then OSS (squash), under the lock
```

An agent (or anyone) already working in a worktree of its own runs `just feat start my-thing --ee`
there, without `--worktree`: the worktree itself switches to `my-thing`. Agent harnesses that
confine git to the agent's worktree block `--worktree ../x`, and a worktree nested inside the
checkout is refused (its tools would sweep it up). The OSS PR is public: keep private details
out of `--title`/`--body` and put them in `--ee-title`/`--ee-body`.

An OSS-only change is the same without `--ee` (its PR never touches the gitlink); an EE-only
change is the same with only `ee/` commits (only an EE PR is opened).

Keep `ee/` on a branch while you edit it. A plain `git submodule update` checks it out
detached at the pointer; `just feat ee` puts it back on `x`.

## Worktrees (agents and parallel work)

One feature = one OSS worktree. Its `ee/` is that worktree's own checkout of the private repo
(a submodule clone kept under `.git/worktrees/<name>/modules/ee`), so a paired feature needs no
second worktree and no separate clone of the private repo.

1. **Get a worktree.** If your tool already gave you one, use it. Otherwise:
   `git -C <your OSS checkout> worktree add .claude/worktrees/<x> origin/main && cd .claude/worktrees/<x>`
   (`.claude/` is excluded from git, so tools don't sweep it up).
2. **Start or resume the pair** inside it: `just feat start <x> [--ee]`. It branches `x` off
   `origin/main`, or resumes `origin/x` and EE `x` if they exist (someone else's pair). `--ee` (or
   an existing EE `x`) initialises `ee/`, its nested submodules (libvpx), and puts `ee/` on branch
   `x`. Check with `just feat status`. If `ee/` is detached, run `just feat ee`.
3. **Commit on both sides**: OSS changes at the root, EE changes with `git -C ee commit`.
4. **Build the full product** with `just full-*`. It uses `target/full-tree` inside this worktree;
   nothing outside it.
5. **Scratch files** go under the worktree (e.g. `.scratch/`, git-ignored) or in uniquely named
   paths (prefix them with `<x>-`), never at fixed shared paths.
6. **After the merge**: delete the build output (`rm -rf <worktree>/target`), then
   `git worktree remove --force <worktree>` (`--force` because of the submodule) and
   `git branch -D <x>`.

## Automation

All CI runs in this repository, on GitHub-hosted runners; the private repository has none.

- **Pull requests build and test nothing** (owner, 2026-10-19): only `CLA`, `What changed`,
  `Policy` and `ee/pointer` gate a merge (`feat ship` waits for those alone). Run the relevant
  tests locally before pushing. Every build and test (`ci.yml` on all platforms, the full
  product) runs on `main`; a red `main` opens a `ci-main` issue and is fixed forward in a
  follow-up PR.
- **Full build** (`.github/workflows/full.yml`): the full product (this repo + `ee/` overlaid)
  on every push to `main` (Linux, macOS, Windows, web, e2e) and on demand; its aggregate job is
  **`ee/build`**. Pull requests run no builds or tests (owner, 2026-10-19): only CLA, Policy,
  What changed and `ee/pointer` gate a merge, so run the relevant tests locally, and fix `main`
  forward if it goes red. A PR's full build on demand:
  `gh workflow run full.yml --ref <branch> -f ref=<sha> -f all=false` (it uses the commit's `ee`
  pointer; a fork's PR only after reading it: `-f ref=refs/pull/N/head`).
  - The private code never shows in the public log: each step's output goes to a file
    (`scripts/ci/private-step.mjs`), the log shows the result and the names of what failed, and
    the files are uploaded **encrypted with age** to the recipients in `ee/.sops.yaml`.
    `just full-logs <run-id>` downloads and decrypts them.
  - No private build output in the Actions cache (a fork's PR can restore caches made on
    `main`): the web jobs restore `ci.yml`'s OSS-only caches; the Rust jobs cache only the
    third-party dependencies, built by cargo-chef (`.github/actions/chef-cache`: cook's stubs of
    the workspace crates are removed and checked for before the save; key = toolchain, profile,
    flags, OS, then the recipe, with a fallback to the newest cache of the same settings; only
    `main` saves).
  - Runners: repository variables `FULL_RUNNER`, `FULL_MACOS_RUNNER`, `FULL_WINDOWS_RUNNER`,
    `SYNC_RUNNER` (JSON labels), GitHub-hosted by default.
- **Sync** (`.github/workflows/ee-sync.yml`, every 10 minutes and on demand, plus `feat push`/
  `ship`): moves OSS `main` to EE `main` (unless a ship holds the lock), keeps each open OSS
  PR's pointer on its EE branch tip (or EE `main` once that merged), merges `main` into a PR
  whose only conflict is the gitlink, and posts the **`ee/pointer`** status on OSS PRs. Its API
  commits touch nothing but `ee` and are fast-forward only, retried when the branch moved.
- **Release and dev deploy** (`.github/workflows/release.yml`, `deploy-dev.yml`; docs/RELEASING.md):
  the same overlay and encrypted logs; the signing and Cloudflare secrets live in the protected
  `release` and `dev` environments (`main` only; `release` also needs the owner's approval).
- **EE-only changes** (no OSS PR): `feat ship` merges them on the gating checks like any other;
  the `ee` pointer bump that follows is a push to `main`, which runs `full.yml`. To build one
  before merging: `gh workflow run full.yml --ref main -f ref=main -f ee_ref=<EE head>`.

Secrets (this repository): `ETHER_PRIVATE_TOKEN` (read the private repository: the full build)
and `ETHER_BOT_TOKEN` (write this repository's contents, pull requests and statuses, read the
private one: the sync). Without the bot token the sync does nothing; `feat push` and
`feat ship` still do its part for their own PR.

## Concurrency notes

- Only `feat ship` merges paired PRs, one merge at a time: a lock ref in the private repo,
  held only from the final update to the merges (the long CI wait happens before it), renewed
  every few minutes, and considered dead after 30 minutes without renewal. A ship that is
  killed (Ctrl-C, SIGTERM) lets it go. The sync bot leaves the shipping branch and OSS `main`
  alone while the lock is held. Ships in parallel each wait for their own checks; when one
  merges, the others merge in the new `main` on their final update and, unless it touched
  their files, merge right away. A semantic conflict that slips through (two PRs that merge
  cleanly but break together) shows up in `main`'s own CI after the merge: fix it forward.
- Each running ship keeps a ticket ref in the private repo (`locks/queue/<x>`), renewed with
  what it is waiting for; `just feat queue` lists them.
- Squash merges and `main` pointer bumps make the gitlink conflict on open paired PRs; that is
  expected. The bot, `feat update` and `feat ship` resolve it (to the EE branch tip, then EE
  `main`).
- After the EE PR merged, the OSS PR's head only changes the gitlink (to EE `main`, normally
  the very tree that was built), so the `ee/*` statuses carry over and only the required checks
  are waited for again.
- If a ship stops after the EE PR merged (e.g. the OSS merge failed), it keeps the lock; fix
  the cause and run `just feat ship x --takeover` to finish (OSS `x` is pointed at EE `main`).
