#!/usr/bin/env bash
# The job-started hook of the devbox pool's runners (the runner only runs .sh/.ps1 hooks): the trust
# check (trust.mjs) first, then a clean workspace (clean-workspace.sh), then the job's cargo target
# dir (slot.sh).
set -euo pipefail
node "$(dirname "$0")/job-started.mjs"
"$(dirname "$0")/clean-workspace.sh"
timeout 10 bash -c 'echo "$GITHUB_JOB" > /tmp/target.req' || true
echo "devbox pool: cargo target dir $(timeout 10 cat /tmp/target.ans)"
