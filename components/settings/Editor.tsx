"use client";

import { useState } from "react";
import { Check, Copy } from "lucide-react";
import { getCurrentWorkspace, getFreshness, getMcpTools } from "@/lib/source";
import { Chip, SettingsPage, useStored } from "./ui";
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

export default function Editor() {
  const tools = getMcpTools();
  const ws = getCurrentWorkspace();
  const fresh = getFreshness();
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
              <dt>Index</dt><dd>{ws?.name} · built {new Date(fresh.builtAt).toLocaleDateString("en", { month: "short", day: "numeric" })}</dd>
            </dl>
            <p className="set-note">{transport === "stdio" ? "Your editor starts cartograph mcp itself. Nothing listens on a port." : "Only this machine can connect. The server listens on 127.0.0.1, so other machines can't reach it."}</p>
          </section>
          <section className="set-sec" aria-labelledby="set-ed-tl">
            <h2 id="set-ed-tl">What the agent can do · {tools.length}</h2>
            <ul className="set-ed-tools">{tools.map((t) => <li key={t.name}><code>{t.name}</code><span>{t.summary}</span><Chip tone={t.access === "write" ? "warn" : undefined}>{t.access === "write" ? "writes" : "read only"}</Chip></li>)}</ul>
          </section>
        </div>
      </div>
    </SettingsPage>
  );
}
