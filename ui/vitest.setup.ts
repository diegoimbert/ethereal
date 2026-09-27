import "@testing-library/jest-dom/vitest";
import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { MOTION } from "@/timeline/motion";

afterEach(() => {
  cleanup();
});

// Wheel/zoom animations apply instantly in tests (motion is tested on its own).
MOTION.enabled = false;
