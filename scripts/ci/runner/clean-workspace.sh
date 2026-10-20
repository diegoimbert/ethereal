#!/usr/bin/env bash
# Leaves no private code in a pool slot's runner workspace (docs/CI-RUNNERS.md): a full-product job
# (full.yml) fetches the private `ee/` repository into its checkout, and the workspace outlives the
# job, while the next job on the slot may be an open-source one whose artifacts are public.
# actions/checkout's clean does not reach inside `ee/` (a submodule path), so after every job (and
# before the next, in case a job was killed first): if `ee/` holds anything, put the checkout back
# to its commit, remove every untracked file and empty `ee/`. Open-source jobs leave `ee/` empty,
# so this costs them nothing. Cargo target dirs live outside the workspace.
set -euo pipefail
shopt -s nullglob
for ws in "$CI_POOL_JOB"/runner/_work/*/*/; do
  ws="${ws%/}"
  [ -d "$ws/.git" ] && [ -d "$ws/ee" ] || continue
  [ -n "$(ls -A "$ws/ee")" ] || continue
  find "$ws/ee" -mindepth 1 -delete
  git -C "$ws" reset -q --hard HEAD
  git -C "$ws" clean -qffdx
  echo "devbox pool: removed ee/ from $(basename "$ws")"
done
