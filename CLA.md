# Contributor License Agreement

Thank you for your interest in Ethereal. This Contributor License Agreement ("Agreement")
documents the rights you grant to **Diego Imbert** ("the Maintainer", "we", "us") when you
contribute to the Ethereal project (the repository at
<https://github.com/diegoimbert/ethereal>, and any successor, "the Project").

It protects you as a contributor, and it protects the Maintainer and everyone who uses
Ethereal. It does **not** change your rights to use your own contributions for any other
purpose: **you keep the copyright to your contribution.** The Agreement only grants the
permissions the Maintainer needs to distribute Ethereal under the AGPL *and*, separately,
under other terms — see [Why this Agreement](#why-this-agreement) and
[LICENSING.md](LICENSING.md).

By commenting the signing phrase on a pull request (see [How to sign](#how-to-sign)), or by
otherwise submitting a contribution to the Project, you accept and agree to the following
terms for your present and future contributions.

## 1. Definitions

- **"You"** means the individual who submits a Contribution, and, if the Contribution is
  submitted in the course of your employment or on behalf of a legal entity, also that legal
  entity.
- **"Contribution"** means any work of authorship — including source code, documentation,
  configuration, assets and any modifications or additions to existing work — that you
  intentionally submit to the Maintainer for inclusion in, or for use with, the Project,
  including anything submitted by email, through a code-review system (for example a pull
  request), or through any other means, excluding anything you clearly mark in writing as
  "Not a Contribution".
- **"Project"** means the Ethereal software and documentation.

## 2. Copyright license

You grant the Maintainer a **perpetual, worldwide, non-exclusive, royalty-free, irrevocable**
copyright license, with the right to **sublicense and transfer**, to reproduce, prepare
derivative works of, publicly display, publicly perform, distribute and otherwise use and
exploit your Contributions and works based on them, in source and object form, **under any
license terms**, including terms that differ from the AGPL and that may be proprietary or
commercial, and including the right to **relicense, sell, license and otherwise distribute**
the Project (with your Contributions) under any terms the Maintainer chooses.

To keep the community grant intact, you also grant every recipient of the Project a license
to your Contributions under the **GNU Affero General Public License, version 3 or later**
(`AGPL-3.0-or-later`; see [LICENSE](LICENSE)). That grant to recipients is in addition to the
broad grant above and does not reduce it.

## 3. Patent license

You grant the Maintainer and every recipient of the Project a perpetual, worldwide,
non-exclusive, royalty-free, irrevocable patent license to make, have made, use, offer to
sell, sell, import and otherwise transfer the Project, where the license applies only to
patent claims that you own or control that are necessarily infringed by your Contributions
alone or by their combination with the Project. If a third party brings a patent claim
against you or another contributor alleging that the Project or a Contribution infringes a
patent, the patent licenses you granted under this Agreement for that Project terminate as of
the date the claim is filed.

## 4. Your representations

You represent and warrant that:

1. you are legally entitled to grant the licenses above;
2. each Contribution is your original creation, or you have identified in the Contribution
   (or in a pull request) any third-party material and its license, and you have the right to
   submit it under this Agreement;
3. your Contribution does not violate any third party's copyright, patent, trademark, trade
   secret or other rights;
4. if your employer (or another entity) has rights to intellectual property you create, you
   have either received permission to make the Contribution on its behalf, or the rights to
   the Contribution have been waived for the Maintainer, or that entity has signed a separate
   agreement with the Maintainer; and
5. you will notify the Maintainer of any facts or circumstances of which you become aware
   that would make these representations inaccurate.

If you are submitting on behalf of an entity, you represent that you are authorized to bind
that entity.

## 5. No obligation; no support

You understand that the Maintainer is not obligated to use, merge, publish or maintain any
Contribution. You are not expected to provide support for your Contributions, except to the
extent you choose to. Unless required by applicable law or agreed in writing, you provide
your Contributions **"AS IS", without warranties or conditions of any kind**, express or
implied.

## 6. Moral rights

To the extent permitted by applicable law, you waive, and agree not to assert against the
Maintainer, its licensees or sublicensees, any moral rights (or rights of a similar nature,
such as rights of attribution or integrity) you may have in your Contributions, so that the
Maintainer can exercise the licenses granted here — including relicensing under different
terms. Where such rights cannot be waived, you agree not to exercise them in a way that
interferes with the Maintainer's exercise of the licenses granted here.

## 7. Miscellaneous

This Agreement is governed by the laws of the Maintainer's jurisdiction of residence,
excluding its conflict-of-law rules. If a provision is held unenforceable, the rest remains
in effect. This Agreement is the entire agreement about its subject and supersedes prior
discussions; it may be assigned by the Maintainer together with the Project. Your signature
stays valid for all your Contributions unless you tell the Maintainer in writing that you
revoke it, which does not affect grants already made.

## Why this Agreement

Ethereal is open source under the AGPL-3.0-or-later. The Maintainer also offers Ethereal
under other, including commercial, license terms (see [LICENSING.md](LICENSING.md)). Offering
those alternative licenses is only possible if the Maintainer holds, or can grant, enough
rights over every part of the code. This Agreement is what makes that possible while leaving
your copyright with you. It is the same model used by many dual-licensed open-source
projects.

## How to sign

Open (or update) a pull request. A bot comments once the pull request needs a signature.
Reply on the pull request with exactly:

> I have read the CLA Document and I hereby sign the CLA

The bot records your GitHub account on the `cla-signatures` branch of this repository and
marks the pull request's `CLA` check as passed; you only need to sign once. If the check does
not update, comment `recheck`. Signatures are recorded against the version of this document
(`CLA_VERSION`, currently **1**); if this document changes materially, that version is bumped
and existing signers are asked to sign again.

## For maintainers

- **Order matters.** Land `.github/workflows/cla.yml` on the default branch (`main`) *first* —
  the workflow runs from the base branch, so until it is on `main` no `CLA` status is ever
  reported. If you require the check before that, every pull request is stuck on
  "Expected — Waiting for status to be reported".
- Then make `CLA` a **required** status check on the default branch, so a pull request cannot
  merge until it is green. Setting branch protection is a `PUT` (a `PATCH` returns 404):

  ```sh
  gh api -X PUT repos/diegoimbert/ethereal/branches/main/protection \
    -H "Accept: application/vnd.github+json" \
    --input - <<'JSON'
  {
    "required_status_checks": { "strict": false, "contexts": ["CLA"] },
    "enforce_admins": false,
    "required_pull_request_reviews": null,
    "restrictions": null
  }
  JSON
  ```

  This *replaces* the branch's protection; on a branch that has none it just creates it. (If
  you later add other required checks, add them to `contexts`.) Use `"strict": true` only if
  you also want branches to be up to date with `main` before merging.
- The automation lives in `.github/workflows/cla.yml` and `.github/scripts/cla.mjs`. It uses
  only the workflow token (`contents`, `pull-requests` and `statuses` write) and never runs
  pull-request code. Signatures live in `cla-signatures.json` on the `cla-signatures` branch.
- The Maintainer's own account and bots are exempt (`CLA_ALLOWLIST` in the workflow).

---

*This document is a template for contributors and is not legal advice; have it reviewed by a
qualified lawyer before relying on it.*
