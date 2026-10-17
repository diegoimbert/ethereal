#!/usr/bin/env node
// Paired OSS + EE feature branches: the `ee/` submodule workflow (docs/EE-WORKFLOW.md).
//
//   just feat start <x> [--ee] [--worktree <dir>]   branch x off origin/main (and ee/ x with --ee)
//   just feat ee                                    put ee/ on the current branch's name (init it)
//   just feat push [--draft] [--force] [--title T] [--body B]
//                                                   push EE x, then the pointer + OSS x; open/update
//                                                   both PRs, cross-linked
//   just feat update                                merge main into OSS x (and EE main into EE x)
//   just feat status                                branches, pointers, PRs, invariant verdicts
//   just feat ship [x] [--admin] [--force] [--timeout <min>]
//                                                   merge EE first, point OSS x at EE main, merge
//                                                   OSS, under a global lock
//   just feat sync [--dry-run] [--ci]               the reconcile the EE-side bot runs (any token
//                                                   with write access to both repos works)
//   just feat check <oss-pr>                        dry-run the pointer guard for one OSS PR
//   just feat hooks                                 install the pre-push pointer guard
//   just feat tree <dir>                            full-product throwaway tree (ee/overlay.mjs)
//
// Invariants (docs/EE-WORKFLOW.md): OSS branch x pairs with EE branch x; an OSS PR either
// leaves the `ee` gitlink untouched or points it at the tip of EE x; OSS main points at EE
// main; a pushed OSS commit never points at an EE commit that is not pushed.
//
// Everything that talks to GitHub goes through `gh` (its auth, or GH_TOKEN). In CI,
// FEAT_EE_TOKEN (when set) is used for calls to the EE repo and GH_TOKEN for the OSS repo.
import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync, chmodSync } from "node:fs";
import { hostname } from "node:os";
import { join, resolve } from "node:path";

class Fail extends Error {}
const die = (msg) => {
  throw new Fail(msg);
};
const log = (msg) => console.log(msg);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------------- shell

function run(cmd, args, o = {}) {
  const r = spawnSync(cmd, args, {
    cwd: o.cwd ?? ROOT,
    input: o.input,
    encoding: "utf8",
    maxBuffer: 64 << 20,
    env: { ...process.env, ...o.env },
  });
  if (r.error) throw r.error;
  if (r.status !== 0 && !o.allowFail) {
    die(`${cmd} ${args.join(" ")} failed (${r.status}):\n${(r.stderr || r.stdout).trim()}`);
  }
  return o.full ? r : r.stdout.trim();
}
const git = (...a) => run("git", a);
const gitIn = (cwd, ...a) => run("git", a, { cwd });
const gitOk = (cwd, ...a) => run("git", a, { cwd, allowFail: true, full: true }).status === 0;
const gitTry = (cwd, ...a) => {
  const r = run("git", a, { cwd, allowFail: true, full: true });
  return r.status === 0 ? r.stdout.trim() : null;
};

const ROOT = (() => {
  const cwd = process.cwd();
  const r = (a) => spawnSync("git", a, { cwd, encoding: "utf8" }).stdout?.trim();
  return r(["rev-parse", "--show-superproject-working-tree"]) || r(["rev-parse", "--show-toplevel"]) || cwd;
})();
const EE_DIR = join(ROOT, "ee");
const eeReady = () => existsSync(join(EE_DIR, ".git"));

// ---------------------------------------------------------------------------------- repos

function slug(url) {
  const m = url.match(/github\.com[:/]+([^/]+)\/([^/]+?)(?:\.git)?\/?$/);
  return m ? `${m[1]}/${m[2]}` : null;
}
const OSS =
  process.env.FEAT_OSS_REPO ||
  slug(gitTry(ROOT, "remote", "get-url", "origin") ?? "") ||
  die("cannot tell the OSS repo (set FEAT_OSS_REPO=owner/name)");
const EE = (() => {
  if (process.env.FEAT_EE_REPO) return process.env.FEAT_EE_REPO;
  const url = gitTry(ROOT, "config", "-f", ".gitmodules", "submodule.ee.url");
  if (!url) die("no ee submodule in .gitmodules (set FEAT_EE_REPO=owner/name)");
  if (url.startsWith("../")) return `${OSS.split("/")[0]}/${url.slice(3).replace(/\.git$/, "")}`;
  return slug(url) ?? die(`cannot parse the ee submodule url ${url}`);
})();
const LOCK_REF = "locks/ship";
const LOCK_STALE_MIN = 60;
const POINTER_CTX = "ee/pointer";
const BUILD_CTX = "ee/build";

// ---------------------------------------------------------------------------------- GitHub

function ghEnv(repo) {
  return repo === EE && process.env.FEAT_EE_TOKEN ? { GH_TOKEN: process.env.FEAT_EE_TOKEN } : {};
}
function gh(repo, args, o = {}) {
  return run("gh", args, { ...o, env: { ...ghEnv(repo), ...o.env } });
}
class ApiError extends Error {
  constructor(path, status, msg) {
    super(`GitHub API ${path}: HTTP ${status}: ${msg}`);
    this.status = status;
  }
}
function api(repo, path, { method = "GET", body, allow404 = false, paginate = false } = {}) {
  const full = `repos/${repo}/${path}`;
  const args = ["api", "-X", method, full, "-H", "Accept: application/vnd.github+json"];
  if (paginate) args.push("--paginate", "--slurp");
  if (body !== undefined) args.push("--input", "-");
  const r = gh(repo, args, {
    input: body === undefined ? undefined : JSON.stringify(body),
    allowFail: true,
    full: true,
  });
  if (r.status !== 0) {
    const status = Number((r.stderr.match(/HTTP (\d{3})/) ?? [])[1] ?? 0);
    if (allow404 && status === 404) return null;
    throw new ApiError(full, status, (r.stdout || r.stderr).trim().slice(0, 300));
  }
  if (!r.stdout.trim()) return {};
  const out = JSON.parse(r.stdout);
  return paginate ? out.flat() : out;
}
const refSha = (repo, branch) => api(repo, `git/ref/heads/${branch}`, { allow404: true })?.object.sha ?? null;
// The `ee` gitlink of an OSS commit (null if that commit has no ee/).
function apiPointer(ref) {
  const c = api(OSS, `contents/ee?ref=${encodeURIComponent(ref)}`, { allow404: true });
  return c && c.type === "submodule" ? c.sha : null;
}
function prs(repo, head, state = "open") {
  const out = gh(repo, [
    "pr", "list", "-R", repo, "--head", head, "--state", state, "-L", "20",
    "--json", "number,url,headRefName,headRefOid,baseRefName,isCrossRepository,isDraft,title,body,mergeCommit,state",
  ]);
  return JSON.parse(out || "[]").filter((p) => !p.isCrossRepository && p.headRefName === head);
}
const openPr = (repo, head) => prs(repo, head)[0] ?? null;
const mergedPr = (repo, head) => prs(repo, head, "merged")[0] ?? null;

