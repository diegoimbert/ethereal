# Releasing Ethereal

Releases are built and published by `.github/workflows/release.yml`, which (with the dev
deploy, `deploy-dev.yml`) lives in the private `ee/` repository: official binaries are the full
product, OSS main plus the `ee/` commit it points at (docs/EE-WORKFLOW.md). The GitHub Release
itself is still created in this repository. The rest of this page describes that workflow.

## When it runs

- **Manual only** (Actions → Release → Run workflow). A push or merge to `main` never
  publishes — `ci.yml` runs the checks there, and nothing else.
- On `main` the dispatch builds everything and publishes a GitHub Release (untick `publish`
  to build only). On any other branch it builds and uploads the files as workflow artifacts,
  and never publishes.
- PRs and pushes to `main` never trigger it.

Runs are serialized per branch (`concurrency: release-<ref>`) and never cancelled midway.

## Where releases are published

Releases are published on this repository
(**[`diegoimbert/ethereal`](https://github.com/diegoimbert/ethereal/releases)**): the
`publish` job creates the release with the tag, title, notes (with GitHub's generated
changelog), latest/prerelease flag and files. Users and the download page read it here.

- **Tags:** the tag is created on the default branch.
- **Re-runs:** the step is idempotent. Re-running the failed publish job keeps an existing
  release, updates its notes and flags, and replaces its files (`--clobber`).
- **Changelog links** in the notes point at this repository's PRs.

## Version and tag

The [`VERSION`](../VERSION) file (plain `X.Y.Z`) is the release version. The workflow
passes it to `tauri build` as the app version (bundle version, Info.plist, MSI), so the
Cargo versions don't need bumping. `tauri.conf.json`'s `version` (what a local `tauri build`
labels the app with) mirrors `VERSION`: bump both together
(`scripts/release/tauri-config.test.mjs` fails when they differ).

- If tag `v<VERSION>` doesn't exist yet, the release is `v<VERSION>`, marked as latest.
- If it exists, the release is `v<VERSION>+<shortsha>` and marked as a **prerelease**
  (so every push to `main` still produces downloadable builds).

To cut a new release: bump `VERSION` and `tauri.conf.json`'s `version` on `main` (land it
with a pull request). The same pull request moves the `Unreleased` summary in
[`CHANGELOG.md`](../CHANGELOG.md) under the new version and updates `version` and
`date-released` in [`CITATION.cff`](../CITATION.cff).

### Release notes

`docs/release-notes/<VERSION>.md` (optional) holds the hand-written, user-facing notes for
that version. The publish job puts it right under the release title, before the download
guide; GitHub's generated changelog follows. It is what the download page and the app's
"What's new" show (they drop the download and signing sections). Use `###` headings.

### Before a release (required)

CI runs these on every PR (the e2e and tours too, except the `@frames` and `@pixels` specs,
which only hold on the devbox); run them locally before a release anyway:

- `just check-all` and the test suites (`check-all` starts with `just check-case-clashes`:
  tracked paths or TS module names that differ only in case break macOS/Windows builds).
- `just e2e-web` (the default Playwright run).
- `just e2e-tours`: the guided tours walked through end to end (opt-in, about a minute;
  docs/TUTORIALS.md "Tests"). Tours break silently in UI refactors; a release ships only with
  every tour green.
- Third-party licenses are part of `check-all` too (`check-notices-fresh`, `check-licenses`).
  If a dependency changed, run `just notices` and commit the regenerated
  `THIRD-PARTY-NOTICES.md`; if `check-licenses` fails, a dependency brought in a license that
  is not on the `deny.toml` allow-list (see that file before widening it).

## What gets built

| Job | Runner | Output |
|---|---|---|
| macOS arm64 | `macos-latest` | `Ethereal_<v>_aarch64.dmg`, `Ethereal_<v>_aarch64.app.zip` |
| macOS x64 | `macos-latest` (cross, `x86_64-apple-darwin`) | `Ethereal_<v>_x64.dmg`, `Ethereal_<v>_x86_64.app.zip` |
| Linux x64 | `ubuntu-22.04` (glibc 2.35 baseline) | `Ethereal_<v>_amd64.AppImage`, `Ethereal_<v>_amd64.deb` |
| Windows x64 | `windows-latest` | `Ethereal_<v>_x64_en-US.msi`, `Ethereal_<v>_x64-setup.exe` |
| Web | `ubuntu-latest` | `Ethereal_<v>_web.zip` (static site + `_headers` + `DEPLOY.txt` + `LICENSE` + `THIRD-PARTY-NOTICES.md`) |

Each release also carries `SHA256SUMS.txt`, and, when the
updater key is configured, the updater bundles, their `.sig` files and `latest.json`
(see Auto-update). The notes start
with a download guide and each platform's signing status, followed by GitHub's generated
changelog. `LICENSE` and `THIRD-PARTY-NOTICES.md` are also copied next to the installers and
bundled into each app (`tauri.conf.json` `bundle.licenseFile` / `files` and the
`.deb`/`.rpm`/AppImage `/usr/share/doc/ethereal/`), so every artifact carries the license
(the AGPL) and the third-party license texts.

- **Sandbox helper.** `ether-sandbox-helper` is built and staged by `tauri.conf.json`'s
  `beforeBuildCommand` (`scripts/build-sandbox-helper.mjs --bundle`, which follows Tauri's
  `--target`), and installed next to the app binary (`Contents/MacOS/`, `/usr/bin/`).
  `scripts/release/collect.mjs` fails the job if the `.app` or `.deb` doesn't contain it.
  Windows has no sandbox yet: the script is a no-op there and the app builds without it.
- **Matrix.** `fail-fast: false`, so one failing platform doesn't cancel the others.
  Publishing requires all jobs to succeed, unless a manual run sets `allow_partial`
  (then it publishes whatever succeeded as a prerelease, and the notes say what's missing).
- **Caching.** `Swatinem/rust-cache` (one key per target) and the pnpm store via
  `actions/setup-node`.
- **Release overlay.** `scripts/release/tauri-config.mjs` writes a config merged over
  `tauri.conf.json` with `--config`: the version and signing settings. The base config stays release-agnostic.

### Bundle identifier

The desktop app's identifier is `ws.ethereal.daw` (`identifier` in `tauri.conf.json`,
`ether_native::app_identity::DESKTOP_IDENTIFIER`; a test keeps the two equal). It is the
reverse-DNS of ethereal.ws plus a third segment: Flathub requires at least three, and Tauri
warns about an identifier that ends in `.app`. It names the app's data, log and WebView
storage directories, so **it must not change again**.

Releases up to v0.6.0 used `dev.ethereal.app`. On the first launch of a newer release,
before anything opens them, `ether_native::app_identity::migrate_on_startup` moves those
directories to the new names: the app data dir (settings, plugin database, caches,
autosaves), the logs, and the WebView storage (macOS `~/Library/{Application Support,
Caches,Logs,WebKit,HTTPStorages}`, Windows `%APPDATA%` and `%LOCALAPPDATA%`, Linux
`$XDG_DATA_HOME`, `$XDG_CONFIG_HOME` and `$XDG_CACHE_HOME`). Nothing is deleted: when both
names exist, entries missing from the new directory are moved, and entries present in both
are left in the old directory, with a warning in the log. Projects are in
`~/Documents/Ethereal/Projects`, which does not depend on the identifier. macOS ties privacy
permissions (microphone, screen recording) to the identifier, so it asks for them again once.
Say so in the release notes.

## Signing (optional)

Without secrets, everything builds **unsigned** and the release notes say so. macOS builds
then get an ad-hoc signature (`signingIdentity: "-"`), so they launch on Apple Silicon after
the quarantine prompt. The sandbox helper is always signed first
(`scripts/release/sign-macos-helper.mjs`, ad-hoc here): codesign refuses to sign the `.app`
around an unsigned nested binary. The linker only signs arm64 binaries, so without this the
x86_64 build fails. With the secrets below, the workflow signs (and, on macOS,
notarizes) automatically. Add them under Settings → Secrets and variables → Actions.

### macOS (Developer ID)

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | base64 of the exported "Developer ID Application" `.p12` (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | the `.p12` export password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Jane Doe (TEAMID1234)` (`security find-identity -v -p codesigning`) |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_PASSWORD` | an **app-specific password** for that Apple ID (appleid.apple.com) |
| `APPLE_TEAM_ID` | the 10-character team id |

The first three sign; all six sign and notarize. The workflow imports the certificate
into a temporary keychain. `scripts/release/sign-macos-helper.mjs` then signs the staged
`ether-sandbox-helper`, because Tauri doesn't sign `bundle.macOS.files`: it runs from the
overlay's `beforeBuildCommand`. Tauri signs the app and notarizes and staples it. Both use
the hardened runtime with
[`scripts/release/macos/entitlements.plist`](../scripts/release/macos/entitlements.plist):
`disable-library-validation` (to load third-party CLAP plugins) and `device.audio-input`
(for recording). Empty or missing secrets are never exported, because Tauri treats a
set-but-empty variable as configured.

### Windows (Authenticode)

| Secret | Value |
|---|---|
| `WINDOWS_CERTIFICATE` | base64 of the code-signing `.pfx` (`[Convert]::ToBase64String([IO.File]::ReadAllBytes("cert.pfx"))`) |
| `WINDOWS_CERTIFICATE_PASSWORD` | the `.pfx` password |
| `WINDOWS_TIMESTAMP_URL` | optional, default `http://timestamp.digicert.com` |

The default timestamp server is plain HTTP on purpose: an RFC 3161 timestamp is signed by the
authority and verified against its certificate chain, so the transport adds nothing (DigiCert's
authority does not answer over HTTPS).

The workflow imports the `.pfx` into the runner's user certificate store and passes its
thumbprint to Tauri (`bundle.windows.certificateThumbprint`, SHA-256, timestamped), which
signs the executable, the MSI and the NSIS installer. Certificates that live only in an
HSM or cloud signing service (EV certificates issued since 2023) need a custom
`bundle.windows.signCommand` instead; that isn't wired up yet.

### Linux

AppImage and `.deb` are not signed; `SHA256SUMS.txt` is the integrity check.

## Auto-update

The self-updater (`tauri-plugin-updater`) is **not part of this open-source build**: it is
part of the private overlay, so the official binaries update themselves from this
repository's GitHub releases while a source build has no updater. This section is kept for
the private build (see `ethereal-private`).

When it is enabled (`full` in the private build), the app reads
`https://github.com/diegoimbert/ethereal/releases/latest/download/latest.json`, so only the
newest stable release is offered; prereleases (`v<VERSION>+<sha>`, partial releases) never
are. Every download is verified against the minisign public key before it is installed:
`plugins.updater.pubkey`, which the private overlay adds to the Tauri config (this
repository's `apps/desktop/src-tauri/tauri.conf.json` has no updater section).

### How a release gets updater files

With the `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` secrets set:

1. The desktop jobs export them, and `scripts/release/tauri-config.mjs` sets
   `bundle.createUpdaterArtifacts: true`. Tauri then signs each platform's updater bundle:
   macOS `Ethereal.app.tar.gz`, Linux `*.AppImage` (the AppImage itself), Windows
   `*-setup.exe` and `*.msi`. Each signature is written to a `.sig` file next to its bundle.
2. `scripts/release/collect.mjs` collects the bundles and their `.sig` files. The macOS
   bundle is renamed `Ethereal_<v>_<arch>.app.tar.gz`, because both macOS builds produce
   the same file name.
3. After `gh release create`, the publish job runs `scripts/release/updater-manifest.mjs`.
   It builds `latest.json` from the assets and the published notes, with download URLs on
   `diegoimbert/ethereal`, and uploads it to the release. The manifest contains the version, the notes (without the download and signing
   sections), `pub_date`, and these platform entries: `darwin-aarch64`, `darwin-x86_64`,
   `linux-x86_64` (+ `-appimage`), `windows-x86_64` (+ `-nsis`, both NSIS) and
   `windows-x86_64-msi`.

**Without the secrets** the build is exactly as before: no updater bundles and no
`latest.json`, with a notice in the run. A missing key never fails a release, and a missing
platform or signature only leaves that platform out of the manifest, with a warning.
Installed apps treat a release without `latest.json` as "no update information" and stay
on their current version.

Apps up to v0.6.0 read a repository that no longer has the releases (`diegoimbert/ethereal-public`
for v0.6.0), so their checks always 404: they never see an update and need one manual install
of the next version (say so in its notes). From then on they update from this repository.

### Keys

| Secret | Value |
|---|---|
| `TAURI_SIGNING_PRIVATE_KEY` | the content of the private key file (`updater.key`, one base64 line) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | its password |

```sh
gh secret set TAURI_SIGNING_PRIVATE_KEY < ~/.config/ethereal/updater.key
gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD < ~/.config/ethereal/updater.key.password
```

The keypair was made with `pnpm tauri signer generate -w ~/.config/ethereal/updater.key`.
Only the public key is in the repository.

> **Keep a backup of the private key and its password.** Apps only accept updates signed
> with the key whose public half they ship. If the key is lost, a new keypair means
> everyone must download and install the next version by hand once; after that, updates
> work again.

### What updates where

| Install | Self-update | How |
|---|---|---|
| macOS `.app` (from the dmg or zip) | yes | the `.app` bundle is replaced, then relaunched |
| Linux AppImage | yes | the AppImage file is replaced (it must be writable), then relaunched |
| Linux `.deb` | no | "Download" opens the release page (the package manager owns `/usr/bin`) |
| Windows NSIS (`-setup.exe`) | yes | the installer runs in passive mode, then relaunches the app |
| Windows MSI | yes | the MSI runs (`windows-x86_64-msi` entry) |
| Web | n/a | always the latest version; no updater UI |

The kind of install is detected at runtime: `APPIMAGE` set means AppImage, otherwise the
bundle type Tauri records in the binary.

Unsigned macOS builds have an ad-hoc signature (see Signing above). The updater replaces
the `.app` in place. A copy that already runs keeps running after the swap, and macOS
Gatekeeper doesn't quarantine files the app downloads itself (no quarantine attribute), so
the update should launch without a new prompt. This hasn't been verified on a Mac yet. If the app sits in a folder the user can't write to, macOS asks for
an administrator password.

### In the app

At startup (after 10 s) and every 6 hours, the app checks for an update, as long as
**Settings > General > Updates > Check for updates automatically** is on (the default).
**Check for Updates…** is in the project menu and in the command palette. A new version
shows a card with these buttons:

- **What's new**: the release notes.
- **Install and restart**: download with progress, verify, then restart. If the project
  has unsaved changes, it asks Save / Don't save / Cancel first.
- **Later**.
- **Skip this version**: remembered. Automatic checks stay quiet about that version; a
  manual check still shows it.

Errors from automatic checks show only in Settings. Errors from manual checks show as a
quiet toast. The preferences live in `<data dir>/updater.json`.

### Testing locally

Debug builds read `ETHER_UPDATER_ENDPOINT` in place of the GitHub URL. A plain `http://`
endpoint also needs `plugins.updater.dangerousInsecureTransportProtocol: true` in a
`--config` overlay. To test an update end to end:

1. Build a signed debug AppImage at a low version:

   ```sh
   TAURI_SIGNING_PRIVATE_KEY="$(cat ~/.config/ethereal/updater.key)" \
   TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat ~/.config/ethereal/updater.key.password)" \
   pnpm tauri build --debug --features updater --bundles appimage \
     --config '{"version":"0.0.9","bundle":{"createUpdaterArtifacts":true},"plugins":{"updater":{"dangerousInsecureTransportProtocol":true}}}'
   ```

2. Build a second one at a higher version the same way.
3. Run `updater-manifest.mjs` over the second build (or write `latest.json` by hand) with
   its URL pointing at a local `python3 -m http.server`.
4. Start the first AppImage with `ETHER_UPDATER_ENDPOINT=http://127.0.0.1:<port>/latest.json`.

The release workflow never sets that variable, and release builds don't read it.

## Validating changes to the workflow

- Lint: `actionlint .github/workflows/release.yml` (with `shellcheck` on `PATH`).
- Local macOS build, same as CI:

  ```sh
  node scripts/release/tauri-config.mjs /tmp/tauri.release.json
  (cd apps/desktop && pnpm tauri build --ci --target aarch64-apple-darwin --bundles app,dmg --config /tmp/tauri.release.json)
  node scripts/release/collect.mjs desktop aarch64-apple-darwin /tmp/release-out
  ```

- Dry run of the publishing: dispatch with `dry_run` on (any branch,
  `gh workflow run release.yml --ref <branch> -f dry_run=true`). It builds everything, runs
  the publish job (assets, notes, `latest.json` with download URLs, printed in the log), but
  every write (release create/edit/upload) is only printed, and Pages is not deployed.
- Real CI without publishing: dispatch the workflow on a non-`main` branch
  (`gh workflow run release.yml --ref <branch>`; GitHub only lists dispatchable workflows
  that exist on the default branch). It builds all platforms and uploads artifacts, and the
  publish job is skipped. Before the workflow is on the default branch, temporarily add a
  `push: branches: [release-dry-run]` trigger and push that branch instead. Either way it
  uses paid runner minutes (macOS runners cost the most), so do it sparingly.

## Web app on Cloudflare Pages

Every push to `main` also deploys the web build (`apps/web/dist` plus the COOP/COEP
`_headers` from `scripts/release/web/`) to Cloudflare Pages (job `pages` in
`.github/workflows/release.yml`, `wrangler` pinned by version). The deploy runs even when
another platform's build failed, as long as the web bundle built. It is skipped (with a
warning), not failed, until these are configured in the GitHub repository settings:

- Secret `CLOUDFLARE_API_TOKEN`: an API token with **Account > Cloudflare Pages > Edit**
  (Cloudflare dashboard > My Profile > API Tokens > Create Token > custom token).
- Secret `CLOUDFLARE_ACCOUNT_ID`: your account id (dashboard sidebar, or Workers & Pages overview).
- Optional variable `CLOUDFLARE_PAGES_PROJECT` (default `ethereal`): the Pages project name. The
  job creates the project (production branch `main`) on first deploy if it doesn't exist.

The site is served at `https://<project>.pages.dev` (a custom domain can be attached in the
Pages dashboard). The COOP/COEP headers are required: the web engine uses SharedArrayBuffer.

### dev.ethereal.ws

A scheduled workflow checks the default branch (`main`) every 15 minutes and, when its tip
moved, deploys the web build alone to `https://dev.ethereal.ws`
(`.github/workflows/deploy-dev.yml`, same secrets; the build steps are the composite action
`.github/actions/web-build`, shared with the release's `web` job). Its Pages project is
`ethereal-dev` (Cloudflare Pages production branch `dev`), with the same Functions bindings
as the release project (`STATIC_SHARE` → `ethereal-static-share`, `ROOMS` → the signaling
durable object), so the dev site shares the production share storage and signaling. The app
shows its version as `<VERSION>-dev.<short sha>`. Runs are serialized: a newer run waits for
the one in progress.

## Download page

The download page (`https://ethereal.ws/download`, the `ui/src/features/download` page and
the release worker it reads through) is **part of the private overlay**
(`ethereal-private`). In the OSS build there is no download page; the Releases page on this
repository is the source of truth for downloads.
