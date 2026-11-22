"use client";

import Link from "next/link";
import { GitBranch, Plus } from "lucide-react";
import { getFreshness, getRepos } from "@/lib/source";
import { serviceColor } from "@/lib/workspace";
import { Chip, SettingsPage } from "./ui";
import "./repositories.css";

export default function Repositories() {
  const repos = getRepos();
  const fresh = getFreshness();
  const dirty = repos.filter((r) => r.dirty).length;
  return (
    <SettingsPage
      title="Repositories"
      saves="readonly"
      intro={`${repos.length} repositories in this workspace. ${fresh.behind === 0 ? "All are indexed up to HEAD" : `${fresh.behind} commits are not indexed yet`}${dirty ? `, and ${dirty} has uncommitted changes that the index does not include.` : "."}`}
      actions={<Link className="set-btn is-line" href="/welcome/add"><Plus size={16} aria-hidden />Add a workspace</Link>}
    >
      <section className="set-sec" aria-labelledby="set-repo-h">
        <h2 id="set-repo-h">Indexed repositories</h2>
        <div className="set-repo-row set-repo-head" aria-hidden>
          <span className="set-k">Repository</span><span className="set-k">Indexed commit</span><span className="set-k">HEAD</span><span className="set-k">Status</span><span className="set-k">Commits</span>
        </div>
        <ul className="set-repo-list">
          {repos.map((r) => (
            <li key={r.id} className="set-repo-row">
              <span className="set-repo-name">
                <i style={{ background: serviceColor(r.id) }} aria-hidden />
                <span><b>{r.name}</b><small className="set-mono">{r.path}</small></span>
              </span>
              <span className="set-mono">{r.indexedCommit.slice(0, 7)}</span>
              <span className="set-mono"><GitBranch size={12} aria-hidden /> {r.branch} · {r.head.slice(0, 7)}</span>
              <span className="set-repo-status">
                {r.behind === 0 ? <Chip tone="ok" dot>Current</Chip> : <Chip tone="warn" dot>{r.behind} {r.behind === 1 ? "commit" : "commits"} behind</Chip>}
                {r.dirty && <Chip tone="warn">Uncommitted changes</Chip>}
              </span>
              <span className="set-mono">{r.commits}</span>
            </li>
          ))}
        </ul>
        <p className="set-note">Uncommitted changes are not in the index. Commit them to include them. To change which repositories are read, edit the workspace file and run init again, or add another workspace.</p>
      </section>
    </SettingsPage>
  );
}
