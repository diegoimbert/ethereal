// The CLAP ABI as a wasm32 WebCLAP sees it: struct offsets and sizes (pointers and
// `size_t` are 4 bytes, function pointers are indices into the module's function table),
// event and status constants. Only what the host reads or writes.
//
// Layouts follow CLAP 1.2 (free-audio/clap, MIT) for the wasm32 data layout.

export const CLAP_VERSION = [1, 2, 2] as const;

/** `clap_plugin_entry`. */
export const ENTRY = { init: 12, deinit: 16, getFactory: 20 } as const;

/** `clap_plugin_factory`. */
export const FACTORY = { count: 0, descriptor: 4, create: 8 } as const;

/** `clap_plugin_descriptor`. */
export const DESC = {
  id: 12,
  name: 16,
  vendor: 20,
  url: 24,
  manualUrl: 28,
  supportUrl: 32,
  version: 36,
  description: 40,
  features: 44,
} as const;

/** `clap_host` (48 bytes). */
export const HOST = {
  size: 48,
  hostData: 12,
  name: 16,
  vendor: 20,
  url: 24,
  version: 28,
  getExtension: 32,
  requestRestart: 36,
  requestProcess: 40,
  requestCallback: 44,
} as const;

/** `clap_plugin`. */
export const PLUGIN = {
  desc: 0,
  init: 8,
  destroy: 12,
  activate: 16,
  deactivate: 20,
  startProcessing: 24,
  stopProcessing: 28,
  reset: 32,
  process: 36,
  getExtension: 40,
  onMainThread: 44,
} as const;

/** `clap_process` (40 bytes, 8-aligned). */
export const PROCESS = {
  size: 40,
  steadyTime: 0,
  frames: 8,
  transport: 12,
  audioInputs: 16,
  audioOutputs: 20,
  inputsCount: 24,
  outputsCount: 28,
  inEvents: 32,
  outEvents: 36,
} as const;

/** `clap_audio_buffer` (24 bytes, 8-aligned). */
export const AUDIO_BUFFER = { size: 24, data32: 0, data64: 4, channelCount: 8, latency: 12, constantMask: 16 } as const;

/** `clap_input_events` (12) / `clap_output_events` (8). */
export const IN_EVENTS = { size: 12, ctx: 0, sizeFn: 4, get: 8 } as const;
export const OUT_EVENTS = { size: 8, ctx: 0, tryPush: 4 } as const;
/** `clap_istream` / `clap_ostream` (8). */
export const STREAM = { size: 8, ctx: 0, fn: 4 } as const;

/** `clap_event_header` (16). */
export const EVENT = { size: 0, time: 4, spaceId: 8, type: 10, flags: 12 } as const;
export const CORE_EVENT_SPACE = 0;
export const EVENT_IS_LIVE = 1;

export const EV = {
  noteOn: 0,
  noteOff: 1,
  noteChoke: 2,
  noteEnd: 3,
  noteExpression: 4,
  paramValue: 5,
  paramMod: 6,
  gestureBegin: 7,
  gestureEnd: 8,
  transport: 9,
  midi: 10,
  midiSysex: 11,
  midi2: 12,
} as const;

