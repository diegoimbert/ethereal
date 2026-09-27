# Releasing Ethereal

Releases are built and published by [`.github/workflows/release.yml`](../.github/workflows/release.yml).

## When it runs

- **Push to `main`**: builds everything and publishes a GitHub Release. `main` only moves
  when `dev` is promoted, so every push to `main` is a release.
- **Manual** (Actions → Release → Run workflow): on `main`, same as a push (untick
  `publish` to build only). On any other branch it builds and uploads the files as workflow
  artifacts, and never publishes.
- `dev`, `node/*` and PR branches never trigger it. `ci.yml` is separate and stays disabled.

Runs are serialized per branch (`concurrency: release-<ref>`) and never cancelled midway.

## Version and tag

The [`VERSION`](../VERSION) file (plain `X.Y.Z`) is the release version. The workflow
passes it to `tauri build` as the app version (bundle version, Info.plist, MSI), so
`tauri.conf.json` and the Cargo versions don't need bumping.

- If tag `v<VERSION>` doesn't exist yet, the release is `v<VERSION>`, marked as latest.
- If it exists, the release is `v<VERSION>+<shortsha>` and marked as a **prerelease**
  (so every push to `main` still produces downloadable builds).

To cut a new release: bump `VERSION` on `dev` and promote `dev` to `main`.

## What gets built

| Job | Runner | Output |
|---|---|---|
| macOS arm64 | `macos-latest` | `Ethereal_<v>_aarch64.dmg`, `Ethereal_<v>_aarch64.app.zip` |
| macOS x64 | `macos-latest` (cross, `x86_64-apple-darwin`) | `Ethereal_<v>_x64.dmg`, `Ethereal_<v>_x86_64.app.zip` |
| Linux x64 | `ubuntu-22.04` (glibc 2.35 baseline) | `Ethereal_<v>_amd64.AppImage`, `Ethereal_<v>_amd64.deb` |
| Windows x64 | `windows-latest` | `Ethereal_<v>_x64_en-US.msi`, `Ethereal_<v>_x64-setup.exe` |
| Web | `ubuntu-latest` | `Ethereal_<v>_web.zip` (static site + `_headers` + `DEPLOY.txt`) |

Each release also carries `SHA256SUMS.txt` and `THIRD_PARTY_NOTICES.txt`. The notes start
with a download guide and each platform's signing status, followed by GitHub's generated
changelog.

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
  `tauri.conf.json` with `--config`: the version, the bundled `THIRD_PARTY_NOTICES.txt` and
  `LICENSE.txt` resources, and signing settings. The base config stays release-agnostic.

## Signing (optional)

Without secrets, everything builds **unsigned** and the release notes say so. macOS builds
then get an ad-hoc signature (`signingIdentity: "-"`), so they launch on Apple Silicon after
the quarantine prompt. With the secrets below, the workflow signs (and, on macOS,
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

The workflow imports the `.pfx` into the runner's user certificate store and passes its
thumbprint to Tauri (`bundle.windows.certificateThumbprint`, SHA-256, timestamped), which
signs the executable, the MSI and the NSIS installer. Certificates that live only in an
HSM or cloud signing service (EV certificates issued since 2023) need a custom
`bundle.windows.signCommand` instead; that isn't wired up yet.

### Linux

AppImage and `.deb` are not signed; `SHA256SUMS.txt` is the integrity check.

## Third-party notices

`node scripts/release/third-party-notices.mjs` writes `THIRD_PARTY_NOTICES.txt` from
`cargo metadata` (every non-workspace crate reachable through normal/build dependencies,
all platforms) and `pnpm licenses list --prod`, with the license files found in each
package. Identical texts are printed once and referenced as `[T<n>]`. The MIT notices of
Signalsmith Stretch and Signalsmith Linear (the C++ code vendored in the
`signalsmith-stretch` crate) and of the binding itself are listed first, and the script
fails if they can't be found. `--check` exits 1 if the committed file is stale.

The release regenerates the file, bundles it into the desktop app (as a Tauri resource)
and the web zip, and attaches it to the release. Refresh the committed copy after
dependency changes. Some crates don't ship a license file in their package; for those the
file gives the SPDX id and the source URL.

## Validating changes to the workflow

- Lint: `actionlint .github/workflows/release.yml` (with `shellcheck` on `PATH`).
- Local macOS build, same as CI:

  ```sh
  node scripts/release/third-party-notices.mjs
  node scripts/release/tauri-config.mjs /tmp/tauri.release.json
  (cd apps/desktop && pnpm tauri build --ci --target aarch64-apple-darwin --bundles app,dmg --config /tmp/tauri.release.json)
  node scripts/release/collect.mjs desktop aarch64-apple-darwin /tmp/release-out
  ```

- Real CI without publishing: dispatch the workflow on a non-`main` branch
  (`gh workflow run release.yml --ref <branch>`; GitHub only lists dispatchable workflows
  that exist on the default branch). It builds all platforms and uploads artifacts, and the
  publish job is skipped. Before the workflow is on the default branch, temporarily add a
  `push: branches: [release-dry-run]` trigger and push that branch instead. Either way it
  uses paid runner minutes (macOS runners cost the most), so do it sparingly.
