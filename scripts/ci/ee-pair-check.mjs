// Files the EE side (ee/) keeps in step with the OSS side, checked for each OSS PR without a
// build (the `ee/pointer` status, scripts/feat.mjs sync):
//   - ee/app/src/generated: the full product's TypeScript protocol types, OSS ui/src/generated
//     plus the EE domains (a few OSS files, like Command.ts or index.ts, are extended there).
//     An OSS PR that changes a generated file needs the regenerated EE copy in its paired EE PR.
//   - ee/Cargo.lock: ee/ builds the OSS crates as path dependencies with its own lockfile. An
//     OSS PR that changes the dependencies of an OSS crate needs the same in ee/Cargo.lock.
// Both checks only report what the PR changes (its base may already differ: that is not its
// fault, and main's full build reports it).

export const OSS_TYPES_DIR = "ui/src/generated";
export const EE_TYPES_DIR = "app/src/generated";
export const TYPES_FIX = "cd ee && just gen-types; commit; just feat push";
export const LOCK_FIX = "cd ee && cargo fetch; commit; just feat push";

// Generated type files the PR changed whose EE copy did not follow. Each argument maps a file
// name in the generated directory to its git blob sha (blob shas of equal contents are equal
// across repositories): the OSS directory at the PR's merge base and head, and the EE one at
// the pointers of those two commits.
export function staleTypes({ ossBase, ossHead, eeBase, eeHead }) {
  const stale = [];
  for (const name of new Set([...ossBase.keys(), ...ossHead.keys()])) {
    const [before, after] = [ossBase.get(name), ossHead.get(name)];
    if (before === after) continue; // the PR leaves it alone
    const [eeBefore, eeAfter] = [eeBase.get(name), eeHead.get(name)];
    const extended = before !== undefined && eeBefore !== undefined && eeBefore !== before;
    let ok;
    if (after === undefined) ok = eeAfter === undefined || eeAfter !== before; // removed in OSS
    else if (eeAfter === after) ok = true; // the same file on both sides
    else if (extended || before === undefined) ok = eeAfter !== undefined && eeAfter !== eeBefore; // EE adds to it: it must have been regenerated
    else ok = false;
    if (!ok) stale.push(name);
  }
  return stale.sort();
}

// Cargo.lock -> [{ name, version, source, deps: [dependency strings] }].
export function parseLock(text) {
  const pkgs = [];
  let cur = null;
  let inDeps = false;
  for (const raw of text.split("\n")) {
    const line = raw.trim();
    if (line === "[[package]]") {
      cur = { name: "", version: "", source: null, deps: [] };
      pkgs.push(cur);
      inDeps = false;
      continue;
    }
    if (!cur) continue;
    if (inDeps) {
      if (line.startsWith("]")) inDeps = false;
      else {
        const m = line.match(/^"([^"]+)"/);
        if (m) cur.deps.push(m[1]);
      }
      continue;
    }
    const kv = line.match(/^(\w+) = (.*)$/);
    if (!kv) continue;
    const [, k, v] = kv;
    if (k === "dependencies") {
      // `dependencies = [` then one per line, or all on one line
      const inline = [...v.matchAll(/"([^"]+)"/g)].map((m) => m[1]);
      cur.deps.push(...inline);
      inDeps = !v.includes("]");
    } else if (k === "name" || k === "version" || k === "source") cur[k] = JSON.parse(v);
  }
  return pkgs;
}

// The resolved direct dependencies ("name version") of each path package (no `source`: the
// workspace crates and their path dependencies) of a lockfile.
function pathDeps(pkgs) {
  const byName = new Map();
  for (const p of pkgs) byName.set(p.name, [...(byName.get(p.name) ?? []), p]);
  const resolve = (dep) => {
    const [name, version] = dep.split(" ");
    if (version) return `${name} ${version}`;
    const c = byName.get(name) ?? [];
    return c.length === 1 ? `${name} ${c[0].version}` : name;
  };
  const out = new Map();
  for (const p of pkgs) if (!p.source) out.set(p.name, new Set(p.deps.map(resolve)));
  return out;
}

// Differences of the OSS crates' direct dependencies between the OSS lockfile and ee/'s:
// [{ crate, sign: "+" (in OSS, missing from ee/) | "-" (only in ee/), dep: "name version" }].
// Crates ee/ does not build are skipped.
export function lockDiffs(ossText, eeText) {
  const oss = pathDeps(parseLock(ossText));
  const ee = pathDeps(parseLock(eeText));
  const out = [];
  for (const [crate, deps] of oss) {
    const theirs = ee.get(crate);
    if (!theirs) continue;
    for (const dep of deps) if (!theirs.has(dep)) out.push({ crate, sign: "+", dep });
    for (const dep of theirs) if (!deps.has(dep)) out.push({ crate, sign: "-", dep });
  }
  return out;
}
const key = (d) => `${d.crate}: ${d.sign}${d.dep}`;

