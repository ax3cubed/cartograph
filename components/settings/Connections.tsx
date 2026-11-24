"use client";

import Link from "next/link";
import { getCurrentWorkspace, getRepos } from "@/lib/source";
import { useLive, useRepos, useWorkspaces } from "@/lib/api/live";
import { Chip, SettingsPage } from "./ui";
import "./connections.css";

/**
 * FE-13 merge note: local folders only. Cartograph reads git repositories on this machine and nothing else, so the
 * page lists the workspace's repositories as the folders they are read from. There is no provider to connect and no
 * token to keep.
 */
export default function Connections() {
  const live = useLive();
  const { current } = useWorkspaces();
  const liveRepos = useRepos(live ? current?.id : undefined);
  const ws = live ? current : getCurrentWorkspace();
  const repos: { id: string; name: string; path: string; branch: string }[] = (live ? liveRepos.data : getRepos()) ?? [];

  return (
    <SettingsPage title="Sources and connections" saves="readonly"
      intro="The folders Cartograph reads code and history from. Every one is a git repository on this machine.">
      <div className="set-conn-flow" aria-label="How a folder is used">
        <span><b>Local folders</b><small>{repos.length} {repos.length === 1 ? "repository" : "repositories"}</small></span><i aria-hidden>→</i>
        <span><b>Indexer</b><small>CLI on this machine</small></span><i aria-hidden>→</i>
        <span><b className="set-mono">{ws?.name ?? "workspace"}.cartograph</b><small>local</small></span><i aria-hidden>→</i>
        <span><b>App, Ask, MCP</b><small>read only</small></span>
      </div>
      <section className="set-sec" aria-labelledby="set-conn-h">
        <h2 id="set-conn-h">Local folders</h2>
        {live && liveRepos.error && <p className="set-note" role="alert">{liveRepos.error.what}. {liveRepos.error.try_}.</p>}
        <ul className="set-conn-list">
          {repos.map((r) => (
            <li key={r.id} className="set-conn-row">
              <span className="set-conn-ic" aria-hidden>FS</span>
              <span className="set-conn-name"><b>{r.name}</b><small className="set-mono">{r.path}</small></span>
              <span className="set-conn-gives"><Chip>Code</Chip><Chip>History</Chip></span>
              <span className="set-conn-state"><Chip tone="ok" dot>On this machine</Chip>{r.branch && <small>branch {r.branch}</small>}</span>
            </li>
          ))}
        </ul>
        <p className="set-note">Cartograph reads git on disk with no credentials, and never fetches unless the hourly fetch is on. To add a repository, add its folder to the workspace, or <Link className="set-conn-link" href="/welcome/add">map another workspace</Link>.</p>
      </section>
    </SettingsPage>
  );
}
