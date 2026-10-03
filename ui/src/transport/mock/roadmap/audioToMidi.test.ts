/** MockTransport: `AudioToMidi::*` (v0.3, audio-to-midi). Pins the contracts-4 stub. */
import { describe, it } from "vitest";
import type { AudioToMidiOptions } from "@/generated";
import { cmd } from "../../cmd";
import { expectUnsupported, testId, useMock } from "./testUtils";

describe("MockTransport audio to MIDI (audio-to-midi)", () => {
  const f = useMock();

  it("replies Unsupported until the node lands", async () => {
    const options: AudioToMidiOptions = {
      sensitivity: 0.5,
      min_duration: 0.05,
      min_pitch: 21,
      max_pitch: 108,
      kick_key: 36,
      snare_key: 38,
      hihat_key: 42,
    };
    await expectUnsupported(
      f,
      cmd("AudioToMidi", {
        type: "Start",
        job: "job-1",
        clip: testId(),
        mode: "Melody",
        options,
        track: testId(),
        new_clip: testId(),
        seed_notes: testId(),
        instrument: null,
      }),
    );
    await expectUnsupported(f, cmd("AudioToMidi", { type: "Cancel", job: "job-1" }));
  });
});
