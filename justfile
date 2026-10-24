# Ethereal dev commands. Always use these (never hardcode ports/paths): every recipe runs
# with a per-instance ETHER_INSTANCE / ETHER_DEV_PORT (scripts/dev-env.mjs), so several
# worktrees/agents can run dev instances side by side. See README "Running multiple dev
# instances".

#
# Cargo features (docs/ARCHITECTURE.md "Cargo features"): the `dev-*` recipes build the
# fast dev core (no plugin hosts, sandbox or Signalsmith warp); `*-full` builds everything,
# like the releases. `check-all`, `test-all` and `check-features` build every feature.

set shell := ["bash", "-euo", "pipefail", "-c"]

wasm_crates := "-p ether-protocol -p ether-model -p ether-core -p ether-devices -p ether-media -p ether-controller -p ether-stretch -p ether-wasm"
env := 'eval "$(node scripts/dev-env.mjs)"'

default:
    @just --list

# Print this instance's id and base dev port.
dev-port:
    @{{env}}; echo "instance=$ETHER_INSTANCE port=$ETHER_DEV_PORT (preview=$((ETHER_DEV_PORT+1)) playwright=$((ETHER_DEV_PORT+2)) remote=$((ETHER_DEV_PORT+4)))"

# Install JS dependencies.
install:
    pnpm install --frozen-lockfile

# The full app (this checkout + ee/, which needs access to the private repository;
# scripts/full.mjs, docs/EE-WORKFLOW.md "Building the full app"): the desktop app (dev), its
# release bundle, the engine server, the web app. Extra args go to the underlying command.
#
# The full desktop app, dev build (every feature).
full-desktop *args:
    node scripts/full.mjs desktop {{args}}

# The full desktop app's release bundle.
full-build *args:
    node scripts/full.mjs build {{args}}

# The full engine server (every feature).
full-server *args:
    node scripts/full.mjs server {{args}}

# The full web app (dev server).
full-web *args:
    node scripts/full.mjs web {{args}}

# Standalone UI against MockTransport (no Rust needed).
dev-ui:
    {{env}}; echo "UI on http://localhost:$ETHER_DEV_PORT"; pnpm --filter @ethereal/ui dev

# Browser build host (COOP/COEP headers; the wasm engine).
dev-web:
    {{env}}; echo "web on http://localhost:$ETHER_DEV_PORT"; pnpm --filter @ethereal/web dev

# Tauri desktop app, fast dev core (per-instance identifier, data dir and devUrl;
# ETHER_AUDIO=null for no device). Extra args go to `tauri dev`, e.g.
# `just dev-desktop --features plugins` (features: docs/ARCHITECTURE.md "Cargo features";
# with `plugins`, build the helpers first: `node scripts/build-sandbox-helper.mjs`).
dev-desktop *args:
    {{env}}; export ETHER_WORKERS="${ETHER_WORKERS:-1}"; cd apps/desktop && pnpm tauri dev --config "{\"identifier\":\"dev.ethereal.$ETHER_INSTANCE\",\"build\":{\"devUrl\":\"http://localhost:$ETHER_DEV_PORT\",\"beforeDevCommand\":\"pnpm --filter @ethereal/ui dev\"}}" {{args}}

# Tauri desktop app with every feature (what releases ship). Builds the plugin scanner and
# sandbox helper first (sandboxed plugins; ETHER_SANDBOX_HELPER overrides it).
dev-desktop-full:
    node scripts/build-sandbox-helper.mjs; just dev-desktop --features full

# Desktop app with the null audio backend (no device, no contention).
dev-desktop-headless *args:
    ETHER_AUDIO=null just dev-desktop {{args}}

# Headless engine over WebSocket on this instance's remote port (base +4, loopback only).
# The token is printed (dev convenience); connect from the web UI's "Remote" button.
# Extra args go to ether-server (e.g. `just dev-server --listen 0.0.0.0`, see --help).
dev-server *args:
    {{env}}; ETHER_WORKERS="${ETHER_WORKERS:-1}" ETHER_AUDIO="${ETHER_AUDIO:-null}" cargo run -p ether-server -- --print-token {{args}}

# `dev-server` with every feature.
dev-server-full *args:
    {{env}}; ETHER_WORKERS="${ETHER_WORKERS:-1}" ETHER_AUDIO="${ETHER_AUDIO:-null}" cargo run -p ether-server --features full -- --print-token {{args}}

# MCP server for AI agents (docs/MCP.md): attaches to this instance's desktop app (enable
# "Allow AI agents (MCP)" there), or `just mcp --project song.ether` / `--server URL --token T`.
# stdout is the MCP channel: cargo stays quiet.
mcp *args:
    @{{env}}; ETHER_AUDIO="${ETHER_AUDIO:-null}" cargo run -q -p ether-mcp -- {{args}}

# Install the headless Chromium used by the Playwright e2e suite (once per machine).
e2e-install:
    cd apps/web && npx playwright install chromium-headless-shell

