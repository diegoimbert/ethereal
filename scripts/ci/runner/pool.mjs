#!/usr/bin/env node
// The devbox runner pool (docs/CI-RUNNERS.md): keeps one ephemeral GitHub Actions runner per slot,
// each in a fresh sandboxed systemd user unit that runs one job and exits.
//
//   node scripts/ci/runner/pool.mjs setup     install the runner, toolchain, sccache, slice (idempotent)
//   node scripts/ci/runner/pool.mjs run       supervise the slots (the systemd unit's command; runs setup)
//   node scripts/ci/runner/pool.mjs status    slots, runners, disk
//   node scripts/ci/runner/pool.mjs stop      remove the idle runners (busy ones finish their job)
//
// Settings (environment, e.g. from ~/.config/ethereal-ci/env):
//   CI_POOL_HOME          the pool's own directory                 (~/ci-pool)
//   CI_POOL_REPO          repository the runners serve             (diegoimbert/ethereal)
//   CI_POOL_SLOTS         jobs at once                             (3)
//   CI_POOL_CPUS          CPUs for the whole pool (slice quota)    (8)
//   CI_POOL_MEMORY_GB     memory for the whole pool (slice)        (20)
//   CI_POOL_LABELS        runner labels, comma-separated           (devbox-pool)
//   CI_POOL_MIN_FREE_GB   no new job below this much free disk     (100)
//   CI_POOL_TARGET_MAX_GB a cargo target dir is emptied above      (60)
//   CI_POOL_TARGET_DAYS   a target dir unused this long is deleted (7)
//   GH_TOKEN              a token that may administer the repository's runners (else `gh auth token`)
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statfsSync, statSync, symlinkSync, writeFileSync } from "node:fs";
import { homedir, hostname, userInfo } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..", "..");
const env = process.env;
const home = env.CI_POOL_HOME || join(homedir(), "ci-pool");
const cfg = {
  repo: env.CI_POOL_REPO || "diegoimbert/ethereal",
  slots: Number(env.CI_POOL_SLOTS || 3),
  cpus: Number(env.CI_POOL_CPUS || 8),
  memoryGb: Number(env.CI_POOL_MEMORY_GB || 20),
  labels: (env.CI_POOL_LABELS || "devbox-pool")
    .split(",")
    .map((s) => s.trim())
    .filter(Boolean),
  minFreeGb: Number(env.CI_POOL_MIN_FREE_GB || 100),
  targetMaxGb: Number(env.CI_POOL_TARGET_MAX_GB || 60),
  targetDays: Number(env.CI_POOL_TARGET_DAYS || 7),
};
// Pinned downloads (bump version and sha256 together).
const RUNNER = { version: "2.338.0", sha256: "af4b794c1bc41d73d40535e3fe092a39f9679cd8d965954c2aca25a05ca41d32" };
const RUSTUP = { version: "1.29.0", sha256: "4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10" };
const SCCACHE = { version: "0.18.0", sha256: "45f1447fbe231e3037bde351ef70677dd212216c8d62ae7ca409fecc4d6acc89" };

const SLICE = "ethereal-ci-pool.slice";
// Runner names: `pool-<host>-<slot>-<time>`, so each machine only manages its own.
const NAME = `pool-${hostname()}-`;
const slotOf = (runnerName) => runnerName.slice(NAME.length).split("-")[0];
const unitOf = (slot) => `ethereal-ci-slot-${slot}.service`;
const p = (...a) => join(home, ...a);
const log = (...a) => console.log(new Date().toISOString(), ...a);

function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: "utf8", ...opts });
  if (r.status !== 0 && !opts.allowFail) throw new Error(`${cmd} ${args.slice(0, 3).join(" ")}: ${(r.stderr || r.stdout || "").trim()}`);
  return r;
}
const systemctl = (...a) => sh("systemctl", ["--user", ...a], { allowFail: true });

