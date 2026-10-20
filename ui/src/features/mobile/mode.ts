/**
 * The mobile UI: phones and touch tablets get `MobileShell` instead of the desktop
 * editor shell. The host turns it on before rendering (`enableMobileUi`; the web app does so
 * for `?mobile=1`, see apps/web/src/mobile.ts), and `<App />` picks the shell from it.
 *
 * The layout follows the viewport, not the device: `formFactorOf` (pure) and
 * `useFormFactor` (re-renders on resize / rotation).
 */
import { useEffect, useSyncExternalStore } from "react";
import { create } from "zustand";
import { setRenderDprCap } from "@/timeline/renderDpr";

export type FormFactor = "phone-portrait" | "phone-landscape" | "tablet";

/** A phone's short side is under this (CSS px); the web gate uses the same bound. */
export const PHONE_SHORT_SIDE_PX = 600;

/** The layout for a viewport of `width` × `height` CSS px. */
export function formFactorOf(width: number, height: number): FormFactor {
  if (Math.min(width, height) >= PHONE_SHORT_SIDE_PX) return "tablet";
  return width > height ? "phone-landscape" : "phone-portrait";
}

export const isPhone = (f: FormFactor): boolean => f !== "tablet";

/** The default lane height on phones (px): the header's name above its touch-sized buttons. */
export const PHONE_TRACK_HEIGHT = 72;

/** The default lane height on tablets (px): the name, then touch-sized buttons, volume and pan. */
export const TABLET_TRACK_HEIGHT = 80;

/** The timeline canvases' pixel ratio cap on mobile (the design's §11). */
export const MOBILE_MAX_DPR = 2;

interface MobileUiState {
  enabled: boolean;
}

export const useMobileUi = create<MobileUiState>()(() => ({ enabled: false }));

/** Use the mobile shell (call before the first render). */
export function enableMobileUi(enabled = true): void {
  useMobileUi.setState({ enabled });
  setRenderDprCap(enabled ? MOBILE_MAX_DPR : Infinity);
}

export const isMobileUi = (): boolean => useMobileUi.getState().enabled;

function viewportSize(): { width: number; height: number } {
  return { width: window.innerWidth, height: window.innerHeight };
}

function subscribeViewport(onChange: () => void): () => void {
  window.addEventListener("resize", onChange);
  window.addEventListener("orientationchange", onChange);
  window.visualViewport?.addEventListener("resize", onChange);
  return () => {
    window.removeEventListener("resize", onChange);
    window.removeEventListener("orientationchange", onChange);
    window.visualViewport?.removeEventListener("resize", onChange);
  };
}

const currentFormFactor = (): FormFactor => {
  const { width, height } = viewportSize();
  return formFactorOf(width, height);
};

/**
 * The current layout, re-rendering on resize and rotation. Also marks the document
 * (`<html data-eth-mobile="…">`) so styles of shared components can adapt under it without
 * touching the desktop.
 */
export function useFormFactor(): FormFactor {
  const factor = useSyncExternalStore(subscribeViewport, currentFormFactor, () => "phone-portrait" as FormFactor);
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.ethMobile = factor;
    return () => {
      delete root.dataset.ethMobile;
    };
  }, [factor]);
  return factor;
}
