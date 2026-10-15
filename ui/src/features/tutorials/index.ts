// Guided tours (docs/TUTORIALS.md): the welcome screen's "get started" items walk the user
// through the real UI. The app shell mounts `TourRoot` once; `startTutorial` runs a tour.
export { TourRoot } from "./TourRoot";
export { canStartTutorial, startTutorial } from "./start";
export { isTourDone, markTourDone, useTutorialProgress } from "./progress";
export { exitTour, useTour } from "./store";
export { platformOf } from "./targets";
export { EXAMPLES, RECORD_AUDIO_TOUR, TOURS } from "./tours";
export type { ExampleDef } from "./tours";
export type { TourDef, TourPlatform, TourStep } from "./types";