let token;
function ghToken() {
  token ??= env.GH_TOKEN || execFileSync("gh", ["auth", "token"], { encoding: "utf8" }).trim();
  return token;
}
async function api(method, path, body) {
  const r = await fetch(`https://api.github.com/${path}`, {
    method,
    headers: { authorization: `Bearer ${ghToken()}`, accept: "application/vnd.github+json", "x-github-api-version": "2022-11-28" },
    body: body && JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${(await r.text()).slice(0, 300)}`);
  return r.status === 204 ? null : r.json();
}

// ------------------------------------------------------------------------------------- setup

async function download(url, sha256, dest) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`GET ${url}: ${r.status}`);
  const buf = Buffer.from(await r.arrayBuffer());
  const got = createHash("sha256").update(buf).digest("hex");
  if (got !== sha256) throw new Error(`${url}: sha256 ${got}, expected ${sha256}`);
  writeFileSync(dest, buf);
}

const toolEnv = () => ({ ...env, RUSTUP_HOME: p("rustup"), CARGO_HOME: p("cargo"), PATH: `${p("cargo", "bin")}:${env.PATH}` });

async function setup() {
  for (const d of ["bin", "cache/sccache", "cache/pnpm-store", "cache/ms-playwright", "cache/corepack", "targets", "slots", "job", "dl"])
    mkdirSync(p(d), { recursive: true });
  const runnerDir = p(`runner-${RUNNER.version}`);
  if (!existsSync(join(runnerDir, "run.sh"))) {
    log(`installing the Actions runner ${RUNNER.version}`);
    const tgz = p("dl", "runner.tgz");
    await download(
      `https://github.com/actions/runner/releases/download/v${RUNNER.version}/actions-runner-linux-x64-${RUNNER.version}.tar.gz`,
      RUNNER.sha256,
      tgz,
    );
    mkdirSync(runnerDir, { recursive: true });
    sh("tar", ["-xzf", tgz, "-C", runnerDir]);
    rmSync(tgz);
  }
  // The runner's own Node runs the job-started hook (and is the `node` of steps without setup-node).
  const nodes = readdirSync(join(runnerDir, "externals")).filter((d) => /^node\d+$/.test(d));
  const node = join(runnerDir, "externals", nodes.sort((a, b) => Number(b.slice(4)) - Number(a.slice(4)))[0], "bin", "node");
  rmSync(p("bin", "node"), { force: true });
  symlinkSync(node, p("bin", "node"));
  if (!existsSync(p("bin", "sccache"))) {
    log(`installing sccache ${SCCACHE.version}`);
    const tgz = p("dl", "sccache.tgz");
    await download(
      `https://github.com/mozilla/sccache/releases/download/v${SCCACHE.version}/sccache-v${SCCACHE.version}-x86_64-unknown-linux-musl.tar.gz`,
      SCCACHE.sha256,
      tgz,
    );
    sh("tar", ["-xzf", tgz, "-C", p("bin"), "--strip-components=1", "--wildcards", "*/sccache"]);
    rmSync(tgz);
  }
  if (!existsSync(p("cargo", "bin", "rustup"))) {
    log(`installing rustup ${RUSTUP.version}`);
    const init = p("dl", "rustup-init");
    await download(`https://static.rust-lang.org/rustup/archive/${RUSTUP.version}/x86_64-unknown-linux-gnu/rustup-init`, RUSTUP.sha256, init);
    chmodSync(init, 0o755);
    sh(init, ["-y", "--no-modify-path", "--profile", "minimal", "--default-toolchain", "none"], { env: toolEnv(), stdio: "inherit" });
    rmSync(init);
  }
  // Pinned: no self-update behind our back.
  sh("rustup", ["set", "auto-self-update", "disable"], { env: toolEnv() });
  // ubuntu-latest has `zip` (the web release zip); this machine may not: a stand-in on Python's.
  if (sh("sh", ["-c", "command -v zip"], { allowFail: true }).status !== 0) {
    writeFileSync(
      p("bin", "zip"),
      `#!/bin/sh
# Stand-in for \`zip [-q] [-r] out.zip paths...\` (pool.mjs setup; the machine has no zip).
while [ "\${1#-}" != "$1" ]; do shift; done
out="$1"; shift
exec python3 -m zipfile -c "$out" "$@"
`,
      { mode: 0o755 },
    );
  }
  // The toolchain of rust-toolchain.toml, before any job: concurrent jobs must not race to install it.
  sh("rustup", ["toolchain", "install"], { cwd: root, env: toolEnv(), stdio: "inherit" });
  // The pool's share of the CI budget (its parent, ethereal-ci.slice, caps all CI together).
  const unitDir = join(homedir(), ".config", "systemd", "user");
  mkdirSync(unitDir, { recursive: true });
  const slice = `# Generated by scripts/ci/runner/pool.mjs setup: the devbox runner pool's jobs.
[Unit]
Description=Ethereal CI: devbox runner pool (${cfg.cpus} CPUs, ${cfg.memoryGb} GB)

[Slice]
CPUQuota=${cfg.cpus * 100}%
CPUWeight=50
IOWeight=50
MemoryHigh=${cfg.memoryGb}G
MemoryMax=${Math.round(cfg.memoryGb * 1.25)}G
`;
  const slicePath = join(unitDir, SLICE);
  if (!existsSync(slicePath) || readFileSync(slicePath, "utf8") !== slice) {
    writeFileSync(slicePath, slice);
    systemctl("daemon-reload");
    // Applies to a running slice too (never restart a slice: that stops every unit in it).
    systemctl("set-property", SLICE, `CPUQuota=${cfg.cpus * 100}%`, `MemoryHigh=${cfg.memoryGb}G`, `MemoryMax=${Math.round(cfg.memoryGb * 1.25)}G`);
  }
}

