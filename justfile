# Ethereal dev commands. Always use these (never hardcode ports/paths): every recipe runs
# with a per-instance ETHER_INSTANCE / ETHER_DEV_PORT (scripts/dev-env.mjs), so several
# worktrees/agents can run dev instances side by side. See README "Running multiple dev
# instances".

set shell := ["bash", "-euo", "pipefail", "-c"]

wasm_crates := "-p ether-protocol -p ether-model -p ether-core -p ether-devices -p ether-media -p ether-controller -p ether-stretch -p ether-wasm"
env := 'eval "$(node scripts/dev-env.mjs)"'

default:
    @just --list

# Print this instance's id and base dev port.
dev-port:
    @{{env}}; echo "instance=$ETHER_INSTANCE port=$ETHER_DEV_PORT (preview=$((ETHER_DEV_PORT+1)) playwright=$((ETHER_DEV_PORT+2)))"

# Install JS dependencies.
install:
    pnpm install --frozen-lockfile

# Standalone UI against MockTransport (no Rust needed).
dev-ui:
    {{env}}; echo "UI on http://localhost:$ETHER_DEV_PORT"; pnpm --filter @ethereal/ui dev

# Browser build host (COOP/COEP headers; wasm engine once wasm-host lands).
dev-web:
    {{env}}; echo "web on http://localhost:$ETHER_DEV_PORT"; pnpm --filter @ethereal/web dev

# Tauri desktop app (per-instance identifier, data dir and devUrl; ETHER_AUDIO=null for no device).
# Builds the sandbox helper first (sandboxed plugins; ETHER_SANDBOX_HELPER overrides it).
dev-desktop:
    {{env}}; node scripts/build-sandbox-helper.mjs; cd apps/desktop && pnpm tauri dev --config "{\"identifier\":\"dev.ethereal.$ETHER_INSTANCE\",\"build\":{\"devUrl\":\"http://localhost:$ETHER_DEV_PORT\",\"beforeDevCommand\":\"pnpm --filter @ethereal/ui dev\"}}"

# Desktop app with the null audio backend (no device, no contention).
dev-desktop-headless:
    ETHER_AUDIO=null just dev-desktop

# Install the headless Chromium used by the Playwright e2e suite (once per machine).
e2e-install:
    cd apps/web && npx playwright install chromium-headless-shell

# Playwright e2e of the web build (real wasm engine + controller, headless Chromium, this
# instance's Playwright port). Extra args go to `playwright test` (e.g. `just e2e-web flow`).
e2e-web *args:
    {{env}}; cd apps/web && node scripts/build-wasm.mjs && npx playwright test {{args}}

# Native smoke test: the desktop host (null audio backend) driven through the full user flow.
e2e-native:
    ETHER_AUDIO=null cargo test -p ether-native --test e2e_flow --test null_host

# Regenerate ui/src/theme/tokens.css from tokens.ts (the dev server also does this on save).
gen-tokens:
    cd ui && node src/theme/gen-css.mjs

# Inventory of hard-coded colors/sizes left in ui/src/features (for the design sweep).
report-hardcoded *dirs:
    cd ui && node src/theme/report-hardcoded.mjs {{dirs}}

# Regenerate TypeScript types from ether-protocol into ui/src/generated.
gen-types:
    cargo run -q -p ether-protocol --example gen-ts -- ui/src/generated

# Fail if ui/src/generated is stale (CI).
check-types-fresh: gen-types
    git diff --exit-code --stat -- ui/src/generated || (echo "ui/src/generated is stale: run 'just gen-types' and commit" && exit 1)
    test -z "$(git status --porcelain -- ui/src/generated)" || (git status --porcelain -- ui/src/generated; echo "untracked generated files: run 'just gen-types' and commit" && exit 1)

# cargo check for the wasm32 target (engine-side crates).
check-wasm:
    cargo check --target wasm32-unknown-unknown {{wasm_crates}}

# Ownership check for the current branch (node/<id>) against origin/main.
check-ownership:
    python3 scripts/check-ownership.py

# Everything CI checks, except tests.
check-all: check-wasm
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    pnpm -r typecheck
    pnpm -r lint

# All tests (Rust + vitest). Engine tests render offline and never need an audio device.
test-all:
    ETHER_AUDIO=null cargo test --workspace
    pnpm -r test

# Format Rust.
fmt:
    cargo fmt --all
