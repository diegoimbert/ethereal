// The names of what failed in a tool's output (tests, crates, specs, files), never its messages:
// scripts/ci/private-step.mjs prints these in the public log of the full-product CI.
/** The names of what failed, from the tools' own report lines (never their messages). */
export function failures(log) {
  const found = new Set();
  const add = (s) => found.add(s.trim().slice(0, 200));
  for (const line of log.split(/\r?\n/)) {
    let m;
    // cargo-nextest: `FAIL [ 0.1s] (3/9) crate::binary test_name`
    if ((m = /^\s*(?:TRY \d+\s+)?(FAIL|TIMEOUT|SIGSEGV|SIGABRT|SIGKILL|FLAKY)\s+\[[^\]]*\]\s+(?:\([^)]*\)\s+)?(\S+\s+\S+)/.exec(line))) add(`${m[1].toLowerCase()} ${m[2]}`);
    // libtest: `test path::name ... FAILED`
    else if ((m = /^test (\S+) \.\.\. FAILED$/.exec(line))) add(`fail ${m[1]}`);
    // cargo: `error: could not compile `crate` ...`
    else if ((m = /^error: could not compile `([^`]+)`/.exec(line))) add(`compile error in ${m[1]}`);
    // Playwright: `  ✘  12 [project] › e2e/x.spec.ts:10:3 › title (1.0s)`
    else if ((m = /^\s*✘\s+\d+\s+\[([^\]]+)\]\s+›\s+(\S+)/.exec(line))) add(`fail [${m[1]}] ${m[2]}`);
    // vitest: ` FAIL  src/x.test.ts > suite > test`
    else if ((m = /^\s*FAIL\s+(\S+\.(?:test|spec)\.[cm]?[jt]sx?)/.exec(line))) add(`fail ${m[1]}`);
    // tsc: `src/x.ts(1,2): error TS1234: ...`
    else if ((m = /^(\S+\.[cm]?[jt]sx?)\(\d+,\d+\): error TS\d+/.exec(line))) add(`type error in ${m[1]}`);
    // prettier --check: `[warn] src/x.ts`
    else if ((m = /^\[warn\] (\S+\.\w+)$/.exec(line))) add(`not formatted: ${m[1]}`);
    // eslint: a file path line followed by `  1:2  error ...`
    else if ((m = /^\/\S+\.[cm]?[jt]sx?$/.exec(line))) found.add(`lint: ${line.trim().replace(/^.*?\/(?=(crates|ui|apps|services|scripts)\/)/, "")}`);
    // rustfmt --check: `Diff in /path/file.rs:12:`
    else if ((m = /^Diff in (\S+\.rs)/.exec(line))) add(`not formatted: ${m[1].replace(/^.*?\/(?=crates\/|apps\/)/, "")}`);
  }
  return [...found];
}
