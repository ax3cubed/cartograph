const KEY = "cartograph.add-origin";

/** Call before navigating to the wizard, so Cancel and the close button can go back to where the person was. */
export const rememberOrigin = () => {
  try { sessionStorage.setItem(KEY, location.pathname + location.search + location.hash); } catch { /* storage can be blocked */ }
};
export const takeOrigin = (): string | null => {
  try { const v = sessionStorage.getItem(KEY); return v && !v.startsWith("/welcome/add") ? v : null; } catch { return null; }
};