// What the PR adds to those differences (present at its head, not at its base) that ee/ must
// follow: a dependency an OSS crate gained or lost. A version that differs is not reported (ee/
// may follow OSS's bumps later: Dependabot PRs can't pair), nor a gained dependency that ee/
// does not resolve: `isNormal(crate, name)` says whether the crate's manifest lists it as a
// non-optional normal or build dependency (ee/ builds the OSS crates as path dependencies:
// their dev-dependencies and unused optional ones are not in its lockfile).
export function lockDrift({ ossBase, ossHead, eeBase, eeHead }, isNormal = () => true) {
  const before = new Set(lockDiffs(ossBase, eeBase).map(key));
  const now = lockDiffs(ossHead, eeHead);
  const name = (d) => d.dep.split(" ")[0];
  const bumped = (d) => now.some((o) => o.crate === d.crate && o.sign !== d.sign && name(o) === name(d));
  return now
    .filter((d) => !before.has(key(d)) && !bumped(d))
    .filter((d) => d.sign === "-" || isNormal(d.crate, name(d)))
    .map(key)
    .sort();
}

// A Cargo.toml -> { name: its package name, deps: the names of the normal and build
// dependencies (any target) a default build resolves: the non-optional ones and the optional
// ones its default features turn on }. Enough TOML for this repository's manifests.
export function manifestDeps(text) {
  let section = "";
  let name = null;
  const deps = new Map(); // name -> optional
  const features = new Map(); // feature -> [entries]
  let table = null; // a [dependencies.foo] table
  let pending = null; // a feature whose array continues on the next lines
  const isDeps = (s) => /^(?:target\..+\.)?(?:dependencies|build-dependencies)$/.test(s);
  for (const raw of text.split("\n")) {
    const line = raw.replace(/#.*$/, "").trim();
    if (pending) {
      features.get(pending).push(...[...line.matchAll(/"([^"]+)"/g)].map((m) => m[1]));
      if (line.includes("]")) pending = null;
      continue;
    }
    const h = line.match(/^\[([^\]]+)\]$/);
    if (h) {
      section = h[1].replace(/\s+/g, "");
      table = null;
      const t = section.match(/^(?:target\..+\.)?(?:dependencies|build-dependencies)\.([\w-]+)$/);
      if (t) {
        table = t[1];
        if (!deps.has(table)) deps.set(table, false);
      }
      continue;
    }
    const kv = line.match(/^([\w-]+)((?:\.[\w-]+)*)\s*=\s*(.*)$/);
    if (!kv) continue;
    const [, key, sub, value] = kv;
    if (section === "package") {
      if (key === "name" && !sub) name = JSON.parse(value);
    } else if (section === "features") {
      features.set(key, [...value.matchAll(/"([^"]+)"/g)].map((m) => m[1]));
      if (value.startsWith("[") && !value.includes("]")) pending = key;
    } else if (table) {
      if (key === "optional" && /^true\b/.test(value)) deps.set(table, true);
    } else if (isDeps(section)) {
      const optional = sub === ".optional" ? /^true\b/.test(value) : /optional\s*=\s*true/.test(value);
      deps.set(key, (deps.get(key) ?? false) || optional);
    }
  }
  // The optional dependencies the default features turn on (`dep:x`, `x`, `x/feature`).
  const on = new Set();
  const seen = new Set();
  const walk = (f) => {
    if (seen.has(f)) return;
    seen.add(f);
    for (const e of features.get(f) ?? []) {
      const d = e.startsWith("dep:") ? e.slice(4) : e.split(/\??\//)[0];
      if (deps.has(d) && (e.startsWith("dep:") || e.includes("/") || !features.has(e))) on.add(d);
      if (features.has(e)) walk(e);
    }
  };
  walk("default");
  return { name, deps: new Set([...deps].filter(([d, opt]) => !opt || on.has(d)).map(([d]) => d)) };
}

// One status description (GitHub keeps 140 characters).
export function describe(what, items, fix) {
  const shown = items.slice(0, 2).join(", ") + (items.length > 2 ? ` +${items.length - 2}` : "");
  return `${what} (${shown}): ${fix}`.slice(0, 139);
}
