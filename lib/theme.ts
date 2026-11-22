// Appearance settings live on this machine only (localStorage). The same logic runs before paint
// as the inline script in app/layout.tsx (THEME_BOOT), so the page never flashes the wrong theme.
import type { AppearanceSettings } from "./model.ts";

export const APPEARANCE_KEY = "cartograph:appearance";

export const DEFAULT_APPEARANCE: AppearanceSettings = {
  theme: "dark", contrast: "standard", motion: "follow", hues: "default", size: 100, density: "comfortable",
};

export function loadAppearance(): AppearanceSettings {
  try {
    const raw = localStorage.getItem(APPEARANCE_KEY);
    return raw ? { ...DEFAULT_APPEARANCE, ...JSON.parse(raw) } : DEFAULT_APPEARANCE;
  } catch { return DEFAULT_APPEARANCE; }
}

export function applyAppearance(a: AppearanceSettings, el: HTMLElement = document.documentElement) {
  el.dataset.theme = a.theme;
  el.dataset.contrast = a.contrast === "more" ? "more" : "standard";
  el.dataset.motion = a.motion;
  el.dataset.hues = a.hues;
  el.dataset.size = String(a.size);
  el.dataset.density = a.density;
}

export function saveAppearance(a: AppearanceSettings) {
  try { localStorage.setItem(APPEARANCE_KEY, JSON.stringify(a)); } catch { /* storage unavailable */ }
  applyAppearance(a);
}

// ponytail: kept as a string because it must run before React; keep in step with applyAppearance
export const THEME_BOOT = `try{var a=JSON.parse(localStorage.getItem("${APPEARANCE_KEY}")||"{}"),d=document.documentElement.dataset;d.theme=a.theme||"dark";d.contrast=a.contrast==="more"?"more":"standard";d.motion=a.motion||"follow";d.hues=a.hues||"default";d.size=String(a.size||100);d.density=a.density||"comfortable"}catch(e){}`;
