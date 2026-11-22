// Colour rule (board 00): the six pinned hues mark domains. Services in a domain take one of three tints by fan-in,
// and domains past the sixth fall to steel. Domains come from the repo's top folder, or a `domain:` key in the override file.
import { LINKS, SERVICES } from "@/lib/workspace";

export const DOMAINS = [
  { id: "edge", services: ["gateway"] },
  { id: "identity", services: ["auth"] },
  { id: "money", services: ["billing"] },
  { id: "catalog", services: ["catalog"] },
  { id: "messaging", services: ["notify"] },
  { id: "platform", services: ["shared"] },
];

/** CSS colour of domain number i: one of --hue-1..6, then steel. */
export const domainHue = (i: number) => (i >= 0 && i < 6 ? `var(--hue-${i + 1})` : "var(--steel)");
const fanIn = (id: string) => LINKS.filter((l) => !l.removed && l.to.startsWith(id + "/") && !l.from.startsWith(id + "/")).length;

const tint = new Map<string, string>();
DOMAINS.forEach((d, i) => {
  const base = domainHue(i);
  [...d.services].sort((a, b) => fanIn(b) - fanIn(a)).forEach((id, rank) => {
    tint.set(id, rank === 0 ? base : rank === 1 ? `color-mix(in oklab, ${base}, white 35%)` : `color-mix(in oklab, ${base}, black 25%)`);
  });
});
for (const s of SERVICES) if (!tint.has(s.id)) tint.set(s.id, "var(--steel)");

export const serviceHue = (id: string) => tint.get(id) ?? "var(--steel)";
/** Text set in a hue stays readable in light mode. */
export const hueText = (c: string) => `color-mix(in oklab, ${c}, black var(--hue-dim))`;
