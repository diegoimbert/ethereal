// scripts/ci/runner/thermal-guard.sh against fake sensors and a fake systemctl.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const guard = join(dirname(fileURLToPath(import.meta.url)), "runner", "thermal-guard.sh");

function machine() {
  const dir = mkdtempSync(join(tmpdir(), "thermal-"));
  const hwmon = join(dir, "hwmon");
  const bin = join(dir, "bin");
  mkdirSync(bin);
  // Fake systemctl: quotas live in files; `show` prints them as systemd does ("20s").
  writeFileSync(
    join(bin, "systemctl"),
    `#!/bin/sh
shift
case "$1" in
  show) cat "${dir}/q-$2" 2>/dev/null || echo infinity ;;
  set-property) q=\${4#CPUQuota=}; echo "\${q%00%}s" > "${dir}/q-$3"; echo "$3 $4" >> "${dir}/calls" ;;
esac
`,
    { mode: 0o755 },
  );
  const sensor = (i, name, c) => {
    mkdirSync(join(hwmon, `hwmon${i}`), { recursive: true });
    writeFileSync(join(hwmon, `hwmon${i}`, "name"), `${name}\n`);
    writeFileSync(join(hwmon, `hwmon${i}`, "temp1_input"), `${c * 1000}\n`);
  };
  const run = (temps, extra = {}) => {
    sensor(0, "k10temp", temps.tctl);
    sensor(1, "spd5118", temps.dimm);
    sensor(2, "spd5118", temps.dimm2 ?? 30);
    sensor(3, "nvme", 99);
    const r = spawnSync("bash", [guard], {
      encoding: "utf8",
      env: { PATH: `${bin}:/usr/bin:/bin`, HOME: dir, CI_THERMAL_HWMON: hwmon, CI_THERMAL_DIR: join(dir, "t"), CI_POOL_CPUS: "20", ...extra },
    });
    assert.equal(r.status, 0, r.stderr);
    const q = (s) => readFileSync(join(dir, `q-${s}`), "utf8").trim();
    return { ci: q("ethereal-ci.slice"), pool: q("ethereal-ci-pool.slice"), state: readFileSync(join(dir, "t", "state"), "utf8").trim() };
  };
  const calls = () => (existsSync(join(dir, "calls")) ? readFileSync(join(dir, "calls"), "utf8").trim().split("\n") : []);
  return { run, calls, dir };
}

test("throttles when hot, keeps it through the band, restores when cool", () => {
  const m = machine();
  assert.deepEqual(m.run({ tctl: 60, dimm: 45 }), { ci: "20s", pool: "20s", state: "full" });
  assert.deepEqual(m.run({ tctl: 91, dimm: 45 }), { ci: "10s", pool: "10s", state: "low" });
  assert.deepEqual(m.run({ tctl: 85, dimm: 45 }), { ci: "10s", pool: "10s", state: "low" });
  assert.deepEqual(m.run({ tctl: 79, dimm: 66 }), { ci: "10s", pool: "10s", state: "low" });
  assert.deepEqual(m.run({ tctl: 79, dimm: 60 }), { ci: "20s", pool: "20s", state: "full" });
  // The hottest DIMM counts.
  assert.deepEqual(m.run({ tctl: 60, dimm: 40, dimm2: 76 }), { ci: "10s", pool: "10s", state: "low" });
  const log = readFileSync(join(m.dir, "t", "log"), "utf8")
    .trim()
    .split("\n");
  assert.equal(log.length, 4);
  assert.match(log[0], /tctl=60C dimm=45C -> full \(ci 20 CPUs, pool 20\)/);
  assert.match(log[1], /tctl=91C .* -> low/);
});

test("reconciles a quota moved by someone else, without a state change", () => {
  const m = machine();
  m.run({ tctl: 60, dimm: 45 });
  const n = m.calls().length;
  m.run({ tctl: 60, dimm: 45 });
  assert.equal(m.calls().length, n, "no calls when nothing changed");
  writeFileSync(join(m.dir, "q-ethereal-ci-pool.slice"), "16s\n");
  assert.deepEqual(m.run({ tctl: 60, dimm: 45 }), { ci: "20s", pool: "20s", state: "full" });
  assert.equal(m.calls().length, n + 1);
});

test("thresholds and quotas come from the environment", () => {
  const m = machine();
  const env = { CI_CPUS: "24", CI_THERMAL_LOW_CPUS: "6", CI_THERMAL_HOT_TCTL: "85", CI_THERMAL_COOL_TCTL: "70" };
  assert.deepEqual(m.run({ tctl: 84, dimm: 45 }, env), { ci: "24s", pool: "20s", state: "full" });
  assert.deepEqual(m.run({ tctl: 86, dimm: 45 }, env), { ci: "6s", pool: "6s", state: "low" });
  assert.deepEqual(m.run({ tctl: 71, dimm: 45 }, env), { ci: "6s", pool: "6s", state: "low" });
  assert.deepEqual(m.run({ tctl: 70, dimm: 45 }, env), { ci: "24s", pool: "20s", state: "full" });
});

test("no sensors: never throttled", () => {
  const m = machine();
  const dir = mkdtempSync(join(tmpdir(), "nohwmon-"));
  const r = spawnSync("bash", [guard], {
    encoding: "utf8",
    env: { PATH: `${join(m.dir, "bin")}:/usr/bin:/bin`, HOME: m.dir, CI_THERMAL_HWMON: dir, CI_THERMAL_DIR: join(m.dir, "t"), CI_POOL_CPUS: "12" },
  });
  assert.equal(r.status, 0, r.stderr);
  assert.equal(readFileSync(join(m.dir, "t", "state"), "utf8").trim(), "full");
  assert.equal(readFileSync(join(m.dir, "q-ethereal-ci-pool.slice"), "utf8").trim(), "12s");
});
