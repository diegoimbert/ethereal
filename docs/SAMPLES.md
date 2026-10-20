# Sample packs

The browser's samples section, the drum rack's kit picker and the sampler's picker draw from
two sources:

- **The built-in kit** (pack `builtin`): a kick, snare and closed hi-hat synthesized in code
  (`crates/ether-controller/src/sample_pack/fallback.rs`). It is always available, offline
  included, and is never downloaded.
- **The default pack**: a signed, versioned manifest downloaded on first use, whose files are
  fetched lazily and cached on the engine side (the app data folder on desktop, OPFS on the
  web). Until a pack is published the app shows the built-in kit and "more samples coming
  soon".

The protocol is `ether_protocol::samples` (`Samples::Open`, `Samples::Fetch`,
`SamplesEvent`); the controller side is `crates/ether-controller/src/sample_pack/`.

## Source

`Samples::Open { url }` picks the manifest URL: the command's, else the host's override
(`ETHER_SAMPLES_URL` on desktop), else
`https://samples.ethereal.ws/packs/default/manifest.json`. An empty string means "no remote
pack". The URL must be `https`, or a loopback address for local testing.

The cached manifest (verified again on load) is used at once, then a refresh is fetched:

- a newer `version` replaces the cached manifest; files it no longer lists are deleted unless
  a project references them;
- an older `version` is refused (no rollback);
- a 404 with nothing cached shows "coming soon"; a network failure shows the pack offline, or
  the cached pack marked offline.

## Manifest

The URL serves an envelope `{"manifest": "<json text>", "signature": "<base64>"}`. The
signature is Ed25519 over `ethereal sample pack v1\n` followed by the manifest text, so a
signature from another signed manifest cannot stand in for it. Manifests above 4 MiB are
refused before parsing. Manifests served from a loopback address may also be signed with the
public dev key (`manifest::DEV_SEED`), which the tests use.

The manifest text (schema 1):

```json
{
  "schema": 1,
  "id": "default",
  "version": 3,
  "name": "ethereal default pack",
  "generated_at": "2026-01-01T00:00:00Z",
  "files": [
    {
      "id": "drums/909/kick",
      "name": "kick",
      "path": "files/<sha256>.wav",
      "sha256": "<hex>",
      "size": 123456,
      "sample_rate": 48000,
      "channels": 1,
      "frames": 24000,
      "category": "drum",
      "license": "cc0",
      "tags": ["909"],
      "bpm": null,
      "key": null
    }
  ],
  "kits": [{ "id": "909", "name": "909", "pads": [{ "note": 36, "sample": "drums/909/kick" }] }],
  "instruments": [
    { "id": "piano", "name": "piano", "zones": [{ "sample": "keys/piano/c3", "root": 60 }] }
  ]
}
```

- File `path`s are relative to the manifest URL and content-addressed. A file is at most
  64 MiB; each download is capped at its `size` and checked against its `sha256` before it is
  cached. At most four downloads run at once.
- Kit pads are MIDI notes (36 = C1, the first pad), with an optional `name` and `choke`
  group. Instrument zones take `root` plus optional `lo_key`/`hi_key` and `lo_vel`/`hi_vel`
  (0-127 by default).
- `category` is `drum`, `one_shot`, `loop` or `instrument`; `license` is `cc0` or `own`.
- Sample, kit and instrument ids are qualified `<pack>/<id>` in the app and stay stable across
  versions: projects reference pack samples by id and content hash, never by URL.

## Cache

```text
files/<sha256>.<ext>      downloaded files (shared by every source)
<source>/manifest.json    the signed envelope as downloaded, per source URL
sources.json              the source folders seen
pins.json                 sha256s that projects reference: kept across updates
```

## Building a pack

The `sample_pack` example builds the files and the unsigned manifest of a pack:

```sh
cargo run -p ether-controller --example sample_pack -- build <folder> <out> [--version N]
cargo run -p ether-controller --example sample_pack -- dev <out>
```

`build` reads `<folder>/samples.json`, checks every file (WAV or FLAC, 44.1-96 kHz, mono or
stereo, a license, length limits), measures loudness, and writes `<out>/files/`,
`<out>/manifest.json` and `<out>/report.md`. `dev` writes the synthesized test pack, signed
with the dev key, as it would be served. Signing and publishing the default pack is done by
the maintainers.