// ---------------------------------------------------------------------------------- the slots

function freeGb() {
  const s = statfsSync(home);
  return (s.bavail * s.bsize) / 1e9;
}

function prepareSlot(slot) {
  const dir = p("slots", String(slot));
  const marker = join(dir, "runner", `.version-${RUNNER.version}`);
  if (!existsSync(marker)) {
    rmSync(join(dir, "runner"), { recursive: true, force: true });
    mkdirSync(dir, { recursive: true });
    sh("cp", ["-a", p(`runner-${RUNNER.version}`), join(dir, "runner")]);
    writeFileSync(marker, "");
  }
  // A runner registration is per job: the previous job's files would make run.sh refuse the next.
  for (const f of [".runner", ".credentials", ".credentials_rsaparams"]) rmSync(join(dir, "runner", f), { force: true });
  mkdirSync(join(dir, "home"), { recursive: true });
  return dir;
}

async function start(slot) {
  const dir = prepareSlot(slot);
  const name = `${NAME}${slot}-${Date.now().toString(36)}`;
  const jit = await api("POST", `repos/${cfg.repo}/actions/runners/generate-jitconfig`, {
    name,
    runner_group_id: 1,
    // Just-in-time runners get only the labels asked for: add the defaults of a configured runner.
    labels: ["self-hosted", "Linux", "X64", ...cfg.labels],
    work_folder: "_work",
  });
  writeFileSync(join(dir, "jit"), jit.encoded_jit_config, { mode: 0o600 });
  const job = p("job");
  const vars = {
    HOME: join(job, "home"),
    USER: userInfo().username,
    LANG: "C.UTF-8",
    PATH: `${p("bin")}:${p("cargo", "bin")}:/usr/local/bin:/usr/bin:/bin`,
    RUSTUP_HOME: p("rustup"),
    CARGO_HOME: p("cargo"),
    CARGO_TARGET_DIR: join(job, "cargo-target"),
    CARGO_INCREMENTAL: "0",
    CARGO_BUILD_JOBS: String(Math.max(2, Math.round((cfg.cpus / cfg.slots) * 1.5))),
    RUSTC_WRAPPER: p("bin", "sccache"),
    SCCACHE_DIR: p("cache", "sccache"),
    SCCACHE_CACHE_SIZE: "60G",
    // Jobs share the host's network: each slot its own sccache server (never the owner's, on
    // the default 4226) and its own dev/Playwright ports (scripts/dev-env.mjs; worktrees use
    // 20000-29999).
    SCCACHE_SERVER_PORT: String(4310 + slot),
    ETHER_INSTANCE: `ci-pool-${slot}`,
    ETHER_DEV_PORT: String(31000 + 10 * slot),
    npm_config_store_dir: p("cache", "pnpm-store"),
    PLAYWRIGHT_BROWSERS_PATH: p("cache", "ms-playwright"),
    COREPACK_HOME: p("cache", "corepack"),
    ACTIONS_RUNNER_HOOK_JOB_STARTED: join(here, "job-started.sh"),
    ACTIONS_RUNNER_HOOK_JOB_COMPLETED: join(here, "job-completed.sh"),
    CI_POOL_JOB: job,
    CI_POOL_TARGETS: p("targets"),
  };
  const props = [
    // Nothing of the owner's home but the pool's directory (this slot's at a fixed path) and the
    // read-only checkout the hooks come from; no user bus or agent sockets, no Docker, no view of
    // other processes, a private /tmp, a read-only system.
    "ProtectHome=tmpfs",
    `BindPaths=${home}`,
    `BindPaths=${dir}:${job}`,
    `BindReadOnlyPaths=${root}`,
    "ProtectSystem=strict",
    `ReadWritePaths=${home}`,
    `TemporaryFileSystem=/run/user/${userInfo().uid}`,
    "InaccessiblePaths=/run/docker.sock",
    "PrivateTmp=yes",
    "PrivatePIDs=yes",
    "NoNewPrivileges=yes",
    "Nice=10",
    "RuntimeMaxSec=4h",
  ];
  sh("systemd-run", [
    "--user",
    "--quiet",
    "--collect",
    `--unit=${unitOf(slot)}`,
    `--slice=${SLICE}`,
    ...props.flatMap((x) => ["-p", x]),
    "/usr/bin/env",
    "-i",
    ...Object.entries(vars).map(([k, v]) => `${k}=${v}`),
    "bash",
    join(here, "slot.sh"),
  ]);
  log(`slot ${slot}: runner ${name} up`);
  return jit.runner.id;
}

