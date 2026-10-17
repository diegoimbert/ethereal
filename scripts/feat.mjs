#!/usr/bin/env node
// Paired OSS + EE feature branches: the `ee/` submodule workflow (docs/EE-WORKFLOW.md).
//
//   just feat start <x> [--ee] [--worktree <dir>]   branch x off origin/main (and ee/ x with --ee)
//   just feat ee                                    put ee/ on the current branch's name (init it)
//   just feat push [--draft] [--title T] [--body B | --body-file F]
//                                                   push EE x, then the pointer + OSS x; open/update
//                                                   both PRs, cross-linked; start the full build
//   just feat update                                merge main into OSS x (and EE main into EE x)
//   just feat status                                branches, pointers, PRs, checks, warnings
//   just feat ship [x] [--admin] [--force] [--no-build] [--timeout <min>] [--takeover]
//                                                   under a global lock: bring both sides up to
//                                                   date, wait for every check, merge EE, point
//                                                   OSS x at EE main, merge OSS
//   just feat sync [--dry-run] [--ci] [--pr N]      the reconcile the EE-side bot runs
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
// The integration branch of both repos. Only for rehearsing the workflow against a throwaway
// pair of base branches (FEAT_MAIN=<name>); the bots and the workflows always use main.
const MAIN = process.env.FEAT_MAIN || "main";
const LOCK_REF = "locks/ship";
const LOCK_STALE_MIN = 30; // no heartbeat for this long: the ship died
const LOCK_BEAT_MIN = 3;
const POINTER_CTX = "ee/pointer";
const BUILD_CTX = "ee/build";
const BUILD_WORKFLOW = "oss-pr.yml";

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
const treeOf = (repo, sha) => api(repo, `git/commits/${sha}`).tree.sha;
// The `ee` gitlink of an OSS commit (null if that commit has no ee/).
function apiPointer(ref) {
  const c = api(OSS, `contents/ee?ref=${encodeURIComponent(ref)}`, { allow404: true });
  return c && c.type === "submodule" ? c.sha : null;
}
// Is `a` an ancestor of (or equal to) `b` in `repo`?
function apiContains(repo, b, a) {
  const st = api(repo, `compare/${a}...${b}`).status;
  return st === "ahead" || st === "identical";
}
const PR_FIELDS = "number,url,headRefName,headRefOid,baseRefName,isCrossRepository,isDraft,title,body,mergeCommit,state";
function prs(repo, head, state = "open") {
  const out = gh(repo, ["pr", "list", "-R", repo, "--head", head, "--state", state, "-L", "20", "--json", PR_FIELDS]);
  return JSON.parse(out || "[]").filter((p) => !p.isCrossRepository && p.headRefName === head);
}
const openPr = (repo, head) => prs(repo, head)[0] ?? null;
const mergedPr = (repo, head) => prs(repo, head, "merged")[0] ?? null;

function statusesOf(sha) {
  const out = api(OSS, `commits/${sha}/status`);
  return new Map((out.statuses ?? []).map((s) => [s.context, s]));
}
function postStatus(sha, context, state, description, targetUrl) {
  const body = { state, context, description: description.slice(0, 139) };
  if (targetUrl) body.target_url = targetUrl;
  api(OSS, `statuses/${sha}`, { method: "POST", body });
}