function postStatus(sha, context, state, description, targetUrl) {
  const body = { state, context, description: description.slice(0, 139) };
  if (targetUrl) body.target_url = targetUrl;
  api(OSS, `statuses/${sha}`, { method: "POST", body });
}

// Point `branch` of `repo`'s `ee` gitlink at `eeSha` with a fast-forward API commit; retries
// when someone pushed in between. No clone or checkout needed, never a force push.
function apiSetPointer(branch, eeSha, message) {
  for (let attempt = 0; attempt < 6; attempt++) {
    const head = refSha(OSS, branch);
    if (!head) die(`${OSS} has no branch ${branch}`);
    const commit = api(OSS, `git/commits/${head}`);
    if (apiPointer(head) === eeSha) return { changed: false, head };
    const tree = api(OSS, "git/trees", {
      method: "POST",
      body: { base_tree: commit.tree.sha, tree: [{ path: "ee", mode: "160000", type: "commit", sha: eeSha }] },
    });
    const next = api(OSS, "git/commits", { method: "POST", body: { message, tree: tree.sha, parents: [head] } });
    try {
      api(OSS, `git/refs/heads/${branch}`, { method: "PATCH", body: { sha: next.sha, force: false } });
      return { changed: true, head: next.sha };
    } catch (e) {
      if (!(e instanceof ApiError) || e.status !== 422) throw e;
      log(`  ${branch} moved while updating its pointer; retrying`);
    }
  }
  die(`could not update the ee pointer of ${OSS}:${branch} (kept racing)`);
}

// ---------------------------------------------------------------------------------- lock

function readLock() {
  const refs = api(EE, `git/matching-refs/${LOCK_REF}`) ?? [];
  const ref = refs.find((r) => r.ref === `refs/${LOCK_REF}`);
  if (!ref) return null;
  const c = api(EE, `git/commits/${ref.object.sha}`);
  const field = (k) => (c.message.match(new RegExp(`^${k}: (.*)$`, "m")) ?? [])[1] ?? "";
  const at = Date.parse(c.committer.date);
  return { sha: ref.object.sha, branch: field("branch"), holder: field("holder"), ageMin: (Date.now() - at) / 60000 };
}
async function acquireLock(branch, { takeover = false, waitMin = 30 } = {}) {
  const holder = `${process.env.USER ?? "?"}@${hostname()} pid ${process.pid}`;
  const tree = api(EE, "git/commits/" + refSha(EE, "main")).tree.sha;
  const deadline = Date.now() + waitMin * 60000;
  let told = false;
  for (;;) {
    const mine = api(EE, "git/commits", {
      method: "POST",
      body: { message: `ship lock\n\nbranch: ${branch}\nholder: ${holder}\n`, tree, parents: [] },
    }).sha;
    try {
      api(EE, "git/refs", { method: "POST", body: { ref: `refs/${LOCK_REF}`, sha: mine } });
      return mine;
    } catch (e) {
      if (!(e instanceof ApiError) || e.status !== 422) throw e;
    }
    const cur = readLock();
    if (!cur) continue;
    if (cur.ageMin > LOCK_STALE_MIN || (takeover && cur.branch === branch)) {
      log(`taking over the ship lock (${cur.branch}, ${cur.holder}, ${cur.ageMin.toFixed(0)} min old)`);
      api(EE, `git/refs/${LOCK_REF}`, { method: "PATCH", body: { sha: mine, force: true } });
      if (readLock()?.sha === mine) return mine;
      continue;
    }
    if (Date.now() > deadline) die(`ship lock still held by ${cur.branch} (${cur.holder}); try again later`);
    if (!told) log(`waiting for the ship lock (held by ${cur.branch}, ${cur.holder}) ...`);
    told = true;
    await sleep(15000);
  }
}
function releaseLock(mine) {
  if (readLock()?.sha === mine) api(EE, `git/refs/${LOCK_REF}`, { method: "DELETE" });
}

// ---------------------------------------------------------------------------------- local git

const currentBranch = (cwd = ROOT) => gitTry(cwd, "symbolic-ref", "--short", "-q", "HEAD");
const pointerAt = (rev, cwd = ROOT) => gitTry(cwd, "rev-parse", `${rev}:ee`);
const short = (sha) => (sha ? sha.slice(0, 9) : "-");

