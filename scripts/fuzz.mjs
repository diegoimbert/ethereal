#!/usr/bin/env node
// cargo-fuzz over the parsers that take untrusted bytes (fuzz/; needs a nightly toolchain and
// `cargo install cargo-fuzz`).
//
//   node scripts/fuzz.mjs list
//   node scripts/fuzz.mjs run <target> [seconds]   one target (default 60 s)
//   node scripts/fuzz.mjs all [seconds]            every target, one after the other
//
// The corpus lives in fuzz/corpus/<target> (git-ignored), seeded from the test fixtures. With
// FUZZ_QUIET=1 (CI: the log is public) libFuzzer's output goes to fuzz/logs/<target>.log and a
// failure prints only the target, the seed and the crashing input's hash, never the input or
// the stack: reproduce locally with `node scripts/fuzz.mjs run <target>` and `-seed=<seed>`
// (FUZZ_ARGS), then fix it the private way first.
import { execFileSync, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, openSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
// The cargo-fuzz crate (FUZZ_DIR, relative to the repository root: another crate, e.g. ee/fuzz,
// can list its seeds in its own seeds.json).
const fuzz = join(root, process.env.FUZZ_DIR || "fuzz");
const quiet = process.env.FUZZ_QUIET === "1";
// The nightly toolchain cargo-fuzz runs on (CI pins a dated one).
const toolchain = `+${process.env.FUZZ_TOOLCHAIN || "nightly"}`;
// The target triple to build for (a prebuilt cargo-fuzz defaults to the triple it was built for,
// e.g. musl; CI sets the runner's).
const triple = process.env.FUZZ_TRIPLE ? ["--target", process.env.FUZZ_TRIPLE] : [];

// Seeds per target: fixture folders (every file) and inline bytes.
const SEEDS = {
  media_decode: { dirs: ["crates/ether-media/tests/fixtures"] },
  project_file: { dirs: ["crates/ether-model/tests/fixtures"] },
  midi_file: {
    inline: [
      // A format 0 file, 96 PPQ, one note.
      Buffer.from("4d546864000000060000000100604d54726b0000000d00903c40600080 3c4000ff2f00".replace(/ /g, ""), "hex"),
    ],
  },
  server_frames: {
    inline: [Buffer.from('{"type":"Command","id":1,"command":{"type":"Transport","command":{"type":"Play"}}}')],
  },
};

// `<fuzz dir>/seeds.json`: { "<target>": { "dirs": [...], "hex": [...] } } (dirs relative to the
// repository root).
const extra = existsSync(join(fuzz, "seeds.json")) ? JSON.parse(readFileSync(join(fuzz, "seeds.json"), "utf8")) : {};

function targets() {
  return readdirSync(join(fuzz, "fuzz_targets"))
    .filter((f) => f.endsWith(".rs"))
    .map((f) => f.slice(0, -3))
    .sort();
}

function seed(target) {
  const dir = join(fuzz, "corpus", target);
  mkdirSync(dir, { recursive: true });
  const s = { ...(process.env.FUZZ_DIR ? {} : SEEDS[target]), ...extra[target] };
  for (const d of s.dirs ?? []) {
    for (const f of readdirSync(join(root, d), { recursive: true, withFileTypes: true })) {
      if (!f.isFile()) continue;
      const from = join(f.parentPath, f.name);
      copyFileSync(from, join(dir, `seed-${from.slice(root.length + 1).replace(/[^A-Za-z0-9._-]/g, "_")}`));
    }
  }
  (s.inline ?? []).forEach((b, i) => writeFileSync(join(dir, `seed-inline-${i}`), b));
  (s.hex ?? []).forEach((h, i) => writeFileSync(join(dir, `seed-hex-${i}`), Buffer.from(h, "hex")));
  return dir;
}

function run(target, seconds) {
  const corpus = seed(target);
  const seedValue = String(1 + Math.floor(Math.random() * 2 ** 31));
  const args = [
    toolchain,
    "fuzz",
    "run",
    "-O",
    ...triple,
    target,
    corpus,
    "--",
    `-max_total_time=${seconds}`,
    "-rss_limit_mb=2048",
    "-max_len=262144",
    "-timeout=20",
    `-seed=${seedValue}`,
    ...(process.env.FUZZ_ARGS ?? "").split(" ").filter(Boolean),
  ];
  let stdio = "inherit";
  const log = join(fuzz, "logs", `${target}.log`);
  if (quiet) {
    mkdirSync(dirname(log), { recursive: true });
    const fd = openSync(log, "w");
    stdio = ["ignore", fd, fd];
  }
  const r = spawnSync("cargo", args, { cwd: fuzz, stdio });
  if (r.status === 0) {
    console.log(`${target}: ok (${seconds} s, seed ${seedValue})`);
    return true;
  }
  const art = join(fuzz, "artifacts", target);
  const found = existsSync(art) ? readdirSync(art) : [];
  const hashes = found.map((f) =>
    createHash("sha256")
      .update(readFileSync(join(art, f)))
      .digest("hex")
      .slice(0, 16),
  );
  console.log(`${target}: FAILED (exit ${r.status}, seed ${seedValue}${hashes.length ? `, input sha256 ${hashes.join(" ")}` : ""})`);
  return false;
}

const [cmd, a, b] = process.argv.slice(2);
if (cmd === "list") {
  console.log(targets().join("\n"));
} else if (cmd === "run" && a) {
  if (!targets().includes(a)) throw new Error(`no fuzz target ${a} (node scripts/fuzz.mjs list)`);
  process.exit(run(a, Number(b ?? 60)) ? 0 : 1);
} else if (cmd === "all") {
  // The build output is public-safe (no input reached the code yet).
  execFileSync("cargo", [toolchain, "fuzz", "build", "-O", ...triple], { cwd: fuzz, stdio: "inherit" });
  const failed = targets().filter((t) => !run(t, Number(a ?? 60)));
  process.exit(failed.length ? 1 : 0);
} else {
  console.error("usage: node scripts/fuzz.mjs list | run <target> [seconds] | all [seconds]");
  process.exit(2);
}
