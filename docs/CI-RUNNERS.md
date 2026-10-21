# CI runners

Pull requests run only the cheap checks (`CLA`, `What changed`, `Policy`, `ee/pointer`); the
builds and tests run on `main` and on manual runs. GitHub's hosted Linux runners have 4 vCPUs and
start cold every time (the Rust job takes ~36 min there), so a self-hosted **runner pool** on a
build machine (the "devbox") can run the Linux jobs with warm caches. macOS and Windows jobs stay on
GitHub-hosted runners.

## Who runs where

| job                                           | runner                                                                          |
| --------------------------------------------- | ------------------------------------------------------------------------------- |
| Linux jobs of a push to `main` or a manual run, but e2e | the pool, when the repository variable `CI_POOL` is `devbox`; else GitHub-hosted |
| the full product's Linux build jobs (`full.yml`), but e2e | the pool, when `FULL_RUNNER` is `["self-hosted","devbox-pool"]`; else GitHub-hosted |
| e2e shards                                    | GitHub-hosted (they build nothing, and four of them per run would fill the pool) |
| the full product's e2e (`full.yml`)           | GitHub-hosted, or the pool when `FULL_E2E_RUNNER` names it: then one job builds what the four shards need and the shards use its target dir (`scripts/ci/e2e-shared-target.mjs`); that job also deals the tests to the shards by their last times on the machine (`scripts/ci/e2e-shard-plan.mjs`) |
| pull requests (forks included), any bot       | always GitHub-hosted                                                            |
| macOS, Windows                                | always GitHub-hosted                                                            |

This repository is public, and a pull request runs the workflow files of its own head branch, so a
fork could edit a workflow to ask for the pool. Three layers stop that:

1. Workflow runs from outside contributors wait for a maintainer's approval (Settings, Actions,
   "Require approval for all external contributors").
2. The pool's runners refuse any job whose event is not trusted before its first step runs: a
   job-started hook (`scripts/ci/runner/trust.mjs`) that lives in the pool's own checkout, outside
   anything a workflow can change.
3. Every job runs in its own sandboxed systemd unit (below).

Trusted jobs build the same code the maintainers build on that machine anyway; the sandbox is
defense in depth. The pool's caches can hold private build output, which is why no untrusted job
may ever reach them.

The full product's jobs (`full.yml`) fetch the private `ee/` repository into their checkout. Their
cargo target dirs are their own (`full-*` job ids), and the hooks empty `ee/` from the slot's
workspace after every job and again before the next (`scripts/ci/runner/clean-workspace.sh`), so an
open-source job that lands on the same slot, whose artifacts are public, never sees private code.

## Runs on `main`

Each of CI, CI (Windows), Full product and Full product (macOS, Windows) has one concurrency group
for pushes to `main`. A streak of merges builds only its last commit: the runs waiting in the group
are skipped (GitHub reports them as cancelled), and `Supersede main runs` (`main-supersede.yml`,
`scripts/ci/main-supersede.mjs`) cancels the running one as soon as a newer push's run waits, so the
newest starts at once. Starvation guard: when a workflow has had no green or red push run on `main`
for more than ~2 h (counted from the later of its last finished run and the first push after it),
the running run is let finish. Manual runs, re-runs, `release.yml` and EE sync are never cancelled.
`Main status` (`main-status.yml`) reads only green and red runs: a cancelled run never opens,
comments on or closes the `ci-main` issue.

A cancelled job saves no Rust cache (a last `if: cancelled()` step turns rust-cache's
`cache-on-failure` off; the superseding run saves instead). On the pool, a cancelled job ends its
runner, and systemd then stops the rest of its unit (cargo, sccache, the target dir's lock holder):
the dir is free for the next job, and cargo rebuilds whatever was cut short (its fingerprints are
written last). The e2e build drops its target dir's stamp before it builds, so a build cut short
never passes for the earlier one; a cancelled shard records no times.

## The pool

`scripts/ci/runner/pool.mjs run` keeps `CI_POOL_SLOTS` ephemeral runners alive. Each one is a
transient systemd user unit that registers with a just-in-time config, runs one job, and exits; the
supervisor then starts a fresh one in that slot. Everything lives in the pool's own directory
(`CI_POOL_HOME`, `~/ci-pool`): its runner, Rust toolchain (rustup), sccache, caches.

The sandbox of a job's unit:

- the owner's home is replaced by an empty tmpfs, with only the pool's directory and the pool's
  checkout (read-only) mounted back: no `~/.ssh`, `~/.config/gh`, sops/age keys or other repos;
- an empty environment (`env -i`) plus the pool's variables: no tokens, no agent sockets;
- no user D-Bus or `/run/user/<uid>` sockets, no Docker socket, a private `/tmp`, its own PID
  namespace (no view of other processes), a read-only system, `NoNewPrivileges`;
- the network is the host's (jobs download dependencies).

Limits: the jobs run in `ethereal-ci-pool.slice` (CPU quota, memory, low CPU and IO weight, so the
owner's work goes first), inside `ethereal-ci.slice`, which caps all CI on the machine together.
A thermal guard (`thermal-guard.sh`, every 20 s from `ethereal-ci-thermal.timer`) lowers both
slices' CPU quota while the CPU (Tctl) or a DIMM runs hot (default: Tctl >= 90 C or DIMM >= 75 C ->
half of `CI_CPUS`) and gives it back once both have cooled (Tctl <= 80 C and DIMM <= 65 C). Its
thresholds are `CI_THERMAL_*` in `~/.config/ethereal-ci/env` (header of the script); its
transitions go to `~/ci-pool/thermal/log`, the last reading to `~/ci-pool/thermal/last`.

Caches, shared by every slot: the cargo registry, sccache (one server per slot, never the owner's),
the pnpm store, Playwright's browsers. Each slot keeps its runner workspace (a checkout only
rewrites the files that changed, so cargo rebuilds just the crates they touch), mounted at the same
path in every slot. Cargo target dirs belong to a kind of job (`targets/<job>-<n>`, locked for the
job's duration), so a job reuses its kind's last build whichever slot runs it. A target dir above
`CI_POOL_TARGET_MAX_GB` is emptied, one unused for `CI_POOL_TARGET_DAYS` is deleted, and no job
starts below `CI_POOL_MIN_FREE_GB` of free disk. See the header of `pool.mjs` for every setting.

### Install on a machine

```sh
git clone https://github.com/diegoimbert/ethereal ~/ci/ethereal
mkdir -p ~/.config/ethereal-ci
# A fine-grained token for this repository with "Administration: read and write" (runner
# registration); without it the pool uses `gh auth token`.
echo 'GH_TOKEN=github_pat_...' > ~/.config/ethereal-ci/env && chmod 600 ~/.config/ethereal-ci/env
cp ~/ci/ethereal/scripts/ci/runner/ethereal-ci.slice ~/ci/ethereal/scripts/ci/runner/ethereal-ci-pool.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now ethereal-ci-pool
node ~/ci/ethereal/scripts/ci/runner/pool.mjs status
```

The unit runs `pool.mjs setup` first (runner, toolchain, sccache, slices, thermal guard; idempotent). The machine
needs the build's system libraries (the `apt-get install` lines of `ci.yml`). Then set the
repository variable: `gh variable set CI_POOL --body devbox`. A second machine runs the same unit
(its runners get unique names); the jobs go to whichever pool runner is free.

### Turn it off

`gh variable delete CI_POOL` sends every job back to GitHub-hosted runners from the next run on.
`systemctl --user stop ethereal-ci-pool && node scripts/ci/runner/pool.mjs stop` removes the idle
runners; busy ones finish their job.