function eeState() {
  if (!eeReady()) return { ready: false };
  return {
    ready: true,
    head: gitIn(EE_DIR, "rev-parse", "HEAD"),
    branch: currentBranch(EE_DIR),
    dirty: gitIn(EE_DIR, "status", "--porcelain") !== "",
  };
}
function eeFetch() {
  gitIn(EE_DIR, "fetch", "-q", "--prune", "origin");
}
// Initialise ee/ (the submodule is `update = none`, so clones never fetch it on their own) and
// put it on branch `x`: the existing EE branch x, or a new one off EE main.
function eeEnable(x) {
  if (!eeReady()) {
    log("initialising ee/ ...");
    git("submodule", "update", "--init", "--checkout", "ee");
  }
  const st = eeState();
  if (st.dirty) die("ee/ has uncommitted changes; commit or stash them first");
  eeFetch();
  if (gitOk(EE_DIR, "rev-parse", "--verify", "-q", `refs/heads/${x}`)) gitIn(EE_DIR, "switch", "-q", x);
  else if (gitOk(EE_DIR, "rev-parse", "--verify", "-q", `refs/remotes/origin/${x}`))
    gitIn(EE_DIR, "switch", "-q", "-c", x, "--track", `origin/${x}`);
  else gitIn(EE_DIR, "switch", "-q", "--no-track", "-c", x, "origin/main");
  log(`ee/ is on branch ${x} (${short(gitIn(EE_DIR, "rev-parse", "HEAD"))})`);
}

