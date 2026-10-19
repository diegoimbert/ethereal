#!/usr/bin/env bash
# One job, then exit: an ephemeral just-in-time runner in the sandboxed unit pool.mjs starts per
# job (docs/CI-RUNNERS.md). Inside the unit, $CI_POOL_JOB is this slot's directory (bind-mounted at
# the same path in every slot, so builds see the same paths whichever slot runs them).
set -euo pipefail

jit=$(cat "$CI_POOL_JOB/jit")
rm -f "$CI_POOL_JOB/jit"

# Cargo target dirs belong to a kind of job, not to a slot: a job lands on any free slot, and the
# target dir its kind used last time is what makes its build incremental. $CI_POOL_TARGETS holds
# `<job>-<n>` dirs shared by every slot; the job-started hook asks this helper for one, and the
# helper holds that dir's lock (an flock, released when the unit ends) for the whole job, so two
# concurrent jobs of one kind never share a dir. Until then (and if every dir of the kind is
# taken) CARGO_TARGET_DIR points at the slot's own.
mkdir -p "$CI_POOL_JOB/target" "$CI_POOL_TARGETS"
ln -sfn "$CI_POOL_JOB/target" "$CARGO_TARGET_DIR"
mkfifo /tmp/target.req /tmp/target.ans
(
  read -r kind < /tmp/target.req
  kind="${kind//[^A-Za-z0-9_-]/_}"
  for n in 1 2 3 4 5 6 7 8; do
    dir="$CI_POOL_TARGETS/$kind-$n"
    exec 9> "$dir.lock"
    if flock -n 9; then
      mkdir -p "$dir" && touch "$dir.lock" && ln -sfn "$dir" "$CARGO_TARGET_DIR"
      echo "$dir" > /tmp/target.ans
      exec sleep infinity
    fi
  done
  echo "$CI_POOL_JOB/target" > /tmp/target.ans
) &

cd "$CI_POOL_JOB/runner"
exec ./run.sh --jitconfig "$jit"
