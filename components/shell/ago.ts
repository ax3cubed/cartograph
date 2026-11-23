import { now } from "@/lib/source";
import { isLive } from "@/lib/api/session";

/** The one place a build time becomes words, so the chip, popover, band and Activity always agree. */
export const ago = (iso: string) => {
  const at = Date.parse(iso);
  if (Number.isNaN(at)) return "not built yet";
  const h = Math.round(((isLive() ? Date.now() : now()) - at) / 3_600_000); // the demo pins its own "today"
  return h < 1 ? "just now" : h < 24 ? `${h}h ago` : `${Math.round(h / 24)}d ago`;
};
