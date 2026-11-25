// XS-06: the link inspector's evidence, by role, from …/links/{id}. Plain functions so `pnpm check` can run them.
import type { EvidenceSite, LinkDetail, LinkOperation } from "../../lib/api/types.gen.ts";

const ROLES = [["contract", "Contract"], ["caller", "Caller"], ["handler", "Handler"]] as const;

/** The roles that have at least one site, in reading order: the contract, then who calls, then who handles. */
export const roles = (d: LinkDetail) =>
  ROLES.map(([role, title]) => ({ role, title, sites: d.evidence[role] })).filter((g) => g.sites.length > 0);

export const opText = (o: LinkOperation) => `${o.method.toUpperCase()} ${o.template}`;

export const siteLabel = (s: EvidenceSite) => `${s.file}:${s.line}`;