# Playwright e2e of the web build (real wasm engine + controller, headless Chromium, this
# instance's Playwright port). Extra args go to `playwright test` (e.g. `just e2e-web flow`).
e2e-web *args:
    {{env}}; cd apps/web && node scripts/build-wasm.mjs && npx playwright test {{args}}

# The guided tours walked through like a user, every tour and step, at three window sizes and
# on the mobile shell (opt-in, about a minute; docs/TUTORIALS.md "Tests"). Run it before a
# release and after UI refactors. Extra args go to `playwright test`.
e2e-tours *args:
    {{env}}; cd apps/web && node scripts/build-wasm.mjs && ETHEREAL_TOURS=1 npx playwright test --project=tours --workers=2 {{args}}

# Graph snapshot and IR swap benchmarks in real wasm (V8): builds ether-wasm with the `bench`
# feature (left out of release builds) and runs apps/web/src/engine/*.bench.test.ts.
# `just dev-web`/`e2e-web` rebuild the pkg without it.
bench-wasm:
    cd apps/web && ETHER_WASM_FEATURES="${ETHER_WASM_FEATURES:-full},bench" node scripts/build-wasm.mjs && npx vitest run src/engine/graphSnapshot.bench.test.ts src/engine/irSwap.bench.test.ts src/engine/stretch.bench.test.ts

# Native smoke test: the desktop host (null audio backend) driven through the full user flow.
e2e-native:
    ETHER_WORKERS="${ETHER_WORKERS:-1}" ETHER_AUDIO=null cargo test -p ether-native --test it -- e2e_flow:: null_host::

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

# Regenerate THIRD-PARTY-NOTICES.md (Rust lockfile + npm production deps; commits the result).
# Needs `pnpm install` and `cargo install cargo-bundle-licenses --version 4.2.0`.
notices:
    node scripts/gen-notices.mjs

# Fail if THIRD-PARTY-NOTICES.md is stale (CI).
check-notices-fresh: notices
    git diff --exit-code --stat -- THIRD-PARTY-NOTICES.md || (echo "THIRD-PARTY-NOTICES.md is stale: run 'just notices' and commit" && exit 1)
    test -z "$(git status --porcelain -- THIRD-PARTY-NOTICES.md)" || (echo "THIRD-PARTY-NOTICES.md is untracked: run 'just notices' and commit" && exit 1)

# Every dependency's license must be on the allow-list in deny.toml (CI).
# Needs `cargo install cargo-deny`.
check-licenses:
    cargo deny check licenses

# Security advisories, yanked crates, bans and dependency sources (deny.toml; CI).
# Needs `cargo install cargo-deny`.
check-advisories:
    cargo deny check advisories bans sources

# Fuzz a parser that takes untrusted bytes (fuzz/; nightly + `cargo install cargo-fuzz`):
# `just fuzz media_decode 300`; `just fuzz-all 60` runs every target (`node scripts/fuzz.mjs list`).
fuzz target seconds="60":
    node scripts/fuzz.mjs run {{target}} {{seconds}}

fuzz-all seconds="60":
    node scripts/fuzz.mjs all {{seconds}}

# cargo check for the wasm32 target (engine-side crates), with every feature and with none.
check-wasm:
    cargo check --target wasm32-unknown-unknown {{wasm_crates}} --all-features
    cargo check --target wasm32-unknown-unknown {{wasm_crates}} --no-default-features

# Neither feature set rots: the workspace with every feature and with no default features
# (clippy, all targets). Part of `check-all`.
check-features:
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo clippy --workspace --all-targets --no-default-features -- -D warnings

# Tracked paths that collide on case-insensitive file systems (macOS, Windows).
check-case-clashes:
    node scripts/check-case-clashes.mjs

# Everything CI checks, except tests. (check-notices-fresh and check-licenses need
# cargo-bundle-licenses / cargo-deny, see their recipes.)
check-all: check-case-clashes check-wasm check-features check-types-fresh check-notices-fresh check-licenses
    cargo fmt --all --check
    pnpm -r typecheck
    pnpm -r lint
    pnpm format:check

# All tests (Rust + vitest), every feature. Engine tests render offline and never need an
# audio device.
test-all:
    ETHER_WORKERS="${ETHER_WORKERS:-1}" ETHER_AUDIO=null cargo test --workspace --all-features
    pnpm -r test
    node --test scripts/release/*.test.mjs

# Format Rust (rustfmt) and TS/CSS (prettier, .prettierrc.json).
fmt:
    cargo fmt --all
    pnpm format

# The full-product CI's logs of one run, decrypted (.github/workflows/full.yml; needs `age` and
# an identity among ee/.sops.yaml's recipients): `just full-logs <run-id> [out-dir]`.
full-logs run *out:
    node scripts/ci/full-logs.mjs {{run}} {{out}}

# Paired OSS + EE branches and PRs through the `ee/` submodule (docs/EE-WORKFLOW.md):
# `just feat start x [--ee]`, `just feat push`, `just feat status`, `just feat ship x`, ...
[positional-arguments]
feat *args:
    node scripts/feat.mjs "$@"
