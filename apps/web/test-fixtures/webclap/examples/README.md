# Example WebCLAPs (test fixtures)

Third-party WebCLAP builds the host's tests use (`src/engine/webclap/*.test.ts`,
`e2e/webclap.spec.ts`). They don't ship with the app. They are copied unmodified from
[WebCLAP/examples](https://github.com/WebCLAP/examples) (2026-01-29):

| File | Source | License |
|---|---|---|
| `clack/clack_plugin_gain.wasm` | the gain example of [Clack](https://github.com/prokopyl/clack) (Rust) | MIT OR Apache-2.0 (`clack/LICENSE-*.txt`) |
| `signalsmith-basics/basics.wclap.tar.gz` | [Signalsmith Basics](https://github.com/Signalsmith-Audio/basics) with web UIs (C++, wasi-sdk; a bundle that imports shared memory) | MIT (`signalsmith-basics/LICENSE.txt`) |
