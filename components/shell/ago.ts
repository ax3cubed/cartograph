import { now } from "@/lib/source";

/** The one place a build time becomes words, so the chip, popover, band and Activity always agree. */
export const ago = (iso: string) => {
  const h = Math.round((now() - Date.parse(iso)) / 3_600_000);
  return h < 1 ? "just now" : h < 24 ? `${h}h ago` : `${Math.round(h / 24)}d ago`;
};
