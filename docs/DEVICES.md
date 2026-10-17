# Writing a built-in device

The rules for an instrument or effect in `crates/ether-devices` (synths, samplers, effects,
MIDI effects, racks). The contracts they implement are in [CONTRACTS.md](CONTRACTS.md) §12;
the real-time rules in [ARCHITECTURE.md](ARCHITECTURE.md).

1. **Module.** `crates/ether-devices/src/<group>/` (split into files freely). The param
   table in `descriptor()` is a contract: ids are dense and **append-only** (never
   renumber; rename only before a release). Use the named ids in `<group>::<device>::*`.
2. **Steps.** Integer or stepped params carry `ParamInfo::step` (plain step; 1 for enums,
   toggles, transposes in semitones, keys, voices, counts; `contract::{choice, toggle,
   stepped}` set it, continuous `param(..)` + `.with_step(1.0)` otherwise). Knobs, automation
   lanes, MIDI learn and the renderer snap to it; `tests/v02_descriptors.rs` fails when a
   semitone/count param ships without one.
3. **Real-time rules**: no allocation, locks, I/O or unbounded work in
   `process`/`reset`/`set_data`/`analysis`; allocate in `new`/`prepare`. Apply
   `EventKind::Param` at its sample offset (`util::split_at_events`) and smooth continuous
   params. Write `assert_no_alloc` tests like `tests/devices2.rs` (`tests/<group>*.rs`,
   global `AllocDisabler`), plus render tests (impulse/sine, known outputs, extreme params,
   NaN-free).
4. **Latency** (lookahead, oversampling): report it with `Node::latency` (PDC).
5. **Sidechain**, for a device with a detector: declare `sidechain_inputs = 2` and key the
   detector from `Node::process_sidechain` like `compressor.rs`/`limiter.rs`. The sidechain
   arrives latency-aligned (CONTRACTS.md §11.10); without a source, `process` keys from the
   input.
6. **Descriptor ↔ mock parity.** After any descriptor change run
   `UPDATE_MOCK_DESCRIPTORS=1 cargo test -p ether-devices --test v02_descriptors` and commit
   the JSON (`ui/src/transport/mock/devices/<group>.json`). Never edit it by hand.
7. **Layout.** Ship a `DeviceLayout` for every device in `descriptor().layout` (checked by
   `tests/layouts.rs`; `EqCurve` is for the EQ only: filters use `FilterCurve`, multiband
   crossovers `Crossover`), built with `contract::{layout, section, item, knob}`
   (`ether_protocol::layout`): hero controls `Large`, typed widgets where they help
   (envelopes, filter curve, transfer curve, zone map, spectrum/tuner, step editor). Only
   specs and widget data: no bespoke panels, no styling. The shared renderer draws it with
   kit components and tokens and makes every param MIDI-learnable and a modulation target.
8. **Analysis / meters.** Implement `Node::{has_analysis, analysis}` (copy only; compute in
   `process`) with the encodings of CONTRACTS.md §12.4.3.
9. **Factory presets.** A few good ones per device under
   `crates/ether-devices/presets/<device-key>/<slug>.etherpreset` (`ether_model::preset`
   format, `BuiltinDeviceType::key()` folder), registered in the module's
   `factory_presets()` with `include_str!`; `v02_descriptors.rs` parses them.
10. **MIDI effects** follow CONTRACTS.md §12.4.4 (MIDI-thru of what they don't transform,
    `AllNotesOff` forwarded, own note ids, delays only).
11. **Tests and e2e.** `tests/<group>*.rs`, and an e2e (`apps/web/e2e/<device>*.spec.ts`:
    insert the device, turn a knob, hear or see the effect).
