// The block the engine hands the WebCLAP host each `process` call, in the engine's wasm
// memory: `Exchange` and `EventRec` of crates/ether-wasm/src/webclap/node.rs (wasm32
// offsets, asserted there at compile time).

export const EXCHANGE = {
  frames: 0,
  inChannels: 4,
  outChannels: 8,
  scChannels: 12,
  nEvents: 16,
  nOutEvents: 20,
  eventCapacity: 24,
  flags: 28,
  inputs: 32,
  outputs: 40,
  sidechain: 48,
  events: 56,
  outEvents: 60,
  steadyTime: 64,
  position: 72,
  seconds: 80,
  bpm: 88,
  tempoInc: 96,
  barStart: 104,
  loopStart: 112,
  loopEnd: 120,
  barNumber: 128,
  tsNum: 132,
  tsDen: 136,
} as const;

/** `Exchange.flags`. */
export const XFLAGS = { playing: 1, recording: 2, loopActive: 4 } as const;

/** `EventRec` (24 bytes). */
export const REC = { size: 24, offset: 0, kind: 4, channel: 5, key: 6, extra: 7, id: 8, value: 16 } as const;

/** `EventRec.kind`. */
export const KIND = {
  noteOn: 1,
  noteOff: 2,
  noteChoke: 3,
  allNotesOff: 4,
  param: 5,
  midi: 6,
  noteExpression: 7,
} as const;

/** `Guest::process` results (crates/ether-wasm/src/webclap/guest.rs `status`). */
export const STATUS = { error: 0, faulted: -1, pending: -2 } as const;

/** `takeNotifications` kinds (crates/ether-wasm/src/webclap/js.rs `note`). */
export const NOTE = { param: 1, begin: 2, end: 3, dirty: 4, params: 5, crashed: 6 } as const;
