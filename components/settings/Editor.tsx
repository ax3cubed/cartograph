"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { getCurrentWorkspace, getFreshness, getMcpTools } from "@/lib/source";
import { useFreshness, useLive, useWorkspaces } from "@/lib/api/live";
import { bump, useResource } from "@/lib/api/cache";
import { RequestError, send } from "@/lib/api/client";
import { AGENTS, MCP_TOOLS, type AgentInstall, type AgentStatus, type FileDiff, type McpTool } from "@/lib/api/settings";
import { Chip, SettingsPage, useStored } from "./ui";
import { fmtDate } from "@/lib/date";
import "./editor.css";

const PLUGIN = "/plugin marketplace add ax3cubed/cartograph\n/plugin install cartograph";
const CONFIG = `{\n  "mcpServers": {\n    "cartograph": {\n      "command": "cartograph",\n      "args": ["mcp"]\n    }\n  }\n}`;

function CopyButton({ text, label }: { text: string; label: string }) {
  const [done, setDone] = useState(false);
  return (
    <button type="button" className="set-btn is-sm" onClick={async () => { try { await navigator.clipboard.writeText(text); setDone(true); setTimeout(() => setDone(false), 1800); } catch { /* clipboard blocked */ } }}>
      {done ? <Check size={14} aria-hidden /> : <Copy size={14} aria-hidden />}{done ? "Copied" : label}
    </button>
  );
}

/** Each agent's own config snippet (F5). VS Code names the key `servers`; Codex reads TOML. */
const SNIPPET: Record<string, string> = {
  vscode: CONFIG.replace('"mcpServers"', '"servers"'),
  codex: '[mcp_servers.cartograph]\ncommand = "cartograph"\nargs = ["mcp"]',
};