// Point `branch` of the OSS repo's `ee` gitlink at `eeSha` with a fast-forward API commit;
// retries when someone pushed in between. No clone or checkout needed, never a force push.
function apiSetPointer(branch, eeSha, message) {
  for (let attempt = 0; attempt < 6; attempt++) {
    const head = refSha(OSS, branch);
    if (!head) die(`${OSS} has no branch ${branch}`);
    if (apiPointer(head) === eeSha) return { changed: false, head };
    const tree = api(OSS, "git/trees", {
      method: "POST",
      body: { base_tree: treeOf(OSS, head), tree: [{ path: "ee", mode: "160000", type: "commit", sha: eeSha }] },
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
//
// One ship at a time: a ref in the EE repo, created atomically (POST fails if it exists). The
// holder renews it (a child commit, fast-forward) every few minutes while it waits; a lock
// with no renewal for LOCK_STALE_MIN is dead and can be taken over.

function readLock() {
  const refs = api(EE, `git/matching-refs/${LOCK_REF}`) ?? [];
  const ref = refs.find((r) => r.ref === `refs/${LOCK_REF}`);
  if (!ref) return null;
  const c = api(EE, `git/commits/${ref.object.sha}`);
  const field = (k) => (c.message.match(new RegExp(`^${k}: (.*)$`, "m")) ?? [])[1] ?? "";
  const at = Date.parse(c.committer.date);
  return { sha: ref.object.sha, branch: field("branch"), holder: field("holder"), ageMin: (Date.now() - at) / 60000 };
}
const lockFresh = (l) => !!l && l.ageMin <= LOCK_STALE_MIN;

class Lock {
  constructor(sha, branch) {
    this.sha = sha;
    this.branch = branch;
    this.beatAt = Date.now();
  }
  static async acquire(branch, { takeover = false, waitMin = 120 } = {}) {
    const holder = `${process.env.USER ?? "?"}@${hostname()} pid ${process.pid}`;
    const tree = treeOf(EE, refSha(EE, MAIN));
    const deadline = Date.now() + waitMin * 60000;
    let told = false;
    for (;;) {
      const mine = api(EE, "git/commits", {
        method: "POST",
        body: { message: `ship lock\n\nbranch: ${branch}\nholder: ${holder}\n`, tree, parents: [] },
      }).sha;
      try {
        api(EE, "git/refs", { method: "POST", body: { ref: `refs/${LOCK_REF}`, sha: mine } });
        return new Lock(mine, branch);
      } catch (e) {
        if (!(e instanceof ApiError) || e.status !== 422) throw e;
      }
      const cur = readLock();
      if (!cur) continue;
      if (!lockFresh(cur) || (takeover && cur.branch === branch)) {
        log(`taking over the ship lock (${cur.branch}, ${cur.holder}, ${cur.ageMin.toFixed(0)} min since its last renewal)`);
        try {
          api(EE, `git/refs/${LOCK_REF}`, { method: "PATCH", body: { sha: mine, force: true } });
        } catch {
          /* lost a race: look again */
        }
        if (readLock()?.sha === mine) return new Lock(mine, branch);
        continue;
      }
      if (Date.now() > deadline) die(`ship lock still held by ${cur.branch} (${cur.holder}); try again later`);
      if (!told) log(`waiting for the ship lock (held by ${cur.branch}, ${cur.holder}) ...`);
      told = true;
      await sleep(15000);
    }
  }
  // Renew (throttled). Dies if someone took the lock over.
  beat(force = false) {
    if (!force && Date.now() - this.beatAt < LOCK_BEAT_MIN * 60000) return;
    const c = api(EE, `git/commits/${this.sha}`);
    const next = api(EE, "git/commits", { method: "POST", body: { message: c.message, tree: c.tree.sha, parents: [this.sha] } }).sha;
    try {
      api(EE, `git/refs/${LOCK_REF}`, { method: "PATCH", body: { sha: next, force: false } });
    } catch (e) {
      if (e instanceof ApiError && e.status === 422) die("lost the ship lock (taken over after it went stale); re-run the ship");
      throw e;
    }
    this.sha = next;
    this.beatAt = Date.now();
  }
  release() {
    if (readLock()?.sha === this.sha) api(EE, `git/refs/${LOCK_REF}`, { method: "DELETE" });
  }
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
  else gitIn(EE_DIR, "switch", "-q", "--no-track", "-c", x, `origin/${MAIN}`);
  log(`ee/ is on branch ${x} (${short(gitIn(EE_DIR, "rev-parse", "HEAD"))})`);
}

// Is EE commit `sha` pushed (reachable from an EE branch)? `mainOnly`: from EE main.
function eePushed(sha, { mainOnly = false } = {}) {
  if (eeReady() && gitOk(EE_DIR, "cat-file", "-e", `${sha}^{commit}`)) {
    eeFetch();
    if (mainOnly) return gitOk(EE_DIR, "merge-base", "--is-ancestor", sha, `origin/${MAIN}`);
    return gitIn(EE_DIR, "branch", "-r", "--contains", sha) !== "";
  }
  // No local ee/: ask GitHub (needs read access to the EE repo).
  try {
    if (mainOnly) return apiContains(EE, MAIN, sha);
    return !!api(EE, `commits/${sha}`, { allow404: true });
  } catch {
    return false;
  }
}

// Merge `ref` into the branch checked out at `cwd`. A conflict on the `ee` gitlink resolves to
// `want` (ours when null); any other conflict aborts the merge and fails. Then the pointer is
// set to `want` (when given).
function mergeInto(cwd, ref, want, into = currentBranch(cwd) ?? "the branch") {
  if (!gitOk(cwd, "merge-base", "--is-ancestor", ref, "HEAD")) {
    const ours = pointerAt("HEAD", cwd);
    const name = ref.replace(/^origin\//, "");
    const msg = `Merge ${name} into ${into}${want ? ` (ee -> ${short(want)})` : ""}`;
    if (!gitOk(cwd, "merge", "-q", "--no-edit", "-m", msg, ref)) {
      const conflicted = gitIn(cwd, "diff", "--name-only", "--diff-filter=U").split("\n").filter(Boolean);
      if (!conflicted.length || conflicted.some((f) => f !== "ee")) {
        gitOk(cwd, "merge", "--abort");
        die(`merging ${ref} conflicts in ${conflicted.join(", ") || "(unknown)"}: run \`just feat update\` and resolve them`);
      }
      gitIn(cwd, "update-index", "--cacheinfo", `160000,${want ?? ours},ee`);
      gitIn(cwd, "commit", "-q", "--no-edit");
    }
  }
  if (want && pointerAt("HEAD", cwd) !== want) {
    gitIn(cwd, "update-index", "--cacheinfo", `160000,${want},ee`);
    gitIn(cwd, "commit", "-q", "-m", `ee: point at ${short(want)}`);
  }
}

const pointerOnly = (cwd, c) =>
  gitIn(cwd, "rev-list", "--no-walk", "--parents", c).split(" ").length === 2 &&
  gitIn(cwd, "diff-tree", "--no-commit-id", "--name-only", "-r", c) === "ee";

// Push HEAD of `cwd` to OSS x. If x moved on GitHub (the sync bot's pointer commits or its
// gitlink-conflict merges, or a ship): pointer-only commits are replaced (ours is newer),
// anything else is merged in (gitlink conflicts resolve to ours) and the push retried.
function pushBranch(cwd, x) {
  for (let attempt = 0; attempt < 4; attempt++) {
    const u = currentBranch(cwd) ? ["-u"] : [];
    const r = run("git", ["push", "-q", ...u, "origin", `HEAD:refs/heads/${x}`], { cwd, allowFail: true, full: true });
    if (r.status === 0) return;
    if (!/non-fast-forward|fetch first|rejected/.test(r.stderr)) die(`git push failed:\n${r.stderr}`);
    gitIn(cwd, "fetch", "-q", "origin", `+refs/heads/${x}:refs/remotes/origin/${x}`);
    const theirs = gitIn(cwd, "rev-parse", `origin/${x}`);
    const remoteOnly = gitIn(cwd, "rev-list", `HEAD..${theirs}`).split("\n").filter(Boolean);
    if (remoteOnly.every((c) => pointerOnly(cwd, c))) {
      log(`origin/${x} only had pointer commits; replacing them with yours`);
      const f = run("git", ["push", "-q", ...u, `--force-with-lease=refs/heads/${x}:${theirs}`, "origin", `HEAD:refs/heads/${x}`], { cwd, allowFail: true, full: true });
      if (f.status === 0) return;
      continue;
    }
    log(`origin/${x} has ${remoteOnly.length} commit(s) you don't have; merging them`);
    mergeInto(cwd, `origin/${x}`, pointerAt("HEAD", cwd), x);
  }
  die(`could not push ${x} (it kept moving)`);
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
  if (x === MAIN) die("you are on main: start a branch with `just feat start <name>`");
  return x;
}

function cmdStart(args) {
  const wt = argVal(args, "--worktree");
  const withEe = argFlag(args, "--ee");
  const x = args[0] ?? die("usage: feat start <branch> [--ee] [--worktree <dir>]");
  if (x === MAIN || !gitOk(ROOT, "check-ref-format", "--branch", x)) die(`bad branch name ${x}`);
  git("fetch", "-q", "origin", MAIN);
  const remote = gitOk(ROOT, "ls-remote", "--exit-code", "--heads", "origin", x);
  if (remote) git("fetch", "-q", "origin", `${x}:refs/remotes/origin/${x}`);
  const from = remote ? `origin/${x}` : `origin/${MAIN}`;
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
  log(eeRemote ? `EE work goes in ee/ (on branch ${x}); \`just feat push\` when ready.` : "OSS only. `just feat ee` adds the EE side.");
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

// Push EE x (when it has commits), then the pointer commit and OSS x; open/update the PRs;
// post the pointer status and start the full build (what the bot does, so it works without it).
function cmdPush(args) {
  const draft = argFlag(args, "--draft");
  const title = argVal(args, "--title");
  const bodyArg = argVal(args, "--body");
  const bodyFile = argVal(args, "--body-file");
  const body = bodyFile ? readFileSync(bodyFile, "utf8") : bodyArg;
  const x = featBranch();
  git("fetch", "-q", "origin", MAIN);

  // --- EE side
  const ee = eeState();
  let eeTip = null;
  if (ee.ready) {
    eeFetch();
    if (ee.branch === x) {
      if (ee.dirty) die("ee/ has uncommitted changes: commit them (in ee/) first");
      const ahead = Number(gitIn(EE_DIR, "rev-list", "--count", `origin/${MAIN}..HEAD`));
      const hasRemote = gitOk(EE_DIR, "rev-parse", "--verify", "-q", `refs/remotes/origin/${x}`);
      if (ahead > 0 || hasRemote) {
        if (hasRemote && !gitOk(EE_DIR, "merge-base", "--is-ancestor", `origin/${x}`, "HEAD")) {
          // e.g. `feat ship` merged EE main into EE x on GitHub
          log(`EE ${x} moved on GitHub; merging it in`);
          if (!gitOk(EE_DIR, "merge", "-q", "--no-edit", `origin/${x}`)) die(`merging origin/${x} in ee/ conflicts: resolve, commit, push again`);
        }
        log(`pushing EE ${x} ...`);
        gitIn(EE_DIR, "push", "-q", "-u", "origin", `HEAD:refs/heads/${x}`);
        eeTip = gitIn(EE_DIR, "rev-parse", "HEAD");
      }
    } else if (ee.branch === null) {
      if (pointerAt("HEAD") !== ee.head) die(`ee/ is detached at ${short(ee.head)} with no branch: run \`just feat ee\` to put it on ${x}`);
    } else if (ee.branch === MAIN) {
      if (Number(gitIn(EE_DIR, "rev-list", "--count", `origin/${MAIN}..HEAD`)) > 0)
        die("ee/ has local commits on main: EE work goes on a branch named like the OSS one (`just feat ee`)");
    } else die(`ee/ is on branch ${ee.branch}, expected ${x} (or main): \`just feat ee\``);
  }

  // --- pointer
  if (eeTip && pointerAt("HEAD") !== eeTip) {
    const staged = git("diff", "--cached", "--name-only").split("\n").filter((f) => f && f !== "ee");
    if (staged.length) die(`staged changes besides ee/ (${staged.join(", ")}): commit them first`);
    gitIn(ROOT, "update-index", "--cacheinfo", `160000,${eeTip},ee`);
    git("commit", "-q", "-m", `ee: point at ${x} (${short(eeTip)})`);
    log(`pointer commit: ee -> ${short(eeTip)}`);
  }

  // --- OSS side
  const ossChanged = git("diff", "--name-only", `origin/${MAIN}...HEAD`, "--", ".", ":(exclude)ee") !== "";
  const ossRemote = gitOk(ROOT, "ls-remote", "--exit-code", "--heads", "origin", x);
  if (!ossChanged && !eeTip) die("nothing to push: no OSS changes and no EE commits on " + x);
  let ossPushed = false;
  if (ossChanged || ossRemote) {
    const ptr = pointerAt("HEAD");
    const basePtr = pointerAt(git("merge-base", `origin/${MAIN}`, "HEAD"));
    if (ptr && ptr !== basePtr && !eePushed(ptr)) die(`OSS ${x} points at EE ${short(ptr)}, which is not pushed`);
    pushBranch(ROOT, x);
    ossPushed = true;
  } else log(`EE-only change: OSS ${x} stays local (OSS main gets a pointer bump when the EE PR merges)`);

  // --- PRs
  let eePr = eeTip ? openPr(EE, x) : null;
  let ossPr = ossChanged ? openPr(OSS, x) : null;
  const subject = (cwd) => gitIn(cwd, "log", "-1", "--format=%s", "--no-merges");
  if (eeTip && !eePr) {
    const t = title ?? subject(EE_DIR);
    gh(EE, ["pr", "create", "-R", EE, "--head", x, "--base", MAIN, "--title", t, "--body", body ?? "", ...(draft ? ["--draft"] : [])]);
    eePr = openPr(EE, x);
    log(`EE PR: ${eePr.url}`);
  }
  if (ossChanged && !ossPr) {
    const t = title ?? subject(ROOT);
    gh(OSS, ["pr", "create", "-R", OSS, "--head", x, "--base", MAIN, "--title", t, "--body", body ?? "", ...(draft ? ["--draft"] : [])]);
    ossPr = openPr(OSS, x);
    log(`OSS PR: ${ossPr.url}`);
  }
  // The OSS PR is public: its pair note carries only the link (no titles or text from EE).
  if (eePr)
    setBody(EE, eePr, withPair(eePr.body, ossPr ? `Paired OSS PR: ${ossPr.url} (ship with \`just feat ship ${x}\`: this one merges first).` : "EE-only: no OSS PR. When this merges, OSS main gets an automatic `ee` pointer bump."));
  if (ossPr)
    setBody(OSS, ossPr, withPair(ossPr.body, eePr ? `Paired EE PR: ${eePr.url} (private). The \`ee\` pointer follows its branch; ship with \`just feat ship ${x}\`.` : "Leaves `ee` untouched."));
  if (ossPr) {
    // What the bot would do, done right away (and without it): pointer status + full build.
    sync({ only: String(ossPr.number), build: !ossPr.isDraft, quiet: true, main: true });
  }
  triggerSync();
  log(`done: ${[eePr && `EE ${eePr.url}`, ossPr && `OSS ${ossPr.url}`, ossPushed && !ossPr && `OSS ${x} pushed`].filter(Boolean).join("  ")}`);
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
  git("fetch", "-q", "origin", MAIN);
  const ee = eeState();
  let want = null;
  if (ee.ready && ee.branch === x) {
    if (ee.dirty) die("ee/ has uncommitted changes; commit them first");
    eeFetch();
    if (gitOk(EE_DIR, "rev-parse", "--verify", "-q", `refs/remotes/origin/${x}`) && !gitOk(EE_DIR, "merge", "-q", "--no-edit", `origin/${x}`))
      die(`EE merge of origin/${x} has conflicts: resolve them in ee/, commit, then re-run`);
    if (!gitOk(EE_DIR, "merge", "-q", "--no-edit", `origin/${MAIN}`)) die("EE merge of main has conflicts: resolve them in ee/, commit, then re-run");
    want = gitIn(EE_DIR, "rev-parse", "HEAD");
  }
  mergeInto(ROOT, `origin/${MAIN}`, want);
  // OSS-only branch with ee/ initialised: keep ee/ on the (possibly bumped) pointer.
  if (ee.ready && ee.branch !== x && !ee.dirty) {
    const p = pointerAt("HEAD");
    if (p && p !== ee.head) {
      eeFetch();
      gitIn(EE_DIR, "checkout", "-q", "--detach", p);
    }
  }
  log(`updated ${x} with main${want ? ` (ee -> ${short(want)})` : ""}; \`just feat push\` to publish`);
}

// ---------------------------------------------------------------------------------- checks

function checksOf(repo, pr, required) {
  const r = gh(repo, ["pr", "checks", String(pr), "-R", repo, "--json", "name,bucket", ...(required ? ["--required"] : [])], { allowFail: true, full: true });
  if (!r.stdout.trim()) return [];
  return JSON.parse(r.stdout);
}
const headOf = (repo, n) => JSON.parse(gh(repo, ["pr", "view", String(n), "-R", repo, "--json", "headRefOid"])).headRefOid;

// Wait until every check of PR `n` (only the required ones with `required`) passes, on head
// `sha`. Checks need a moment to register after a push: none at all counts as pending for a
// while.
async function waitChecks(repo, n, sha, { required = false, timeoutMin, label, lock }) {
  const start = Date.now();
  let told = "";
  for (;;) {
    lock?.beat();
    const head = headOf(repo, n);
    if (head !== sha) return { ok: false, moved: true, why: `head moved to ${short(head)}` };
    const cs = checksOf(repo, n, required);
    const bad = cs.filter((c) => c.bucket === "fail" || c.bucket === "cancel").map((c) => c.name);
    if (bad.length) return { ok: false, why: `failing: ${bad.join(", ")}` };
    const pending = cs.filter((c) => c.bucket === "pending").map((c) => c.name);
    if (!pending.length && (cs.length || Date.now() - start > 120000)) return { ok: true };
    if (Date.now() - start > timeoutMin * 60000) return { ok: false, why: `still pending after ${timeoutMin} min: ${pending.join(", ")}` };
    const now = pending.join(", ") || "(none registered yet)";
    if (now !== told) log(`waiting for ${label} checks: ${now}`);
    told = now;
    await sleep(20000);
  }
}

// Re-run the failed jobs of the latest EE CI run of `sha` (once per ship round).
function rerunEeCi(sha) {
  const r = gh(EE, ["run", "list", "-R", EE, "-w", "ee-ci.yml", "--commit", sha, "-L", "5", "--json", "databaseId,status,conclusion,attempt"], { allowFail: true, full: true });
  const run = r.status === 0 ? JSON.parse(r.stdout || "[]")[0] : null;
  if (!run || run.status !== "completed" || run.conclusion === "success" || run.attempt > 2) return false;
  log(`re-running the failed EE CI jobs (run ${run.databaseId}) against the current OSS branch`);
  return gh(EE, ["run", "rerun", String(run.databaseId), "-R", EE, "--failed"], { allowFail: true, full: true }).status === 0;
}

// The run of oss-pr.yml that builds `sha` (its run-name carries the sha).
function buildRun(sha) {
  const out = gh(EE, ["run", "list", "-R", EE, "-w", BUILD_WORKFLOW, "-L", "40", "--json", "databaseId,displayTitle,status,conclusion,url,createdAt"], { allowFail: true, full: true });
  if (out.status !== 0) return null;
  return JSON.parse(out.stdout || "[]").find((r) => r.displayTitle.includes(sha)) ?? null;
}
function startBuild(n, sha) {
  const r = gh(EE, ["workflow", "run", BUILD_WORKFLOW, "-R", EE, "--ref", "main", "-f", `pr=${n}`, "-f", `sha=${sha}`], { allowFail: true, full: true });
  if (r.status !== 0) {
    log(`  could not start the full build: ${r.stderr.trim()}`);
    return false;
  }
  postStatus(sha, BUILD_CTX, "pending", "full build queued");
  return true;
}
// The `ee/build` verdict for OSS PR `n` at `sha`: starts the build when it never ran, and turns
// a finished run into the status itself when the bot could not (no token on the EE side).
async function awaitBuild(n, sha, { timeoutMin, lock }) {
  const start = Date.now();
  let told = false;
  for (;;) {
    lock?.beat();
    const st = statusesOf(sha).get(BUILD_CTX);
    if (st?.state === "success") return { ok: true };
    if (st && st.state !== "pending") return { ok: false, why: `ee/build ${st.state}: ${st.description}` };
    const r = buildRun(sha);
    if (!st && !r) {
      log(`starting the full build of OSS #${n} at ${short(sha)}`);
      if (!startBuild(n, sha)) return { ok: false, why: "could not start the full build" };
    } else if (r?.status === "completed") {
      const ok = r.conclusion === "success";
      postStatus(sha, BUILD_CTX, ok ? "success" : "failure", ok ? "full product builds and passes its tests" : `full build: ${r.conclusion} (private log)`, r.url);
      if (!ok) return { ok: false, why: `ee/build ${r.conclusion}` };
      return { ok: true };
    } else if (!r && Date.now() - Date.parse(st.updated_at) > 10 * 60000) {
      log("ee/build is pending but no build run exists for it; starting one");
      startBuild(n, sha);
    }
    if (Date.now() - start > timeoutMin * 60000) return { ok: false, why: `ee/build still pending after ${timeoutMin} min` };
    if (!told) log(`waiting for the full build of OSS #${n} (${r?.url ?? "queued"}) ...`);
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
  const done = () => git("worktree", "remove", "--force", dir);
  try {
    const out = fn(dir);
    if (out instanceof Promise) return out.finally(done);
    done();
    return out;
  } catch (e) {
    done();
    throw e;
  }
}
// Merge OSS main into OSS x on GitHub (via a temp worktree), with the gitlink at `want` (or
// left as merged when null). Returns the new head (unchanged when already up to date).
function updateOssBranch(x, want) {
  git("fetch", "-q", "origin", `+refs/heads/${MAIN}:refs/remotes/origin/${MAIN}`, `+refs/heads/${x}:refs/remotes/origin/${x}`);
  return withTempWorktree(`origin/${x}`, (dir) => {
    const before = gitIn(dir, "rev-parse", "HEAD");
    mergeInto(dir, `origin/${MAIN}`, want, x);
    if (gitIn(dir, "rev-parse", "HEAD") === before) return before;
    pushBranch(dir, x);
    return gitIn(dir, "rev-parse", "HEAD");
  });
}
// Merge EE main into EE x on GitHub (server-side merge; no checkout needed).
function updateEeBranch(x) {
  try {
    const r = api(EE, "merges", { method: "POST", body: { base: x, head: MAIN, commit_message: `Merge EE ${MAIN} into ${x}` } });
    if (r.sha) log(`merged EE main into EE ${x} (${short(r.sha)})`);
  } catch (e) {
    if (e instanceof ApiError && e.status === 409) die(`EE ${x} conflicts with EE main: \`just feat update\`, resolve in ee/, \`just feat push\`, re-run`);
    throw e;
  }
}

// ---------------------------------------------------------------------------------- ship

async function cmdShip(args) {
  const admin = argFlag(args, "--admin");
  const force = argFlag(args, "--force");
  const takeover = argFlag(args, "--takeover");
  const noBuild = argFlag(args, "--no-build");
  const timeoutMin = Number(argVal(args, "--timeout") ?? 60);
  const x = args[0] ?? featBranch();
  if (x === MAIN) die("ship a feature branch, not main");
  {
    const o = openPr(OSS, x);
    const e = openPr(EE, x);
    if (!o && !e) die(`no open PR for ${x} in ${OSS} or ${EE}`);
    log(`shipping ${x}: ${[e && `EE #${e.number}`, o && `OSS #${o.number}`].filter(Boolean).join(" + ")}`);
  }
  const gate = (c, what) => {
    if (c.ok) return;
    if (force) return log(`! ${what} ${c.why} (--force: shipping anyway)`);
    die(`${what} ${c.why}${c.moved ? "" : " (--force to ship anyway)"}`);
  };

  const lock = await Lock.acquire(x, { takeover });
  let keepLock = false;
  try {
    // 1. Bring both sides up to date and wait for every check on the result; repeat if
    //    anything moved meanwhile. Nothing is merged yet, so a failure here changes nothing.
    let ossPr, eePr, eeMergedBefore, ossHead, eeHead;
    for (let round = 1; ; round++) {
      if (round > 5) die("main kept moving while waiting for checks; re-run later");
      lock.beat(true);
      ossPr = openPr(OSS, x);
      eePr = openPr(EE, x);
      eeMergedBefore = !eePr && !!mergedPr(EE, x);
      if (eePr?.isDraft || ossPr?.isDraft) die("a PR is still a draft (gh pr ready)");
      if (eePr) {
        if (!apiContains(EE, eePr.headRefOid, MAIN)) updateEeBranch(x);
        eeHead = refSha(EE, x);
      }
      if (ossPr) {
        const want = eePr ? eeHead : eeMergedBefore ? refSha(EE, MAIN) : null;
        ossHead = updateOssBranch(x, want);
        if (ossHead !== ossPr.headRefOid) log(`OSS ${x} updated (main merged in, ee -> ${short(apiPointer(ossHead))})`);
        if (!want && apiPointer(ossHead) !== apiPointer(MAIN)) die(`OSS ${x} changes the ee pointer but has no open EE PR ${x}`);
        sync({ only: String(ossPr.number), build: false, quiet: true });
      }
      let moved = false;
      if (eePr) {
        let c = await waitChecks(EE, eePr.number, eeHead, { timeoutMin, label: `EE #${eePr.number}`, lock });
        // EE CI builds against whatever OSS x was when it ran; OSS x may have moved since.
        if (!c.ok && !c.moved && ossPr && rerunEeCi(eeHead)) {
          await sleep(20000);
          c = await waitChecks(EE, eePr.number, eeHead, { timeoutMin, label: `EE #${eePr.number} (re-run)`, lock });
        }
        if (c.moved) moved = true;
        else gate(c, `EE PR #${eePr.number} checks`);
      }
      if (ossPr && !moved) {
        if (noBuild) log(`! not waiting for ee/build (--no-build)`);
        else gate(await awaitBuild(ossPr.number, ossHead, { timeoutMin, lock }), `OSS PR #${ossPr.number}`);
        const c = await waitChecks(OSS, ossPr.number, ossHead, { timeoutMin, label: `OSS #${ossPr.number}`, lock });
        if (c.moved) moved = true;
        else gate(c, `OSS PR #${ossPr.number} checks`);
      }
      // Still current? (an owner merge on GitHub can move either main at any time)
      if (!moved && eePr && (refSha(EE, x) !== eeHead || !apiContains(EE, eeHead, MAIN))) moved = true;
      if (!moved && ossPr && (refSha(OSS, x) !== ossHead || !apiContains(OSS, ossHead, MAIN))) moved = true;
      if (!moved) break;
      log("something moved while waiting; bringing the branches up to date again");
    }

    // 2. EE first.
    let eeMain = refSha(EE, MAIN);
    let sameTree = true;
    if (eePr) {
      log(`merging EE PR #${eePr.number} ...`);
      gh(EE, ["pr", "merge", String(eePr.number), "-R", EE, "--squash", "--match-head-commit", eeHead]);
      keepLock = !!ossPr; // EE is merged: if OSS fails now, keep main's pointer on hold
      try {
        api(EE, `git/refs/heads/${x}`, { method: "DELETE" });
      } catch {
        /* already gone */
      }
      eeMain = refSha(EE, MAIN);
      sameTree = treeOf(EE, eeMain) === treeOf(EE, eeHead);
      log(`EE main is now ${short(eeMain)}${sameTree ? "" : " (its tree differs from the tested EE branch: EE main moved meanwhile)"}`);
    }
    if (!ossPr) {
      // EE-only: OSS main follows EE main right away.
      const r = apiSetPointer(MAIN, eeMain, `ee: bump to EE main (${short(eeMain)})`);
      log(r.changed ? `OSS main ee -> ${short(eeMain)}` : "OSS main already points at EE main");
      log(`shipped ${x}`);
      return;
    }

    // 3. OSS x -> EE main. Only the gitlink changes, and (normally) EE main has exactly the
    //    tree that was built and tested, so the checked head's ee/ statuses carry over.
    for (let attempt = 0; ; attempt++) {
      lock.beat(true);
      let head = ossHead;
      if (eePr || eeMergedBefore) {
        const r = apiSetPointer(x, eeMain, `ee: point at EE main (${short(eeMain)})`);
        head = r.head;
        if (r.changed) {
          log(`OSS ${x} ee -> EE main ${short(eeMain)}`);
          const prev = statusesOf(ossHead);
          for (const ctx of [BUILD_CTX, POINTER_CTX]) {
            const s = prev.get(ctx);
            if (sameTree && s?.state === "success") postStatus(head, ctx, "success", `${s.description} (same trees as ${short(ossHead)})`, s.target_url);
          }
          if (!prev.get(POINTER_CTX)) postStatus(head, POINTER_CTX, "success", "EE side merged: follows EE main");
          if (!sameTree && !noBuild) gate(await awaitBuild(ossPr.number, head, { timeoutMin, lock }), `OSS PR #${ossPr.number}`);
        }
      }
      if (!admin) gate(await waitChecks(OSS, ossPr.number, head, { required: true, timeoutMin, label: `required OSS #${ossPr.number}`, lock }), `OSS PR #${ossPr.number} required checks`);
      const r = gh(OSS, ["pr", "merge", String(ossPr.number), "-R", OSS, "--squash", "--match-head-commit", head, ...(admin ? ["--admin"] : [])], { allowFail: true, full: true });
      if (r.status === 0) break;
      const state = await prField(OSS, ossPr.number, "mergeStateStatus");
      if (attempt >= 4) die(`merging OSS PR #${ossPr.number} failed (${state}): ${r.stderr.trim()}`);
      if (state !== "BEHIND" && state !== "DIRTY") {
        log(`merging OSS PR #${ossPr.number} failed (${state}: ${r.stderr.trim()}); retrying`);
        await sleep(30000);
        continue;
      }
      // Main moved (an owner merge): merge it in again and re-check everything on the result.
      log(`OSS main moved (${state}); merging it in again`);
      eeMain = refSha(EE, MAIN);
      ossHead = updateOssBranch(x, eePr || eeMergedBefore ? eeMain : null);
      sameTree = false;
      if (!noBuild) gate(await awaitBuild(ossPr.number, ossHead, { timeoutMin, lock }), `OSS PR #${ossPr.number}`);
      gate(await waitChecks(OSS, ossPr.number, ossHead, { timeoutMin, label: `OSS #${ossPr.number}`, lock }), `OSS PR #${ossPr.number} checks`);
    }
    keepLock = false;
    try {
      api(OSS, `git/refs/heads/${x}`, { method: "DELETE" });
    } catch {
      /* already gone */
    }
    log(`merged OSS PR #${ossPr.number}`);
    // 4. main must end at EE main
    const tip = refSha(EE, MAIN);
    if (apiPointer(MAIN) !== tip) {
      apiSetPointer(MAIN, tip, `ee: bump to EE main (${short(tip)})`);
      log(`OSS main ee -> ${short(tip)}`);
    }
    log(`shipped ${x}`);
  } finally {
    if (keepLock) log(`keeping the ship lock for ${x}: fix the cause, then re-run \`just feat ship ${x} --takeover\``);
    else lock.release();
    triggerSync();
  }
}

// ---------------------------------------------------------------------------------- sync

// Reconcile the invariants (what the EE-side bot runs; anyone with write access to both repos
// can run it): OSS main -> EE main (unless a ship holds the lock); every open OSS PR's pointer ->
// its EE branch tip (or EE main once the EE PR merged), with a gitlink-only conflict against
// main resolved by merging main in; the ee/pointer status on each OSS PR head; with `build`,
// the full build started for heads that have none.
function sync({ dry = false, ci = false, only = null, build = false, quiet = false, main = !only } = {}) {
  const say = quiet ? () => {} : log;
  const write = (what, fn) => {
    if (dry) return say(`  [dry-run] would ${what}`);
    try {
      return fn();
    } catch (e) {
      if (e instanceof ApiError && (e.status === 403 || e.status === 404)) {
        log(`${ci ? "::warning::" : ""}no write access for: ${what} (HTTP ${e.status}; see docs/EE-WORKFLOW.md)`);
        return null;
      }
      if (e instanceof Fail) {
        log(`${ci ? "::warning::" : ""}could not ${what}: ${e.message.split("\n")[0]}`);
        return null;
      }
      throw e;
    }
  };
  const eeMain = refSha(EE, MAIN);
  const lock = readLock();
  const fresh = lockFresh(lock);
  say(`EE main ${short(eeMain)}${lock ? `; ship lock held (${lock.ageMin.toFixed(0)} min since renewal${fresh ? "" : ", stale"})` : ""}`);

  if (main) {
    const ossMainPtr = apiPointer(MAIN);
    if (ossMainPtr === null) say("OSS main has no ee/ gitlink yet: nothing to bump");
    else if (ossMainPtr === eeMain) say("OSS main points at EE main");
    else if (fresh) say(`OSS main points at ${short(ossMainPtr)}; a ship holds the lock, leaving it`);
    else {
      say(`OSS main points at ${short(ossMainPtr)} -> bumping to ${short(eeMain)}`);
      write(`bump OSS main ee -> ${short(eeMain)}`, () => apiSetPointer(MAIN, eeMain, `ee: bump to EE main (${short(eeMain)})`));
    }
  }

  const list = JSON.parse(
    gh(OSS, ["pr", "list", "-R", OSS, "--state", "open", "-L", "100", "--json", "number,headRefName,headRefOid,baseRefName,isCrossRepository,isDraft,mergeable"]) || "[]",
  ).filter((p) => !only || String(p.number) === String(only));
  const eePrs = JSON.parse(gh(EE, ["pr", "list", "-R", EE, "--state", "all", "-L", "200", "--json", "headRefName,state"]) || "[]");
  const eeOpenHeads = new Set(eePrs.filter((p) => p.state === "OPEN").map((p) => p.headRefName));
  const eeMergedHeads = new Set(eePrs.filter((p) => p.state === "MERGED").map((p) => p.headRefName));
  const eeBranches = (api(EE, "git/matching-refs/heads/", { paginate: true }) ?? []).map((r) => [r.ref.slice("refs/heads/".length), r.object.sha]);
  const eeTips = new Map(eeBranches);
  for (const pr of list) {
    const x = pr.headRefName;
    let head = pr.headRefOid;
    const ptr = apiPointer(head);
    if (ptr === null) continue; // a branch from before the ee/ submodule existed
    const base = api(OSS, `compare/${pr.baseRefName}...${head}`).merge_base_commit.sha;
    const touched = ptr !== apiPointer(base);
    if (pr.isCrossRepository) {
      // A fork can't have an EE branch: it must leave ee alone. Never built automatically.
      const state = touched ? "failure" : "success";
      say(`#${pr.number} (fork) ${state}`);
      write(`status #${pr.number}`, () => postStatus(head, POINTER_CTX, state, touched ? "forks cannot change the ee pointer" : "leaves ee untouched"));
      continue;
    }
    const shipping = fresh && lock.branch === x;
    let want = null;
    let why = "";
    const eeTip = eeTips.get(x) ?? null;
    if (!eeOpenHeads.has(x) && eeMergedHeads.has(x)) [want, why] = [eeMain, "EE side merged: follows EE main"];
    else if (eeTip) [want, why] = [eeTip, "points at the tip of the paired EE branch"];
    let now = ptr;
    if (shipping) say(`#${pr.number} ${x}: being shipped, leaving it`);
    else if (want && pr.mergeable === "CONFLICTING") {
      // Usually main's pointer moved: a gitlink-only conflict. Merge main in, ee -> want.
      say(`#${pr.number} ${x}: conflicts with main; merging main in (ee -> ${short(want)})`);
      const h = write(`merge main into OSS ${x}`, () => updateOssBranch(x, want));
      if (h) [head, now] = [h, want];
    }
    if (!shipping && want && now !== want) {
      say(`#${pr.number} ${x}: ee ${short(ptr)} -> ${short(want)}`);
      const r = write(`point OSS ${x} at ${short(want)}`, () => apiSetPointer(x, want, `ee: point at ${x} (${short(want)})`));
      if (r?.changed) [head, now] = [r.head, want];
    }
    if (shipping) continue;
    let state, desc;
    if (want && now === want) [state, desc] = ["success", why];
    else if (want) [state, desc] = ["pending", "pointer is behind the paired EE branch; it is being updated"];
    else if (!touched) [state, desc] = ["success", "leaves ee untouched"];
    else [state, desc] = ["failure", "points at an EE commit that is not the tip of a paired EE branch (just feat push)"];
    say(`#${pr.number} ${x}: ${state} (${desc})`);
    const prev = statusesOf(head);
    if (prev.get(POINTER_CTX)?.state !== state || prev.get(POINTER_CTX)?.description !== desc)
      write(`status #${pr.number}`, () => postStatus(head, POINTER_CTX, state, desc));
    if (build && !pr.isDraft && !prev.has(BUILD_CTX) && !buildRun(head)) {
      say(`#${pr.number}: starting the full build`);
      write(`start the full build of #${pr.number}`, () => startBuild(pr.number, head));
    }
  }
  if (only) return;
  // OSS branches without a PR still follow their EE twin (invariant 3).
  const withPr = new Set(list.map((p) => p.headRefName));
  for (const [x, tip] of eeBranches) {
    if (x === MAIN || withPr.has(x) || (fresh && lock.branch === x)) continue;
    if (!refSha(OSS, x)) continue;
    const ptr = apiPointer(x);
    if (ptr === null || ptr === tip) continue;
    say(`OSS ${x} (no PR): ee ${short(ptr)} -> ${short(tip)}`);
    write(`point OSS ${x} at ${short(tip)}`, () => apiSetPointer(x, tip, `ee: point at ${x} (${short(tip)})`));
  }
}

function cmdSync(args) {
  const dry = argFlag(args, "--dry-run");
  const ci = argFlag(args, "--ci");
  const only = argVal(args, "--pr") ?? null;
  sync({ dry, ci, only, build: ci || argFlag(args, "--build") });
}
function cmdCheck(args) {
  const n = args[0] ?? die("usage: feat check <oss-pr-number>");
  sync({ dry: true, only: n });
}

// ---------------------------------------------------------------------------------- status

function cmdStatus() {
  const x = currentBranch();
  git("fetch", "-q", "origin", MAIN);
  const ee = eeState();
  if (ee.ready) eeFetch();
  const ptr = pointerAt("HEAD");
  const row = (k, v) => log(`  ${k.padEnd(18)} ${v}`);
  log(`OSS ${OSS} (EE: ${EE})`);
  row("branch", x ?? "(detached)");
  row("ee pointer", short(ptr));
  row("OSS main pointer", short(pointerAt(`origin/${MAIN}`)));
  if (ee.ready) {
    row("ee/ HEAD", `${short(ee.head)} on ${ee.branch ?? "DETACHED"}${ee.dirty ? ", uncommitted changes" : ""}`);
    row("EE main", short(gitTry(EE_DIR, "rev-parse", `origin/${MAIN}`)));
    if (x) row(`EE ${x}`, short(gitTry(EE_DIR, "rev-parse", "-q", "--verify", `refs/remotes/origin/${x}`)));
  } else row("ee/", "not initialised (`just feat ee`)");
  const lock = readLock();
  if (lock) row("ship lock", `${lock.branch} (${lock.holder}, ${lock.ageMin.toFixed(0)} min since renewal${lockFresh(lock) ? "" : ", stale"})`);
  if (x && x !== MAIN) {
    const o = openPr(OSS, x);
    const e = openPr(EE, x);
    row("OSS PR", o ? `${o.url}${o.isDraft ? " (draft)" : ""}` : "-");
    row("EE PR", e ? e.url : "-");
    for (const [repo, pr, label] of [[OSS, o, "OSS"], [EE, e, "EE"]]) {
      if (!pr) continue;
      const cs = checksOf(repo, pr.number, false);
      const by = (b) => cs.filter((c) => c.bucket === b).length;
      const failing = cs.filter((c) => c.bucket === "fail" || c.bucket === "cancel").map((c) => c.name);
      row(`${label} checks`, `${by("pass")} pass, ${failing.length} fail${failing.length ? ` (${failing.join(", ")})` : ""}, ${by("pending")} pending`);
    }
    if (o) {
      const st = statusesOf(o.headRefOid);
      for (const ctx of [POINTER_CTX, BUILD_CTX]) row(ctx, st.get(ctx) ? `${st.get(ctx).state}: ${st.get(ctx).description}` : "-");
    }
    const warn = [];
    if (ee.ready && ee.branch === null && ee.head !== ptr) warn.push("ee/ is detached away from the pointer: `just feat ee` (EE work) or `just feat update`");
    if (ee.ready && ee.branch === x && ee.head !== ptr) warn.push("ee/ HEAD differs from the pointer: `just feat push`");
    const eeRemote = ee.ready && x ? gitTry(EE_DIR, "rev-parse", "-q", "--verify", `refs/remotes/origin/${x}`) : null;
    if (eeRemote && ee.head !== eeRemote && ee.branch === x) warn.push(`EE ${x} differs from origin (unpushed commits?)`);
    for (const w of warn) log(`  ! ${w}`);
    if (!warn.length) log("  ok");
  }
}

// ---------------------------------------------------------------------------------- hooks

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
      if (p === pointerAt(`${c}^`)) continue;
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
