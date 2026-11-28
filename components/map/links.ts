// FE-14: what the correction dialog sends. A link is named by its ends (`service:path`) and, for an HTTP link, its
// operation; the server checks the key against the index and writes the entry. Nothing is applied in the browser: the
// map shows a correction once the index has it.
import type { ConfidenceLevel, CorrectionAction, CorrectionRequest, LinkDetail, LinkKey } from "@/lib/api";
import { opText } from "./linkEvidence.ts";

export const MAX_REASON = 500;

export const VERBS: { verb: CorrectionAction; label: string; help: string }[] = [
  { verb: "remove", label: "Remove", help: "Hide this link: it isn't a real dependency." },
  { verb: "retarget", label: "Retarget", help: "Keep the link, but point it at another file." },
  { verb: "set", label: "Set level", help: "Fix how sure Cartograph is about this link." },
  { verb: "add", label: "Add a link", help: "Open a link between two files that Cartograph didn't find." },
];

export const LEVELS: ConfidenceLevel[] = ["exact", "likely", "guessed"];

/** `service:path`, the form an end takes in the override file. */
export const endOf = (service: string, path: string) => `${service}:${path}`;

export const keyOf = (d: LinkDetail, from: string, to: string): LinkKey => ({
  kind: d.link.kind, from, to, op: d.operation ? opText(d.operation) : null, topic: null, table: null,
});

export interface Form { verb: CorrectionAction; from: string; to: string; level: ConfidenceLevel; reason: string }

/** Why the form can't be sent yet, or null. The server checks the rest. */
export function formProblem(f: Form): string | null {
  const reason = f.reason.trim();
  if (!reason) return "Say why, for the people who read the file.";
  if ([...f.reason].length > MAX_REASON) return `The reason is ${[...f.reason].length} characters; the limit is ${MAX_REASON}.`;
  // the server's rule: no control character, line or paragraph separator, or byte order mark
  if ([...f.reason].some((c) => { const n = c.codePointAt(0)!; return n < 32 || (n >= 127 && n <= 159) || n === 0x2028 || n === 0x2029 || n === 0xfeff; })) return "The reason is one line, without control characters.";
  if (f.verb === "retarget" && !f.to.trim()) return "Name the file it should point at.";
  if (f.verb === "add" && (!f.from.trim() || !f.to.trim())) return "Name both files.";
  return null;
}

/** The request for the form. `key` is the link being corrected; an added link's ends are the form's. */
export function request(f: Form, key: LinkKey): Omit<CorrectionRequest, "dry_run"> {
  const reason = f.reason.trim();
  switch (f.verb) {
    case "remove": return { action: "remove", key, reason };
    case "retarget": return { action: "retarget", key, to: f.to.trim(), reason };
    case "set": return { action: "set", key, confidence: f.level, reason };
    case "add": return { action: "add", key: { ...key, op: null, from: f.from.trim(), to: f.to.trim() }, confidence: f.level, reason };
  }
}
