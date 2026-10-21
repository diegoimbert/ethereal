import type { TourDef } from "../types";
import { FIRST_TRACK_TOUR } from "./firstTrack";
import { RECORD_AUDIO_TOUR } from "./recordAudio";

export { FIRST_TRACK_TOUR, RECORD_AUDIO_TOUR };

/** Every tour, by id. */
export const TOURS: Readonly<Record<string, TourDef>> = {
  [FIRST_TRACK_TOUR.id]: FIRST_TRACK_TOUR,
  [RECORD_AUDIO_TOUR.id]: RECORD_AUDIO_TOUR,
};