// Is EE commit `sha` pushed (reachable from an EE branch)? `mainOnly`: from EE main.
function eePushed(sha, { mainOnly = false } = {}) {
  if (eeReady() && gitOk(EE_DIR, "cat-file", "-e", `${sha}^{commit}`)) {
    eeFetch();
    if (mainOnly) return gitOk(EE_DIR, "merge-base", "--is-ancestor", sha, "origin/main");
    return gitIn(EE_DIR, "branch", "-r", "--contains", sha) !== "";
  }
  // No local ee/: ask GitHub (needs read access to the EE repo).
  try {
    const cmp = api(EE, `compare/${sha}...main`);
    if (mainOnly) return cmp.status === "ahead" || cmp.status === "identical";
    return !!api(EE, `commits/${sha}`, { allow404: true });
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------------- PR bodies

const PAIR_START = "<!-- ee-pair -->";
const PAIR_END = "<!-- /ee-pair -->";
function withPair(body, text) {
  const re = new RegExp(`\\n*${PAIR_START}[\\s\\S]*?${PAIR_END}\\n*`, "g");
  const base = (body ?? "").replace(re, "\n").trim();
  return `${base}\n\n${PAIR_START}\n${text}\n${PAIR_END}\n`;
}
function setBody(repo, pr, body) {
  if ((pr.body ?? "").trim() !== body.trim()) gh(repo, ["pr", "edit", String(pr.number), "-R", repo, "--body-file", "-"], { input: body });
}

// ---------------------------------------------------------------------------------- commands

function argVal(args, name) {
  const i = args.indexOf(name);
  if (i < 0) return undefined;
  const v = args[i + 1];
  if (v === undefined || v.startsWith("--")) die(`${name} needs a value`);
  args.splice(i, 2);
  return v;
}
function argFlag(args, name) {
  const i = args.indexOf(name);
  if (i < 0) return false;
  args.splice(i, 1);
  return true;
}
function featBranch() {
  const x = currentBranch();
  if (!x) die("detached HEAD: check out your feature branch");
  if (x === "main") die("you are on main: start a branch with `just feat start <name>`");
  return x;
}

function cmdStart(args) {
  const wt = argVal(args, "--worktree");
  const withEe = argFlag(args, "--ee");
  const x = args[0] ?? die("usage: feat start <branch> [--ee] [--worktree <dir>]");
  if (x === "main" || !gitOk(ROOT, "check-ref-format", "--branch", x)) die(`bad branch name ${x}`);
  git("fetch", "-q", "origin", "main");
  const remote = gitOk(ROOT, "ls-remote", "--exit-code", "--heads", "origin", x);
  if (remote) git("fetch", "-q", "origin", `${x}:refs/remotes/origin/${x}`);
  const from = remote ? `origin/${x}` : "origin/main";
  let cwd = ROOT;
  if (wt) {
    cwd = resolve(wt);
    if (gitOk(ROOT, "rev-parse", "--verify", "-q", `refs/heads/${x}`)) git("worktree", "add", cwd, x);
    else git("worktree", "add", "--no-track", "-b", x, cwd, from);
    log(`worktree ${cwd} on ${x}`);
  } else {
    if (git("status", "--porcelain", "--ignore-submodules=all") !== "") die("working tree has changes; commit or stash them first");
    if (gitOk(ROOT, "rev-parse", "--verify", "-q", `refs/heads/${x}`)) git("switch", "-q", x);
    else git("switch", "-q", "--no-track", "-c", x, from);
    log(`on ${x} (from ${from})`);
  }
  const eeRemote = withEe || eeHasBranch(x);
  if (eeRemote) {
    // In another worktree, re-run there so ROOT/ee resolve to it.
    if (cwd !== ROOT) log(run(process.execPath, [join(cwd, "scripts/feat.mjs"), "ee"], { cwd }));
    else eeEnable(x);
  }
  log(withEe || eeRemote ? "EE work goes in ee/ (on branch " + x + "); `just feat push` when ready." : "OSS only. `just feat ee` adds the EE side.");
}
function eeHasBranch(x) {
  try {
    return !!refSha(EE, x);
  } catch {
    return false;
  }
}

function cmdEe() {
  eeEnable(featBranch());
}

// Push EE x (when it has commits), then the pointer commit and OSS x; open/update the PRs.
function cmdPush(args) {
  const draft = argFlag(args, "--draft");
  const force = argFlag(args, "--force");
  const title = argVal(args, "--title");
  const bodyArg = argVal(args, "--body");
  const bodyFile = argVal(args, "--body-file");
  const body = bodyFile ? readFileSync(bodyFile, "utf8") : bodyArg;
  const x = featBranch();
  git("fetch", "-q", "origin", "main");

  // --- EE side
  const ee = eeState();
  let eeTip = null;
  if (ee.ready) {
    eeFetch();
    if (ee.branch === x) {
      if (ee.dirty) die("ee/ has uncommitted changes: commit them (in ee/) first");
      const ahead = Number(gitIn(EE_DIR, "rev-list", "--count", "origin/main..HEAD"));
      const hasRemote = gitOk(EE_DIR, "rev-parse", "--verify", "-q", `refs/remotes/origin/${x}`);
      if (ahead > 0 || hasRemote) {
        log(`pushing EE ${x} ...`);
        gitIn(EE_DIR, "push", "-q", ...(force ? ["--force-with-lease"] : []), "-u", "origin", `HEAD:refs/heads/${x}`);
        eeTip = ee.head;
      }
    } else if (ee.branch === null) {
      if (pointerAt("HEAD") !== ee.head) die(`ee/ is detached at ${short(ee.head)} with no branch: run \`just feat ee\` to put it on ${x}`);
    } else if (ee.branch === "main") {
      if (Number(gitIn(EE_DIR, "rev-list", "--count", "origin/main..HEAD")) > 0)
        die("ee/ has local commits on main: EE work goes on a branch named like the OSS one (`just feat ee`)");
    } else die(`ee/ is on branch ${ee.branch}, expected ${x} (or main): \`just feat ee\``);
  }

  // --- pointer
  if (eeTip && pointerAt("HEAD") !== eeTip) {
    const staged = git("diff", "--cached", "--name-only").split("\n").filter((f) => f && f !== "ee");
    if (staged.length) die(`staged changes besides ee/ (${staged.join(", ")}): commit them first`);
    git("add", "ee");
    git("commit", "-q", "-m", `ee: point at ${x} (${short(eeTip)})`, "--", "ee");
    log(`pointer commit: ee -> ${short(eeTip)}`);
  }

  // --- OSS side
  const ossChanged = git("diff", "--name-only", "origin/main...HEAD", "--", ".", ":(exclude)ee") !== "";
  const ossRemote = gitOk(ROOT, "ls-remote", "--exit-code", "--heads", "origin", x);
  if (!ossChanged && !eeTip) die("nothing to push: no OSS changes and no EE commits on " + x);
  let ossPushed = false;
  if (ossChanged || ossRemote) {
    const ptr = pointerAt("HEAD");
    const basePtr = pointerAt(git("merge-base", "origin/main", "HEAD"));
    if (ptr && ptr !== basePtr && !eePushed(ptr)) die(`OSS ${x} points at EE ${short(ptr)}, which is not pushed`);
    pushOss(x, force);
    ossPushed = true;
  } else log(`EE-only change: OSS ${x} stays local (OSS main gets a pointer bump when the EE PR merges)`);

  // --- PRs
  let eePr = eeTip ? openPr(EE, x) : null;
  let ossPr = ossChanged ? openPr(OSS, x) : null;
  const subject = (cwd) => gitIn(cwd, "log", "-1", "--format=%s", "--no-merges");
  if (eeTip && !eePr) {
    const t = title ?? subject(EE_DIR);
    gh(EE, ["pr", "create", "-R", EE, "--head", x, "--base", "main", "--title", t, "--body", body ?? "", ...(draft ? ["--draft"] : [])]);
    eePr = openPr(EE, x);
    log(`EE PR: ${eePr.url}`);
  }
  if (ossChanged && !ossPr) {
    const t = title ?? subject(ROOT);
    gh(OSS, ["pr", "create", "-R", OSS, "--head", x, "--base", "main", "--title", t, "--body", body ?? "", ...(draft ? ["--draft"] : [])]);
    ossPr = openPr(OSS, x);
    log(`OSS PR: ${ossPr.url}`);
  }
  if (eePr)
    setBody(EE, eePr, withPair(eePr.body, ossPr ? `Paired OSS PR: ${ossPr.url} (merge with \`just feat ship ${x}\`: this one first).` : "EE-only: no OSS PR. When this merges, OSS main gets an automatic `ee` pointer bump."));
  if (ossPr)
    setBody(OSS, ossPr, withPair(ossPr.body, eePr ? `Paired EE PR: ${eePr.url} (private). The \`ee\` pointer follows its branch; ship with \`just feat ship ${x}\`.` : "Leaves `ee` untouched."));
  triggerSync();
  log(`done: ${[eePr && `EE ${eePr.url}`, ossPr && `OSS ${ossPr.url}`, ossPushed && !ossPr && `OSS ${x} pushed`].filter(Boolean).join("  ")}`);
}

// Push OSS x. A non-fast-forward caused only by the sync bot's pointer commits (they touch
// nothing but `ee`) is safe to overwrite: our pointer is the newer one.
function pushOss(x, force) {
  const r = run("git", ["push", "-q", "-u", "origin", `HEAD:refs/heads/${x}`, ...(force ? ["--force-with-lease"] : [])], { allowFail: true, full: true });
  if (r.status === 0) return;
  if (!/non-fast-forward|fetch first|rejected/.test(r.stderr)) die(`git push failed:\n${r.stderr}`);
  git("fetch", "-q", "origin", `${x}:refs/remotes/origin/${x}`);
  const theirs = git("rev-parse", `origin/${x}`);
  const remoteOnly = git("rev-list", `HEAD..${theirs}`).split("\n").filter(Boolean);
  const onlyPointer = remoteOnly.every((c) => git("diff-tree", "--no-commit-id", "--name-only", "-r", c) === "ee");
  if (!onlyPointer) die(`origin/${x} has commits you don't have; integrate them (git pull --rebase) and push again`);
  log(`origin/${x} only had pointer-bump commits; replacing them with yours`);
  git("push", "-q", "-u", `--force-with-lease=refs/heads/${x}:${theirs}`, "origin", `HEAD:refs/heads/${x}`);
}

// Kick the EE-side sync workflow so pointers/statuses update now instead of on its schedule.
function triggerSync() {
  const r = gh(EE, ["workflow", "run", "ee-sync.yml", "-R", EE, "--ref", "main"], { allowFail: true, full: true });
  if (r.status !== 0) log("(could not trigger the EE sync workflow; it also runs on a schedule)");
}

// Merge origin/main into OSS x and EE main into EE x; a conflict on the gitlink resolves to
// ee/'s HEAD (your EE branch).
function cmdUpdate() {
  const x = featBranch();
  git("fetch", "-q", "origin", "main");
  const ee = eeState();
  let want = null;
  if (ee.ready && ee.branch === x) {
    if (ee.dirty) die("ee/ has uncommitted changes; commit them first");
    eeFetch();
    if (!gitOk(EE_DIR, "merge", "-q", "--no-edit", "origin/main")) die("EE merge of main has conflicts: resolve them in ee/, commit, then re-run");
    want = gitIn(EE_DIR, "rev-parse", "HEAD");
  }
  mergeMain(ROOT, want);
  log(`updated ${x} with main${want ? ` (ee -> ${short(want)})` : ""}; \`just feat push\` to publish`);
}

// Merge origin/main into the branch checked out at `cwd`. Gitlink conflicts resolve to `want`
// (or to ours when null); any other conflict aborts.
function mergeMain(cwd, want) {
  if (gitOk(cwd, "merge-base", "--is-ancestor", "origin/main", "HEAD")) return setPointerLocal(cwd, want);
  const ours = pointerAt("HEAD", cwd);
  if (!gitOk(cwd, "merge", "-q", "--no-edit", "origin/main")) {
    const conflicted = gitIn(cwd, "diff", "--name-only", "--diff-filter=U").split("\n").filter(Boolean);
    if (conflicted.some((f) => f !== "ee")) {
      gitIn(cwd, "merge", "--abort");
      die(`merging main conflicts in ${conflicted.join(", ")}: run \`just feat update\` in your checkout and resolve`);
    }
    gitIn(cwd, "update-index", "--cacheinfo", `160000,${want ?? ours},ee`);
    gitIn(cwd, "commit", "-q", "--no-edit");
  }
  setPointerLocal(cwd, want);
}
function setPointerLocal(cwd, want) {
  if (!want || pointerAt("HEAD", cwd) === want) return;
  gitIn(cwd, "update-index", "--cacheinfo", `160000,${want},ee`);
  gitIn(cwd, "commit", "-q", "-m", `ee: point at ${short(want)}`);
}

function checksOf(repo, pr, required) {
  const r = gh(repo, ["pr", "checks", String(pr), "-R", repo, "--json", "name,bucket", ...(required ? ["--required"] : [])], { allowFail: true, full: true });
  if (!r.stdout.trim()) return [];
  return JSON.parse(r.stdout);
}
async function waitChecks(repo, pr, { required = false, timeoutMin = 45, label }) {
  const deadline = Date.now() + timeoutMin * 60000;
  let told = false;
  for (;;) {
    const cs = checksOf(repo, pr, required);
    const bad = cs.filter((c) => c.bucket === "fail" || c.bucket === "cancel").map((c) => c.name);
    if (bad.length) return { ok: false, why: `failing: ${bad.join(", ")}` };
    const pending = cs.filter((c) => c.bucket === "pending").map((c) => c.name);
    if (!pending.length) return { ok: true };
    if (Date.now() > deadline) return { ok: false, why: `still pending after ${timeoutMin} min: ${pending.join(", ")}` };
    if (!told) log(`waiting for ${label} checks: ${pending.join(", ")} ...`);
    told = true;
    await sleep(20000);
  }
}
async function prField(repo, n, field) {
  for (let i = 0; i < 10; i++) {
    const v = JSON.parse(gh(repo, ["pr", "view", String(n), "-R", repo, "--json", field]))[field];
    if (v !== "UNKNOWN") return v;
    await sleep(3000);
  }
  return "UNKNOWN";
}

function withTempWorktree(ref, fn) {
  const common = resolve(ROOT, git("rev-parse", "--git-common-dir"));
  const dir = join(common, "feat-tmp", `wt-${process.pid}-${Date.now()}`);
  mkdirSync(join(common, "feat-tmp"), { recursive: true });
  git("worktree", "add", "-q", "--detach", dir, ref);
  return Promise.resolve(fn(dir)).finally(() => git("worktree", "remove", "--force", dir));
}
// Push the temp worktree's HEAD to OSS x; tolerate (and drop) the bot's pointer-only commits.
function pushFrom(dir, x) {
  const r = run("git", ["push", "-q", "origin", `HEAD:refs/heads/${x}`], { cwd: dir, allowFail: true, full: true });
  if (r.status === 0) return;
  gitIn(dir, "fetch", "-q", "origin", x);
  const theirs = gitIn(dir, "rev-parse", "FETCH_HEAD");
  const only = gitIn(dir, "rev-list", `HEAD..${theirs}`).split("\n").filter(Boolean);
  if (!only.every((c) => gitIn(dir, "diff-tree", "--no-commit-id", "--name-only", "-r", c) === "ee"))
    die(`OSS ${x} moved on GitHub while shipping; re-run \`just feat ship ${x}\``);
  gitIn(dir, "push", "-q", `--force-with-lease=refs/heads/${x}:${theirs}`, "origin", `HEAD:refs/heads/${x}`);
}

async function cmdShip(args) {
  const admin = argFlag(args, "--admin");
  const force = argFlag(args, "--force");
  const takeover = argFlag(args, "--takeover");
  const timeoutMin = Number(argVal(args, "--timeout") ?? 45);
  const x = args[0] ?? featBranch();
  let ossPr = openPr(OSS, x);
  let eePr = openPr(EE, x);
  const eeMergedBefore = !eePr && !!mergedPr(EE, x);
  if (!ossPr && !eePr) die(`no open PR for ${x} in ${OSS} or ${EE}`);
  log(`shipping ${x}: ${[eePr && `EE #${eePr.number}`, ossPr && `OSS #${ossPr.number}`].filter(Boolean).join(" + ")}`);

  // 1. checks (before taking the lock: waiting here blocks nobody)
  for (const [repo, pr, label] of [[EE, eePr, "EE"], [OSS, ossPr, "OSS"]]) {
    if (!pr) continue;
    const c = await waitChecks(repo, pr.number, { timeoutMin, label });
    if (!c.ok && !force) die(`${label} PR #${pr.number} checks ${c.why} (--force to ship anyway)`);
  }

  const lock = await acquireLock(x, { takeover });
  let keepLock = false;
  try {
    // re-read under the lock
    ossPr = openPr(OSS, x);
    eePr = openPr(EE, x);
    if (eePr) {
      const eeTip = eePr.headRefOid;
      if (ossPr && apiPointer(ossPr.headRefOid) !== eeTip)
        die(`OSS ${x} points at ${short(apiPointer(ossPr.headRefOid))}, EE ${x} is at ${short(eeTip)}: run \`just feat push\` first`);
      if ((await prField(EE, eePr.number, "mergeable")) === "CONFLICTING")
        die(`EE PR #${eePr.number} conflicts with EE main: \`just feat update\`, push, re-run`);
    }
    // 2. OSS x up to date with main before anything merges (fails early on real conflicts)
    if (ossPr) {
      git("fetch", "-q", "origin", "main", x);
      await withTempWorktree(`origin/${x}`, (dir) => {
        gitIn(dir, "fetch", "-q", "origin", "main");
        const before = gitIn(dir, "rev-parse", "HEAD");
        mergeMain(dir, eePr ? eePr.headRefOid : null);
        if (gitIn(dir, "rev-parse", "HEAD") !== before) {
          log(`merged main into OSS ${x}`);
          pushFrom(dir, x);
        }
      });
    }
    // 3. EE first
    let eeMain = refSha(EE, "main");
    if (eePr) {
      log(`merging EE PR #${eePr.number} ...`);
      gh(EE, ["pr", "merge", String(eePr.number), "-R", EE, "--squash"]);
      try {
        api(EE, `git/refs/heads/${x}`, { method: "DELETE" });
      } catch {
        /* already gone */
      }
      eeMain = refSha(EE, "main");
      log(`EE main is now ${short(eeMain)}`);
    }
    if (!ossPr) {
      // EE-only: OSS main follows EE main right away.
      const r = apiSetPointer("main", eeMain, `ee: bump to EE main (${short(eeMain)})`);
      log(r.changed ? `OSS main ee -> ${short(eeMain)}` : "OSS main already points at EE main");
      return;
    }
    // 4. OSS x -> EE main (when the EE side merged, now or in an earlier, interrupted ship)
    if (eePr || eeMergedBefore) {
      keepLock = true; // EE is merged: if OSS fails now, keep main's bump on hold
      await withTempWorktree(`origin/${x}`, (dir) => {
        gitIn(dir, "fetch", "-q", "origin", x);
        gitIn(dir, "reset", "-q", "--hard", "FETCH_HEAD");
        gitIn(dir, "fetch", "-q", "origin", "main");
        mergeMain(dir, eeMain);
        pushFrom(dir, x);
      });
      log(`OSS ${x} ee -> EE main ${short(eeMain)}`);
    }
    // 5. OSS
    for (let attempt = 0; ; attempt++) {
      if (!admin) {
        await sleep(5000); // let the new head's checks register
        const c = await waitChecks(OSS, ossPr.number, { required: true, timeoutMin, label: "required OSS" });
        if (!c.ok && !force) die(`OSS PR #${ossPr.number} required checks ${c.why}. EE is merged; re-run \`just feat ship ${x}\` (the lock is kept for it).`);
      }
      const r = gh(OSS, ["pr", "merge", String(ossPr.number), "-R", OSS, "--squash", ...(admin ? ["--admin"] : [])], { allowFail: true, full: true });
      if (r.status === 0) break;
      const state = await prField(OSS, ossPr.number, "mergeStateStatus");
      if (state !== "BEHIND" || attempt >= 4) die(`merging OSS PR #${ossPr.number} failed (${state}): ${r.stderr.trim()}`);
      log("OSS main moved; merging it in again");
      await withTempWorktree(`origin/${x}`, (dir) => {
        gitIn(dir, "fetch", "-q", "origin", x);
        gitIn(dir, "reset", "-q", "--hard", "FETCH_HEAD");
        gitIn(dir, "fetch", "-q", "origin", "main");
        mergeMain(dir, eePr || eeMergedBefore ? refSha(EE, "main") : null);
        pushFrom(dir, x);
      });
    }
    keepLock = false;
    try {
      api(OSS, `git/refs/heads/${x}`, { method: "DELETE" });
    } catch {
      /* already gone */
    }
    log(`merged OSS PR #${ossPr.number}`);
    // 6. main must end at EE main
    const tip = refSha(EE, "main");
    if (apiPointer("main") !== tip) {
      apiSetPointer("main", tip, `ee: bump to EE main (${short(tip)})`);
      log(`OSS main ee -> ${short(tip)}`);
    }
  } finally {
    if (keepLock) log(`keeping the ship lock for ${x} (re-run \`just feat ship ${x}\` to finish)`);
    else releaseLock(lock);
    triggerSync();
  }
  log(`shipped ${x}`);
}

// Reconcile the invariants (what the EE-side bot runs): OSS main -> EE main (unless a ship
// holds the lock), every open OSS PR's pointer -> its EE branch tip (or EE main once the EE
// PR merged), and the ee/pointer status on each OSS PR head.
async function cmdSync(args) {
  const dry = argFlag(args, "--dry-run");
  const ci = argFlag(args, "--ci");
  const only = argVal(args, "--pr");
  const write = (what, fn) => {
    if (dry) return log(`  [dry-run] would ${what}`);
    try {
      return fn();
    } catch (e) {
      if (e instanceof ApiError && (e.status === 403 || e.status === 404)) {
        log(`${ci ? "::warning::" : ""}no write access for: ${what} (HTTP ${e.status}; set a token with write access, see docs/EE-WORKFLOW.md)`);
        return null;
      }
      throw e;
    }
  };
  const eeMain = refSha(EE, "main");
  const lock = readLock();
  const lockFresh = lock && lock.ageMin <= LOCK_STALE_MIN;
  log(`EE main ${short(eeMain)}${lock ? `; ship lock: ${lock.branch} (${lock.ageMin.toFixed(0)} min${lockFresh ? "" : ", stale"})` : ""}`);

  if (!only) {
    const ossMainPtr = apiPointer("main");
    if (ossMainPtr === null) log("OSS main has no ee/ gitlink yet: nothing to bump");
    else if (ossMainPtr === eeMain) log("OSS main points at EE main");
    else if (lockFresh) log(`OSS main points at ${short(ossMainPtr)}; a ship holds the lock, leaving it`);
    else {
      log(`OSS main points at ${short(ossMainPtr)} -> bumping to ${short(eeMain)}`);
      write(`bump OSS main ee -> ${short(eeMain)}`, () => apiSetPointer("main", eeMain, `ee: bump to EE main (${short(eeMain)})`));
    }
  }

  const list = JSON.parse(
    gh(OSS, ["pr", "list", "-R", OSS, "--state", "open", "-L", "100", "--json", "number,headRefName,headRefOid,baseRefName,isCrossRepository,isDraft"]) || "[]",
  ).filter((p) => !only || String(p.number) === String(only));
  const eePrs = JSON.parse(gh(EE, ["pr", "list", "-R", EE, "--state", "all", "-L", "200", "--json", "headRefName,state"]) || "[]");
  const eeOpenHeads = new Set(eePrs.filter((p) => p.state === "OPEN").map((p) => p.headRefName));
  const eeMergedHeads = new Set(eePrs.filter((p) => p.state === "MERGED").map((p) => p.headRefName));
  const eeBranches = (api(EE, "git/matching-refs/heads/", { paginate: true }) ?? []).map((r) => [r.ref.slice("refs/heads/".length), r.object.sha]);
  const eeTips = new Map(eeBranches);
  for (const pr of list) {
    const x = pr.headRefName;
    const head = pr.headRefOid;
    const ptr = apiPointer(head);
    if (ptr === null) continue; // a branch from before the ee/ submodule existed
    const base = api(OSS, `compare/${pr.baseRefName}...${head}`).merge_base_commit.sha;
    const touched = ptr !== apiPointer(base);
    let want = null;
    let why = "";
    if (pr.isCrossRepository) {
      // A fork can't have an EE branch: it must leave ee alone.
      const state = touched ? "failure" : "success";
      log(`#${pr.number} (fork) ${state}`);
      write(`status #${pr.number}`, () => postStatus(head, POINTER_CTX, state, touched ? "forks cannot change the ee pointer" : "leaves ee untouched"));
      continue;
    }
    const eeTip = eeTips.get(x) ?? null;
    if (!eeOpenHeads.has(x) && eeMergedHeads.has(x)) [want, why] = [eeMain, "EE side merged: follows EE main"];
    else if (eeTip) [want, why] = [eeTip, "points at the tip of the paired EE branch"];
    let cur = head;
    if (want && ptr !== want) {
      if (lockFresh && lock.branch === x) log(`#${pr.number} ${x}: being shipped, leaving its pointer`);
      else {
        log(`#${pr.number} ${x}: ee ${short(ptr)} -> ${short(want)}`);
        const r = write(`point OSS ${x} at ${short(want)}`, () => apiSetPointer(x, want, `ee: point at ${x} (${short(want)})`));
        if (r?.changed) cur = r.head;
      }
    }
    const now = cur === head ? ptr : want;
    let state, desc;
    if (want && now === want) [state, desc] = ["success", why];
    else if (want) [state, desc] = ["pending", "pointer is behind the paired EE branch; it is being updated"];
    else if (!touched) [state, desc] = ["success", "leaves ee untouched"];
    else [state, desc] = ["failure", "points at an EE commit that is not the tip of a paired EE branch (just feat push)"];
    log(`#${pr.number} ${x}: ${state} (${desc})`);
    write(`status #${pr.number}`, () => postStatus(cur, POINTER_CTX, state, desc));
    if (ci && !pr.isDraft) ensureBuild(pr.number, cur, write);
  }
  if (only) return;
  // OSS branches without a PR still follow their EE twin (invariant 3).
  const withPr = new Set(list.map((p) => p.headRefName));
  for (const [x, tip] of eeBranches) {
    if (x === "main" || withPr.has(x) || (lockFresh && lock.branch === x)) continue;
    if (!refSha(OSS, x)) continue;
    const ptr = apiPointer(x);
    if (ptr === null || ptr === tip) continue;
    log(`OSS ${x} (no PR): ee ${short(ptr)} -> ${short(tip)}`);
    write(`point OSS ${x} at ${short(tip)}`, () => apiSetPointer(x, tip, `ee: point at ${x} (${short(tip)})`));
  }
}

// Start the EE-side full build of an OSS PR head once (marks it pending first).
function ensureBuild(n, sha, write) {
  const st = api(OSS, `commits/${sha}/status`);
  if (st.statuses?.some((s) => s.context === BUILD_CTX)) return;
  log(`#${n}: starting the full build`);
  write(`start build #${n}`, () => {
    const r = gh(EE, ["workflow", "run", "oss-pr.yml", "-R", EE, "--ref", "main", "-f", `pr=${n}`, "-f", `sha=${sha}`], { allowFail: true, full: true });
    if (r.status !== 0) return log(`  could not start it: ${r.stderr.trim()}`);
    postStatus(sha, BUILD_CTX, "pending", "full build queued");
  });
}

async function cmdCheck(args) {
  const n = args[0] ?? die("usage: feat check <oss-pr-number>");
  await cmdSync(["--dry-run", "--pr", n]);
}

function cmdStatus() {
  const x = currentBranch();
  git("fetch", "-q", "origin", "main");
  const ee = eeState();
  if (ee.ready) eeFetch();
  const ptr = pointerAt("HEAD");
  const row = (k, v) => log(`  ${k.padEnd(18)} ${v}`);
  log(`OSS ${OSS} (EE: ${EE})`);
  row("branch", x ?? "(detached)");
  row("ee pointer", short(ptr));
  row("OSS main pointer", short(pointerAt("origin/main")));
  if (ee.ready) {
    row("ee/ HEAD", `${short(ee.head)} on ${ee.branch ?? "DETACHED"}${ee.dirty ? ", uncommitted changes" : ""}`);
    row("EE main", short(gitTry(EE_DIR, "rev-parse", "origin/main")));
    if (x) row(`EE ${x}`, short(gitTry(EE_DIR, "rev-parse", "-q", "--verify", `refs/remotes/origin/${x}`)));
  } else row("ee/", "not initialised (`just feat ee`)");
  if (x && x !== "main") {
    const o = openPr(OSS, x);
    const e = openPr(EE, x);
    row("OSS PR", o ? `${o.url}${o.isDraft ? " (draft)" : ""}` : "-");
    row("EE PR", e ? e.url : "-");
    for (const [repo, pr, label] of [[OSS, o, "OSS"], [EE, e, "EE"]]) {
      if (!pr) continue;
      const cs = checksOf(repo, pr.number, false);
      const by = (b) => cs.filter((c) => c.bucket === b).length;
      row(`${label} checks`, `${by("pass")} pass, ${by("fail") + by("cancel")} fail, ${by("pending")} pending`);
    }
    const warn = [];
    if (ee.ready && ee.branch === null && ee.head !== ptr) warn.push("ee/ is detached with changes: `just feat ee`");
    if (ee.ready && ee.branch === x && ee.head !== ptr) warn.push("ee/ HEAD differs from the pointer: `just feat push`");
    const eeRemote = ee.ready && x ? gitTry(EE_DIR, "rev-parse", "-q", "--verify", `refs/remotes/origin/${x}`) : null;
    if (eeRemote && ee.head !== eeRemote && ee.branch === x) warn.push(`EE ${x} has unpushed commits`);
    for (const w of warn) log(`  ! ${w}`);
    if (!warn.length) log("  ok");
  }
}

function cmdHooks() {
  const hooks = resolve(ROOT, git("rev-parse", "--git-common-dir"), "hooks");
  mkdirSync(hooks, { recursive: true });
  const path = join(hooks, "pre-push");
  const marker = "# feat.mjs pointer guard";
  const line = `${marker}\nf="$(git rev-parse --show-toplevel)/scripts/feat.mjs"; [ -f "$f" ] && { node "$f" pre-push "$@" || exit 1; }\n`;
  const cur = existsSync(path) ? readFileSync(path, "utf8") : "#!/bin/sh\n";
  if (cur.includes(marker)) return log(`${path} already has the guard`);
  writeFileSync(path, cur.endsWith("\n") ? cur + line : `${cur}\n${line}`);
  chmodSync(path, 0o755);
  log(`installed the pointer guard in ${path}`);
}

// pre-push hook: refuse to push an OSS commit whose ee gitlink is not pushed to EE (and, for
// main, not on EE main). Reads `<local ref> <local sha> <remote ref> <remote sha>` lines.
function cmdPrePush(args) {
  const url = args[1] ?? "";
  if (slug(url) !== OSS) return; // pushes to other remotes
  const input = readFileSync(0, "utf8");
  const zero = /^0+$/;
  for (const line of input.split("\n").filter(Boolean)) {
    const [, local, remoteRef, remote] = line.split(" ");
    if (zero.test(local)) continue; // deletion
    const range = zero.test(remote) ? [local, "--not", "--remotes=origin"] : [`${remote}..${local}`];
    const commits = gitTry(ROOT, "rev-list", ...range)?.split("\n").filter(Boolean) ?? [];
    const seen = new Set();
    for (const c of commits) {
      const p = pointerAt(c);
      if (!p || seen.has(p)) continue;
      seen.add(p);
      const parent = pointerAt(`${c}^`);
      if (p === parent) continue;
      const mainOnly = remoteRef === "refs/heads/main";
      if (!eePushed(p, { mainOnly })) {
        console.error(`pre-push: ${short(c)} points ee at ${short(p)}, which is not pushed to ${EE}${mainOnly ? " main" : ""}.`);
        console.error("  Push the EE branch first (`just feat push` does EE, then OSS).");
        process.exit(1);
      }
    }
  }
}

function cmdTree(args) {
  const dir = args[0] ?? die("usage: feat tree <dir>");
  if (!existsSync(join(EE_DIR, "overlay.mjs"))) die("ee/ is not initialised (`just feat ee`)");
  run(process.execPath, [join(EE_DIR, "overlay.mjs"), "--make", resolve(dir), "--from", ROOT], { cwd: ROOT });
  log(`full-product tree in ${resolve(dir)} (throwaway; delete it when done)`);
}

const commands = {
  start: cmdStart,
  ee: cmdEe,
  push: cmdPush,
  update: cmdUpdate,
  status: cmdStatus,
  ship: cmdShip,
  sync: cmdSync,
  check: cmdCheck,
  hooks: cmdHooks,
  "pre-push": cmdPrePush,
  tree: cmdTree,
};

const [name, ...rest] = process.argv.slice(2);
const cmd = commands[name];
if (!cmd) {
  console.error(`usage: just feat <${Object.keys(commands).filter((c) => c !== "pre-push").join("|")}> ... (docs/EE-WORKFLOW.md)`);
  process.exit(2);
}
try {
  await cmd(rest);
} catch (e) {
  if (e instanceof Fail || e instanceof ApiError) {
    console.error(`feat ${name}: ${e.message}`);
    process.exit(1);
  }
  throw e;
}
