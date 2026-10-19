import type { TourDef } from "../types";
import { BASSLINE_TOUR, EXAMPLES, HIP_HOP_TOUR, HOUSE_TOUR, LOFI_TOUR } from "./examples";
import { FIRST_TRACK_TOUR } from "./firstTrack";
import { RECORD_AUDIO_TOUR } from "./recordAudio";

export { EXAMPLES, FIRST_TRACK_TOUR, RECORD_AUDIO_TOUR };
export type { ExampleDef } from "./examples";

/** Every tour, by id. */
export const TOURS: Readonly<Record<string, TourDef>> = {
  [FIRST_TRACK_TOUR.id]: FIRST_TRACK_TOUR,
  [RECORD_AUDIO_TOUR.id]: RECORD_AUDIO_TOUR,
  [HOUSE_TOUR.id]: HOUSE_TOUR,
  [HIP_HOP_TOUR.id]: HIP_HOP_TOUR,
  [LOFI_TOUR.id]: LOFI_TOUR,
  [BASSLINE_TOUR.id]: BASSLINE_TOUR,
};
