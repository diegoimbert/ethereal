/** The mobile UI's main view: the tracks (the arrangement) or the mixer. */
import { create } from "zustand";

export const useMobileView = create<{ view: "tracks" | "mixer" }>()(() => ({ view: "tracks" }));