const active = (slot) => ["active", "activating", "deactivating"].includes(systemctl("is-active", unitOf(slot)).stdout.trim());

// Keeps the cargo target dirs in bounds: a dir above the size cap is emptied, one unused for days
// is deleted, each only when no job holds its lock (slot.sh). Slots' fallback dirs get the size cap.
function pruneTargets() {
  const kb = (d) => Number(sh("du", ["-sxk", d], { allowFail: true }).stdout.split(/\s/)[0] || 0);
  const maxKb = cfg.targetMaxGb * 1e6;
  for (const f of readdirSync(p("targets")).filter((f) => f.endsWith(".lock"))) {
    const lock = p("targets", f);
    const dir = lock.slice(0, -".lock".length);
    const idleDays = (Date.now() - statSync(lock).mtimeMs) / 86400e3;
    const what = idleDays > cfg.targetDays ? "delete" : kb(dir) > maxKb ? "empty" : null;
    if (!what) continue;
    const cmd = what === "delete" ? `rm -rf "$1" "$1.lock"` : `find "$1" -mindepth 1 -delete`;
    if (sh("flock", ["-n", lock, "sh", "-c", cmd, "_", dir], { allowFail: true }).status === 0)
      log(`targets: ${what === "delete" ? "deleted unused" : "emptied"} ${dir}`);
  }
  for (let slot = 1; slot <= cfg.slots; slot++) {
    const t = p("slots", String(slot), "target");
    if (!active(slot) && existsSync(t) && kb(t) > maxKb) {
      rmSync(t, { recursive: true, force: true });
      log(`targets: emptied ${t}`);
    }
  }
}

// This machine's registrations that no live slot owns (left by a crash or an earlier supervisor).
async function pruneRunners(owned) {
  const { runners } = await api("GET", `repos/${cfg.repo}/actions/runners?per_page=100`);
  for (const r of runners) {
    if (!r.name.startsWith(NAME) || r.busy || owned.has(r.id)) continue;
    await api("DELETE", `repos/${cfg.repo}/actions/runners/${r.id}`).catch((e) => log(`prune ${r.name}: ${e.message}`));
  }
}

