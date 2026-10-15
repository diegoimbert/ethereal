// The computer MIDI keyboard: play notes from the computer keyboard (also inside native
// plugin windows, through the key forwarding of `features/plugin-keys`).
export {
  bindTransport,
  handleForwardedKey,
  hasListeningTrack,
  releaseAll,
  setEnabled,
  targetTrack,
  toggleKeyboardMidi,
  TOGGLE_ACTION,
  useKeyboardMidi,
  type KeyInput,
} from "./keyboard";
export { KeyboardMidiButton } from "./KeyboardMidiButton";
export { isKeyboardMidiKey, noteFor, noteName } from "./notes";
