import type { Part } from "@/lib/ask";
import { byId, commitBySha } from "@/lib/workspace";

export const fmtDay = (ms: number) => new Date(ms).toLocaleDateString("en-GB", { day: "numeric", month: "short", year: "numeric", timeZone: "UTC" });
export const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);

/** Plain text of engine parts. */
export const partsText = (parts: Part[]) =>
  parts.map((p) => (typeof p === "string" ? p : "node" in p ? byId.get(p.node)?.name ?? p.node : commitBySha.get(p.commit)?.sha ?? p.commit)).join("");
