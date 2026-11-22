import type { CommitImpact } from "../model.ts";
import { COMMITS, LINKS, commitBySha } from "../workspace.ts";

// A5: what a commit changed in the graph, and who depends on the nodes it touched.
// Steps along the global commit list here; the real index follows first-parent commits per repo.
export function commitImpact(sha: string): CommitImpact | undefined {
  const commit = commitBySha.get(sha);
  if (!commit) return undefined;
  const i = COMMITS.indexOf(commit);
  const addedLinks = LINKS.filter((l) => l.added === sha);
  const removedLinks = LINKS.filter((l) => l.removed === sha);
  const touched = new Set([...addedLinks, ...removedLinks].flatMap((l) => [l.from]));
  const affected = new Set<string>();
  const stack = [...touched];
  while (stack.length) {
    const id = stack.pop()!;
    for (const l of LINKS) if (l.to === id && !l.removed && !affected.has(l.from) && !touched.has(l.from)) { affected.add(l.from); stack.push(l.from); }
  }
  return { commit, prev: COMMITS[i - 1]?.sha, next: COMMITS[i + 1]?.sha, addedLinks, removedLinks, affected: [...affected] };
}
