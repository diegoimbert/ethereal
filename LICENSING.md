# Licensing

## Open-source license

Ethereal is free and open-source software, licensed under the **GNU Affero General Public
License, version 3 or later** (`AGPL-3.0-or-later`). The full text is in [LICENSE](LICENSE);
see also <https://www.gnu.org/licenses/agpl-3.0.html>.

Copyright (C) 2026 Diego Imbert and the Ethereal contributors.

The AGPL means you are free to use, study, modify and share Ethereal, provided that if you
run a modified version on a network server and let others interact with it, you offer those
users the corresponding source code (AGPL section 13). That clause is the reason the license
was chosen: the hosted version at [ethereal.ws](https://ethereal.ws) is a network service, and
the license keeps that service open too.

## Commercial and alternative licenses

If the AGPL does not work for your use — for example you want to embed Ethereal, ship it as
part of a closed-source product, or run a modified version as a service without the source
obligations — the copyright holder can grant you a **separate license under other terms,
including commercial terms**. Contact [@diegoimbert](https://github.com/diegoimbert) to
arrange one.

This is possible because the copyright holder is not limited to the AGPL: the AGPL is the
license granted to the public, not a restriction on the owner. The project can be (and is
intended to be) **dual-licensed** — AGPL-3.0-or-later for the community, and other terms for
those who need them — and the owner can **relicense** the project in the future.

## Contributions

To keep dual licensing and relicensing possible, contributions must come with the rights to
do so. Every contributor therefore signs the [Contributor License Agreement](CLA.md) in their
first pull request. You keep the copyright to your contribution; the CLA grants the
copyright holder the broad, irrevocable copyright and patent permissions needed to
distribute the project under the AGPL *and* under other terms.

The CLA is enforced automatically: a bot comments on new pull requests and the `CLA` check
must pass before a pull request can be merged (see [CLA.md](CLA.md#for-maintainers) for the
one-time branch-protection setup).

## Third-party components

Ethereal is built with third-party open-source software. Every component that ships in a
release, and the license text it requires, is listed in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) (regenerate it with `just notices`). The
licenses of those components apply to them; the AGPL above applies to Ethereal's own code.
All current dependencies are permissive (plus `symphonia`, MPL-2.0); the `deny.toml` allow-list
keeps it that way (`just check-licenses`).
