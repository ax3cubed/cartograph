"use client";

import { useRef, useState } from "react";
import { Check, X } from "lucide-react";
import { CONNECTIONS, PROVIDER_KINDS } from "@/lib/demo/settings";
import type { Connection, ProviderKind } from "@/lib/model";
import { getCurrentWorkspace } from "@/lib/source";
import { Chip, SettingsPage } from "./ui";
import "./connections.css";

export default function Connections() {
  const [rows, setRows] = useState<Connection[]>(CONNECTIONS);
  const [panel, setPanel] = useState<{ kind: ProviderKind; id?: string } | null>(null);
  const [token, setToken] = useState(false);
  const [msg, setMsg] = useState("");
  const heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const ws = getCurrentWorkspace();
  const kind = PROVIDER_KINDS.find((p) => p.kind === panel?.kind);

  const open = (p: { kind: ProviderKind; id?: string }, from?: HTMLElement) => {
    if (from) opener.current = from;
    setPanel(p); setToken(false); setMsg("");
    requestAnimationFrame(() => heading.current?.focus());
  };
  const close = () => { setPanel(null); requestAnimationFrame(() => opener.current?.focus()); };
  const connect = (how: string) => {
    if (panel?.id) setRows(rows.map((r) => (r.id === panel.id ? { ...r, status: "Connected", tone: "ok", when: "just now", action: undefined } : r)));
    setMsg(`${how} accepted. Demo workspace: nothing is stored until the local server can use your keychain.`);
  };

  return (
    <SettingsPage title="Sources and connections" saves="app" scope="Each connection saves when you connect it."
      intro="The places Cartograph reads code, config and specs from. Tokens stay in your system keychain and are never written to the index.">
      <div className="set-conn-flow" aria-label="How a connection is used">
        <span><b>Sources</b><small>{rows.length} connections</small></span><i aria-hidden>→</i>
        <span><b>Indexer</b><small>CLI on this machine</small></span><i aria-hidden>→</i>
        <span><b className="set-mono">{ws?.name ?? "workspace"}.cartograph</b><small>local</small></span><i aria-hidden>→</i>
        <span><b>App, Ask, MCP</b><small>read only</small></span>
      </div>
      <div className={`set-conn-layout${panel ? " has-panel" : ""}`}>
        <section className="set-sec" aria-labelledby="set-conn-h">
          <div className="set-sec-head"><h2 id="set-conn-h">Connections</h2><button type="button" className="set-btn" onClick={(e) => open({ kind: "github" }, e.currentTarget)}>Add connection</button></div>
          <ul className="set-conn-list">
            {rows.map((c) => (
              <li key={c.id} className="set-conn-row">
                <span className="set-conn-ic" aria-hidden>{c.ic}</span>
                <span className="set-conn-name"><b>{c.name}</b><small>{c.auth}</small></span>
                <span className="set-conn-gives">{c.provides.map((g) => <Chip key={g}>{g}</Chip>)}</span>
                <span className="set-conn-state"><Chip tone={c.tone} dot>{c.status}</Chip><small>{c.when}</small></span>
                <span>{c.action && <button type="button" className="set-btn is-sm" aria-label={`${c.action}: ${c.name}`} onClick={(e) => open({ kind: c.kind, id: c.id }, e.currentTarget)}>{c.action}</button>}</span>
              </li>
            ))}
          </ul>
          <p className="set-note">Cartograph reads on demand. It installs no GitHub App and receives no push events.</p>
        </section>

        {panel && kind && (
          <aside className="set-sec set-conn-panel" role="dialog" aria-labelledby="set-conn-ph" onKeyDown={(e) => e.key === "Escape" && close()}>
            <div className="set-sec-head"><h2 id="set-conn-ph" ref={heading} tabIndex={-1}>{panel.id ? `Fix ${kind.label}` : "Add connection"}</h2><button type="button" className="icon-btn sm" aria-label="Close" onClick={close}><X size={14} /></button></div>
            {panel.id ? (
              <p className="set-conn-fix"><span className="set-conn-ic" aria-hidden>{kind.ic}</span>{rows.find((r) => r.id === panel.id)?.name}</p>
            ) : (
              <div className="set-conn-kinds" role="radiogroup" aria-label="Provider">
                {PROVIDER_KINDS.map((p) => (
                  <button key={p.kind} type="button" role="radio" aria-checked={p.kind === panel.kind} onClick={() => open({ kind: p.kind })}>
                    <span className="set-conn-ic" aria-hidden>{p.ic}</span>{p.label}
                  </button>
                ))}
              </div>
            )}
            {kind.needs.length > 0 && (
              <div>
                <span className="set-k">The token needs</span>
                <ul className="set-conn-needs">{kind.needs.map(([n, a]) => <li key={n}><Check size={14} aria-hidden /><span>{n}</span><small>{a}</small></li>)}</ul>
              </div>
            )}
            <p className="set-note">The token is kept in your system keychain and only used from this machine. Cartograph never writes to your repositories.</p>
            {kind.kind === "github" && <button type="button" className="set-btn is-primary" onClick={() => connect("The gh CLI login")}>Use my gh CLI login</button>}
            {kind.kind === "github" && !token && <button type="button" className="set-conn-link" onClick={() => setToken(true)}>or paste a fine-grained token</button>}
            {(token || (kind.kind !== "github" && kind.kind !== "folder")) && (
              <form className="set-conn-token" onSubmit={(e) => { e.preventDefault(); connect("The token"); }}>
                <label className="set-label">{kind.kind === "spec" ? "Spec URL" : "Token"}
                  <input className="set-input is-mono" type={kind.kind === "spec" ? "url" : "password"} autoComplete="off" required />
                </label>
                <button type="submit" className="set-btn is-primary">{kind.kind === "spec" ? "Save URL" : "Save token"}</button>
              </form>
            )}
            {kind.kind === "folder" && <button type="button" className="set-btn is-primary" onClick={() => connect("The folder")}>Choose a folder</button>}
            <p className="set-status" role="status">{msg}</p>
          </aside>
        )}
      </div>
    </SettingsPage>
  );
}