/** `clap_event_note` (40). */
export const NOTE = { size: 40, noteId: 16, port: 20, channel: 22, key: 24, velocity: 32 } as const;
/** `clap_event_note_expression` (40). */
export const NOTE_EXPR = { size: 40, expressionId: 16, noteId: 20, port: 24, channel: 26, key: 28, value: 32 } as const;
/** `clap_event_param_value` (48). */
export const PARAM_VALUE = { size: 48, paramId: 16, cookie: 20, noteId: 24, port: 28, channel: 30, key: 32, value: 40 } as const;
/** `clap_event_param_gesture` (20). */
export const PARAM_GESTURE = { size: 20, paramId: 16 } as const;
/** `clap_event_midi` (24). */
export const MIDI = { size: 24, port: 16, data: 18 } as const;
/** `clap_event_transport` (104). */
export const TRANSPORT = {
  size: 104,
  flags: 16,
  songPosBeats: 24,
  songPosSeconds: 32,
  tempo: 40,
  tempoInc: 48,
  loopStartBeats: 56,
  loopEndBeats: 64,
  loopStartSeconds: 72,
  loopEndSeconds: 80,
  barStart: 88,
  barNumber: 96,
  tsigNum: 100,
  tsigDenom: 102,
} as const;
export const TRANSPORT_FLAGS = {
  hasTempo: 1 << 0,
  hasBeatsTimeline: 1 << 1,
  hasSecondsTimeline: 1 << 2,
  hasTimeSignature: 1 << 3,
  isPlaying: 1 << 4,
  isRecording: 1 << 5,
  isLoopActive: 1 << 6,
} as const;
/** `CLAP_BEATTIME_FACTOR` = `CLAP_SECTIME_FACTOR` = 2^31. */
export const TIME_FACTOR = 2 ** 31;

/** Largest event the host writes (param value). */
export const EVENT_SLOT = 48;

/** `clap_param_info` (1320). */
export const PARAM_INFO = { size: 1320, id: 0, flags: 4, cookie: 8, name: 12, module: 268, min: 1296, max: 1304, default: 1312 } as const;
export const PARAM_FLAGS = { stepped: 1 << 0, hidden: 1 << 2, readonly: 1 << 3, automatable: 1 << 5, enum: 1 << 16 } as const;

/** `clap_plugin_params`. */
export const PARAMS_EXT = { count: 0, getInfo: 4, getValue: 8, valueToText: 12, textToValue: 16, flush: 20 } as const;
/** `clap_plugin_state`. */
export const STATE_EXT = { save: 0, load: 4 } as const;
/** `clap_plugin_audio_ports` / `clap_audio_port_info` (276). */
export const AUDIO_PORTS_EXT = { count: 0, get: 4 } as const;
export const AUDIO_PORT_INFO = { size: 276, id: 0, name: 4, flags: 260, channelCount: 264, portType: 268, inPlacePair: 272 } as const;
export const AUDIO_PORT_IS_MAIN = 1;
/** `clap_plugin_note_ports` / `clap_note_port_info` (268). */
export const NOTE_PORTS_EXT = { count: 0, get: 4 } as const;
export const NOTE_PORT_INFO = { size: 268, id: 0, supportedDialects: 4, preferredDialect: 8, name: 12 } as const;
export const NOTE_DIALECT_CLAP = 1;
/** `clap_plugin_latency` / `clap_plugin_tail`: `get`. */
export const GET_EXT = { get: 0 } as const;
/** `clap_plugin_webview`. */
export const WEBVIEW_EXT = { getUri: 0, getResource: 4, receive: 8 } as const;
/** `clap_plugin_gui`. */
export const GUI_EXT = { isApiSupported: 0, create: 8, destroy: 12, show: 52, hide: 56 } as const;

export const PROCESS_STATUS = { error: 0, continue: 1, continueIfNotQuiet: 2, tail: 3, sleep: 4 } as const;

export const LOG = { debug: 0, info: 1, warning: 2, error: 3, fatal: 4, hostMisbehaving: 5, pluginMisbehaving: 6 } as const;

/** CLAP note expression ids the host sends. */
export const NOTE_EXPRESSION = { tuning: 2, brightness: 5, pressure: 6 } as const;

/** Extension ids. */
export const EXT = {
  params: "clap.params",
  state: "clap.state",
  audioPorts: "clap.audio-ports",
  notePorts: "clap.note-ports",
  latency: "clap.latency",
  tail: "clap.tail",
  log: "clap.log",
  threadCheck: "clap.thread-check",
  webview: "clap.webview/3",
  gui: "clap.gui",
} as const;
