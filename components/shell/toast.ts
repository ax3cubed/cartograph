/** Show a toast from anywhere: `toast("Copied")`. The Toasts component in the shell listens for it. */
export const TOAST_EVENT = "cartograph:toast";
const QUEUE = "cartograph.toast-queue";

export const toast = (message: string) => {
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent(TOAST_EVENT, { detail: message }));
};

/** For a toast that has to appear after navigation: the next route's Toasts shows it once it mounts. */
export const queueToast = (message: string) => {
  try { sessionStorage.setItem(QUEUE, JSON.stringify([...takeQueue(false), message])); } catch { /* storage can be blocked */ }
};
export const takeQueue = (clear = true): string[] => {
  try {
    const q = JSON.parse(sessionStorage.getItem(QUEUE) ?? "[]") as string[];
    if (clear) sessionStorage.removeItem(QUEUE);
    return q;
  } catch { return []; }
};