async function run() {
  await setup();
  log(`pool for ${cfg.repo}: ${cfg.slots} slots in ${cfg.cpus} CPUs / ${cfg.memoryGb} GB, labels ${cfg.labels.join(",")}`);
  let stopping = false;
  // Busy slots finish their job after the supervisor stops (their units are not its children).
  for (const sig of ["SIGTERM", "SIGINT"])
    process.on(sig, () => {
      stopping = true;
    });
  let lastPrune = 0;
  let lastTargets = 0;
  let lowDisk = false;
  const slots = new Map(); // slot -> { runnerId, at, failures, retryAt } of the runner it last started
  while (!stopping) {
    for (let slot = 1; slot <= cfg.slots && !stopping; slot++) {
      if (active(slot)) continue;
      const last = slots.get(slot);
      if (last?.runnerId) {
        // The unit is gone: its job is done (the runner deregistered itself) or it died early.
        await api("DELETE", `repos/${cfg.repo}/actions/runners/${last.runnerId}`).catch(() => {});
        const out = sh("journalctl", ["--user", "-u", unitOf(slot), "--since", `@${Math.floor(last.at / 1000)}`, "-o", "cat", "--no-pager"], {
          allowFail: true,
        }).stdout;
        const job = /Running job: (.*)/.exec(out)?.[1];
        const secs = Math.round((Date.now() - last.at) / 1000);
        if (job) {
          log(`slot ${slot}: ran "${job}" (${secs} s with startup)`);
          last.failures = 0;
        } else {
          log(`slot ${slot}: runner exited after ${secs} s without a job:\n${out.trim().split("\n").slice(-12).join("\n")}`);
          last.failures++;
        }
        // A runner that keeps dying backs off, up to 5 min.
        last.retryAt = Date.now() + (last.failures ? Math.min(300_000, 15_000 * 2 ** last.failures) : 0);
        last.runnerId = null;
      }
      if (last && Date.now() < last.retryAt) continue;
      const free = freeGb();
      if (free < cfg.minFreeGb) {
        if (!lowDisk) log(`::warning:: ${free.toFixed(0)} GB free (< ${cfg.minFreeGb}): not starting runners`);
        lowDisk = true;
        break;
      }
      lowDisk = false;
      systemctl("reset-failed", unitOf(slot));
      try {
        const runnerId = await start(slot);
        slots.set(slot, { runnerId, at: Date.now(), failures: last?.failures ?? 0, retryAt: 0 });
      } catch (e) {
        log(`slot ${slot}: ${e.message}`);
      }
    }
    if (Date.now() - lastTargets > 30 * 60_000) {
      lastTargets = Date.now();
      pruneTargets();
    }
    if (Date.now() - lastPrune > 10 * 60_000) {
      lastPrune = Date.now();
      const owned = new Set([...slots.values()].map((s) => s.runnerId).filter(Boolean));
      // Slots adopted from an earlier supervisor run keep their runners.
      if (slots.size >= cfg.slots) await pruneRunners(owned).catch((e) => log(`prune: ${e.message}`));
    }
    await new Promise((r) => setTimeout(r, 5000));
  }
  log("supervisor stopped; busy runners finish their job");
}

async function status() {
  const { runners } = await api("GET", `repos/${cfg.repo}/actions/runners?per_page=100`);
  console.log(`free disk: ${freeGb().toFixed(0)} GB (min ${cfg.minFreeGb})`);
  for (let slot = 1; slot <= cfg.slots; slot++) {
    const r = runners.find((x) => x.name.startsWith(NAME) && slotOf(x.name) === String(slot));
    console.log(`slot ${slot}: unit ${active(slot) ? "up" : "down"}, runner ${r ? `${r.name} ${r.status}${r.busy ? " busy" : ""}` : "-"}`);
  }
}

async function stop() {
  const { runners } = await api("GET", `repos/${cfg.repo}/actions/runners?per_page=100`);
  for (const r of runners) {
    if (!r.name.startsWith(NAME) || r.busy) continue;
    systemctl("stop", unitOf(slotOf(r.name)));
    await api("DELETE", `repos/${cfg.repo}/actions/runners/${r.id}`).catch((e) => log(e.message));
    log(`removed idle ${r.name}`);
  }
}

const cmd = process.argv[2];
if (cmd === "setup") await setup();
else if (cmd === "run") await run();
else if (cmd === "status") await status();
else if (cmd === "stop") await stop();
else {
  console.error("usage: pool.mjs setup | run | status | stop");
  process.exit(2);
}
