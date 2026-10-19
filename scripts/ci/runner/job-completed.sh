#!/usr/bin/env bash
# The job-completed hook of the devbox pool's runners: no private code stays in the workspace.
set -euo pipefail
"$(dirname "$0")/clean-workspace.sh"