export default function Editor() {
  const live = useLive();
  const liveTools = useResource<McpTool[]>("", live ? MCP_TOOLS : null);
  const agents = useResource<AgentStatus[]>("", live ? AGENTS : null);
  const { current } = useWorkspaces();
  const liveFresh = useFreshness(live ? current?.id : undefined).data;
  const tools = live ? liveTools.data ?? [] : getMcpTools();
  const ws = live ? current : getCurrentWorkspace();
  const fresh = live ? liveFresh : getFreshness();
  const [transport, setTransport] = useStored<"stdio" | "http">("cartograph:mcp-transport", "stdio");

  return (
    <SettingsPage title="Use from your editor" saves="app"
      intro="Let your editor's AI agent look things up in this map. It reads the local index and never sends your code to a model.">
      <div className="set-grid">
        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-ed-cc">
            <h2 id="set-ed-cc">Claude Code</h2>
            <p className="set-note">Install the Cartograph plugin. One step adds the server, a skill and a session hook.</p>
            <pre className="set-pre">{PLUGIN}</pre>
            <div><CopyButton text={PLUGIN} label="Copy commands" /></div>
          </section>
          <section className="set-sec" aria-labelledby="set-ed-ot">
            <h2 id="set-ed-ot">Other editors</h2>
            <p className="set-note">Add this to your editor&apos;s MCP config.</p>
            <pre className="set-pre">{CONFIG}</pre>
            <div><CopyButton text={CONFIG} label="Copy config" /></div>
          </section>
        </div>
        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-ed-sv">
            <div className="set-sec-head"><h2 id="set-ed-sv">Server</h2><Chip tone="ok" dot>{transport === "stdio" ? "Started by your editor" : "HTTP on this machine"}</Chip></div>
            <dl className="set-ed-facts">
              <dt id="set-ed-tr">Connection</dt>
              <dd><div className="set-seg" role="radiogroup" aria-labelledby="set-ed-tr">
                <button type="button" role="radio" aria-checked={transport === "stdio"} onClick={() => setTransport("stdio")}>Editor starts it (default)</button>
                <button type="button" role="radio" aria-checked={transport === "http"} onClick={() => setTransport("http")}>Local web address</button>
              </div></dd>
              {transport === "http" && <><dt>Address</dt><dd className="set-mono">http://127.0.0.1:4317/mcp</dd></>}
              <dt>Index</dt><dd>{ws?.name}{fresh?.builtAt ? <> · built {fmtDate(fresh.builtAt, "md", "local")}</> : null}</dd>
              {live && ws && <><dt>Workspace</dt><dd className="set-mono">{ws.path}</dd></>}
            </dl>
            {live && <p className="set-note">The index itself lives in Cartograph&apos;s data folder (<span className="set-mono">~/.cartograph/workspaces/{ws?.id ?? "<workspace>"}/index.cartograph</span>, or under <span className="set-mono">CARTOGRAPH_HOME</span> when that is set), never inside the workspace.</p>}
            <p className="set-note">{transport === "stdio" ? "Your editor starts cartograph mcp itself. Nothing listens on a port." : "Only this machine can connect. The server listens on 127.0.0.1, so other machines can't reach it."}</p>
          </section>
          {live && <Agents list={agents.data} error={agents.error} loading={agents.loading} />}
          <section className="set-sec" aria-labelledby="set-ed-tl">
            <h2 id="set-ed-tl">What the agent can do · {tools.length}</h2>
            {live && liveTools.error && <p className="set-note" role="alert">{liveTools.error.what}. {liveTools.error.try_}.</p>}
            <ul className="set-ed-tools">{tools.map((t) => <li key={t.name}><code>{t.name}</code><span>{t.summary}</span><Chip tone={t.access === "write" ? "warn" : undefined}>{t.access === "write" ? "writes" : "read only"}</Chip></li>)}</ul>
          </section>
        </div>
      </div>
    </SettingsPage>
  );
}

/**
 * FE-21: which agents are on this machine and set up (F5, F6), each one's snippet, and "Set up" with a preview of every
 * file change before anything is written (E3). The server writes user-level config only; the diff is shown as text.
 */
function Agents({ list, error, loading }: { list?: AgentStatus[]; error?: RequestError; loading: boolean }) {
  const [open, setOpen] = useState<{ agent: string; diffs?: FileDiff[]; error?: string; busy?: boolean; done?: boolean } | null>(null);
  const call = async (agent: string, write: boolean) => {
    setOpen({ agent, busy: true, diffs: open?.agent === agent ? open.diffs : undefined });
    try {
      const diffs = await send<FileDiff[]>("POST", write ? "/api/v1/agents/install" : "/api/v1/agents/preview", { agent } satisfies AgentInstall);
      setOpen({ agent, diffs, done: write });
      if (write) bump("");
    } catch (e) {
      setOpen({ agent, error: e instanceof RequestError ? `${e.what}. ${e.try_}.` : "The change couldn't be made." });
    }
  };
  return (
    <section className="set-sec" aria-labelledby="set-ed-ag">
      <h2 id="set-ed-ag">Agents on this machine</h2>
      {error ? <p className="set-note" role="alert">{error.what}. {error.try_}.</p>
        : !list ? <p className="set-note" aria-busy={loading}>Looking for agents…</p> : (
          <ul className="set-ed-agents">
            {list.map((a) => {
              const mine = open?.agent === a.id ? open : null;
              const changes = mine?.diffs?.filter((d) => d.diff.trim()) ?? [];
              return (
                <li key={a.id}>
                  <div className="set-sec-head">
                    <b id={`set-ed-a-${a.id}`}>{a.name}</b>
                    <span className="set-ed-chips">
                      <Chip tone={a.installed ? "ok" : a.detected ? undefined : "off"} dot>{a.installed ? "Set up" : a.detected ? "Found, not set up" : "Not found"}</Chip>
                      {!a.installed && <button type="button" className="set-btn is-sm" aria-describedby={`set-ed-a-${a.id}`} disabled={mine?.busy} onClick={() => call(a.id, false)}>Preview set-up</button>}
                    </span>
                  </div>
                  {a.config_path && <p className="set-note set-ed-path">Config: <span className="set-mono" title={a.config_path}>{a.config_path}</span></p>}
                  {a.config_path && a.id === "claude-code" && <p className="set-note">With the Cartograph plugin installed, Claude Code needs no entry here.</p>}
                  <details className="set-adv"><summary>Config snippet</summary><div><pre className="set-pre">{SNIPPET[a.id] ?? CONFIG}</pre><CopyButton text={SNIPPET[a.id] ?? CONFIG} label="Copy snippet" /></div></details>
                  {mine?.error && <p className="set-note" role="alert">{mine.error}</p>}
                  {mine?.diffs && !mine.done && (
                    <div className="set-confirm" role="alertdialog" aria-labelledby={`set-ed-c-${a.id}`}>
                      <b id={`set-ed-c-${a.id}`}>{changes.length ? `Set up ${a.name}? This changes ${changes.length} ${changes.length === 1 ? "file" : "files"}.` : `${a.name} needs no change.`}</b>
                      {changes.map((d) => <div key={d.file}><p className="set-mono">{d.file}</p><pre className="set-pre" aria-label={`Changes to ${d.file}`}>{d.diff}</pre></div>)}
                      <div className="set-actions">
                        {changes.length > 0 && <button type="button" className="set-btn is-primary" disabled={mine.busy} onClick={() => call(a.id, true)}>Write {changes.length === 1 ? "this change" : "these changes"}</button>}
                        <button type="button" className="set-btn" onClick={() => setOpen(null)}>Cancel</button>
                      </div>
                    </div>
                  )}
                  {mine?.done && <p className="set-note" role="status">{a.name} is set up. Restart it to load Cartograph.</p>}
                </li>
              );
            })}
          </ul>
        )}
    </section>
  );
}
