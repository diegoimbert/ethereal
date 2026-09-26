#!/usr/bin/env python3
"""Fail if a `node/<id>` branch changes files outside the node's owned globs.

Usage: check-ownership.py [--branch BRANCH] [--base REF] [--files FILE...]
  BRANCH defaults to $GITHUB_HEAD_REF, then the current git branch.
  BASE   defaults to origin/main (diff is BASE...HEAD, i.e. since the merge base).
Branches not named `node/*` are skipped (exit 0). Mapping: .github/ownership.toml.
"""

import argparse
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CONFIG = ROOT / ".github" / "ownership.toml"


def glob_to_regex(glob: str) -> re.Pattern:
    out, i = [], 0
    while i < len(glob):
        c = glob[i]
        if glob.startswith("**/", i):
            out.append("(?:.*/)?")
            i += 3
        elif glob.startswith("**", i):
            out.append(".*")
            i += 2
        elif c == "*":
            out.append("[^/]*")
            i += 1
        elif c == "?":
            out.append("[^/]")
            i += 1
        elif c == "{":
            j = glob.index("}", i)
            out.append("(?:" + "|".join(re.escape(p) for p in glob[i + 1 : j].split(",")) + ")")
            i = j + 1
        else:
            out.append(re.escape(c))
            i += 1
    return re.compile("^" + "".join(out) + "$")


def matches(path: str, globs: list[str]) -> bool:
    return any(glob_to_regex(g).match(path) for g in globs)


def git(*args: str) -> str:
    return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--branch")
    ap.add_argument("--base", default="origin/main")
    ap.add_argument("--files", nargs="*")
    args = ap.parse_args()

    branch = args.branch or os.environ.get("GITHUB_HEAD_REF") or git("rev-parse", "--abbrev-ref", "HEAD")
    if not branch.startswith("node/"):
        print(f"ownership: branch '{branch}' is not node/*, skipping")
        return 0
    node = branch.removeprefix("node/")

    cfg = tomllib.loads(CONFIG.read_text())
    glob_cfg = cfg.get("global", {})
    if node in glob_cfg.get("unrestricted", []) or any(
        node.startswith(p) for p in glob_cfg.get("unrestricted_prefixes", [])
    ):
        print(f"ownership: node '{node}' is unrestricted")
        return 0
    spec = cfg.get("nodes", {}).get(node)
    if spec is None:
        print(f"ownership: node '{node}' has no entry in {CONFIG.relative_to(ROOT)}; ask the manager")
        return 1

    files = args.files
    if files is None:
        files = [f for f in git("diff", "--name-only", f"{args.base}...HEAD").splitlines() if f]

    allowed = spec.get("paths", []) + glob_cfg.get("allow", [])
    excluded = spec.get("exclude", [])
    bad = [f for f in files if not matches(f, allowed) or matches(f, excluded)]
    if bad:
        print(f"ownership: node '{node}' changed files outside its owned paths:")
        for f in bad:
            print(f"  - {f}")
        print("Owned:", ", ".join(spec.get("paths", [])), "| excluded:", ", ".join(excluded) or "-")
        print("Send a BCR to the manager instead (ORCHESTRATION.md §6.1).")
        return 1
    print(f"ownership: {len(files)} file(s) OK for node '{node}'")
    return 0


if __name__ == "__main__":
    sys.exit(main())
