#!/usr/bin/env node
// Runs one step of the full-product CI (this repo + the private `ee/` repository) without leaking
// private code into the public Actions log: the command's output goes to a log file only
// (uploaded age-encrypted by .github/actions/full-logs), and the log shows a summary: the step's
// result and duration, and the names of what failed (tests, crates, specs, files).
//
//   node scripts/ci/private-step.mjs <name> -- <command> [args...]
//   node scripts/ci/private-step.mjs <name> -- "<shell command line>"     (one argument: bash -c)
//
// The log file is $FULL_LOG_DIR/<name>.log (default: $RUNNER_TEMP/full-logs). Exits with the
// command's code. Never prints a line of the command's output.
import { spawn } from "node:child_process";
import { appendFileSync, createWriteStream, mkdirSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { failures } from "./failures.mjs";

const args = process.argv.slice(2);
const sep = args.indexOf("--");
const name = args[0];
if (!name || sep !== 1 || args.length < 3 || !/^[\w.-]+$/.test(name)) {
  console.error("usage: private-step.mjs <name> -- <command> [args...]");
  process.exit(2);
}
const cmd = args.slice(2);
const dir = process.env.FULL_LOG_DIR || join(process.env.RUNNER_TEMP || tmpdir(), "full-logs");
mkdirSync(dir, { recursive: true });
const file = join(dir, `${name}.log`);
const out = createWriteStream(file);

const start = Date.now();
const [prog, ...rest] = cmd.length === 1 ? ["bash", "-c", cmd[0]] : cmd;
const child = spawn(prog, rest, { stdio: ["ignore", "pipe", "pipe"], shell: false });
child.stdout.pipe(out, { end: false });
child.stderr.pipe(out, { end: false });
child.on("error", (e) => {
  out.write(`\n[private-step] could not start ${prog}: ${e.message}\n`);
});
child.on("close", (code, signal) => {
  out.end(() => {
    const secs = Math.round((Date.now() - start) / 1000);
    const ok = code === 0;
    const lines = [`${name}: ${ok ? "ok" : `FAILED (${signal ? `signal ${signal}` : `exit ${code}`})`} in ${secs}s`];
    if (!ok) {
      const names = failures(readFileSync(file, "utf8"));
      for (const n of names.slice(0, 60)) lines.push(`  - ${n}`);
      if (names.length > 60) lines.push(`  - ... and ${names.length - 60} more`);
      if (names.length === 0) lines.push("  (no test or crate named in the output; see the encrypted full log)");
    }
    console.log(lines.join("\n"));
    if (!ok) console.log(`::error title=${name}::${name} failed (details: encrypted full log, \`just full-logs ${process.env.GITHUB_RUN_ID ?? "<run>"}\`)`);
    if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, `\`\`\`\n${lines.join("\n")}\n\`\`\`\n`);
    process.exit(code ?? 1);
  });
});
