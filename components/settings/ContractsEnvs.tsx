"use client";

import { useState } from "react";
import { ADAPTERS, CALL_SITES, UNRECOGNISED_ROUTES, envKeys, type EnvName } from "@/lib/demo/settings";
import { getContracts } from "@/lib/source";
import { Chip, Seg, SettingsPage, useFileDraft } from "./ui";
import "./contractsEnvs.css";

const KINDS = { openapi: "OpenAPI 3.1", events: "AsyncAPI" } as const;

export default function ContractsEnvs() {
  const specs = getContracts();
  const [env, setEnv] = useState<EnvName>("prod");
  const [snippet, setSnippet] = useState(false);
  const f = useFileDraft<Record<string, string>>("cartograph:env-values", {});
  const at = (key: string) => `${env}:${key}`;
  /** An edit is kept only when it differs from the detected value. */
  const setVal = (key: string, detected: string, v: string) => {
    const next = { ...f.value };
    if (v === detected) delete next[at(key)]; else next[at(key)] = v;
    f.set(next);
  };
  const search = f.value[at("SEARCH_URL")] ?? "";
  const keys = envKeys(env);
  const pct = (n: number) => Math.round((n / CALL_SITES.total) * 100);
  const filled = search.trim().length > 0;
  const resolved = CALL_SITES.resolved + (filled ? CALL_SITES.searchUrl : 0);

  return (
    <SettingsPage title="Call matching" saves="file" file={{ count: f.count, onSave: f.save, onDiscard: f.discard }}
      intro="How Cartograph links a call in one service to the code it reaches in another, using API specs and each environment's config.">
      <div className="set-grid set-ce-grid">
        <div className="set-col">
          <section className="set-sec" aria-labelledby="set-ce-c">
            <div className="set-sec-head"><h2 id="set-ce-c">API specs</h2></div>
            <ul className="set-ce-specs">
              <li aria-hidden><span className="set-k">Spec file</span><span className="set-k">Matched</span><span className="set-k">Status</span></li>
              {specs.map((s) => {
                const bound = s.operations.filter((o) => o.handler).length;
                const gap = s.operations.length - bound;
                return (
                  <li key={s.id}>
                    <span><b className="set-mono">{s.file}</b><small>{KINDS[s.kind]} · {s.operations.length} {s.kind === "events" ? "channels" : "operations"}</small></span>
                    <span className="set-mono">{bound} / {s.operations.length}</span>
                    {gap ? <Chip tone="warn">{gap} without code</Chip> : <Chip tone="ok">In sync</Chip>}
                  </li>
                );
              })}
            </ul>
          </section>
          <section className="set-sec" aria-labelledby="set-ce-a">
            <h2 id="set-ce-a">Frameworks it recognises</h2>
            <div className="set-actions">{ADAPTERS.map((a) => <Chip key={a} tone="ok">{a}</Chip>)}</div>
            <p className="set-note"><b className="set-ce-warn">{UNRECOGNISED_ROUTES.count} route files weren&apos;t recognised</b> in <span className="set-mono">{UNRECOGNISED_ROUTES.where}</span>. They use a custom router. Map them in the override file, or ask for support.</p>
            <div className="set-actions">
              <button type="button" className="set-btn is-sm" aria-expanded={snippet} onClick={() => setSnippet(!snippet)}>Map in override file</button>
              <a className="set-btn is-sm" href="https://github.com/ax3cubed/cartograph/issues/new" target="_blank" rel="noreferrer">Ask for support</a>
            </div>
            {snippet && <pre className="set-pre">{`# cartograph.overrides.yaml\n- action: add\n  target: ${UNRECOGNISED_ROUTES.where}/router.ts\n  to: api-gateway\n  reason: Hand-rolled router; routes registered in router.ts.`}</pre>}
          </section>
        </div>

        <section className="set-sec" aria-labelledby="set-ce-e">
          <div className="set-sec-head"><h2 id="set-ce-e">Config values per environment</h2><Seg<EnvName> label="Environment" value={env} onChange={setEnv} options={[["prod", "prod"], ["staging", "staging"], ["local", "local"]]} /></div>
          <p className="set-note">Cartograph reads these from Helm values, Kubernetes config and <span className="set-mono">.env.example</span>, then uses them to follow calls to the right service. Secrets are never read, only key names. Edit a value to correct it for this environment.</p>
          <ul className="set-ce-keys" aria-live="polite">
            <li aria-hidden><span className="set-k">Config key</span><span className="set-k">Value</span><span className="set-k">Calls go to</span></li>
            {keys.map((k) => (
              <li key={k.key}>
                <span className="set-mono">{k.key}</span>
                <span><input className="set-input is-mono" aria-label={`Value for ${k.key} in ${env}`} value={f.value[at(k.key)] ?? k.value} onChange={(e) => setVal(k.key, k.value, e.target.value)} /><small>{f.value[at(k.key)] !== undefined ? "Edited" : k.from}</small></span>
                <Chip tone={k.tone}>{k.to}</Chip>
              </li>
            ))}
            <li>
              <label className="set-mono" htmlFor="set-ce-search">SEARCH_URL</label>
              <input id="set-ce-search" className="set-input is-mono" placeholder="http://search.svc" aria-label={`Value for SEARCH_URL in ${env}`} value={search} onChange={(e) => setVal("SEARCH_URL", "", e.target.value)} />
              {filled ? <Chip tone="acc">Resolves once saved</Chip> : <Chip tone="bad">Not matched · {CALL_SITES.searchUrl} calls</Chip>}
            </li>
          </ul>
          <div className="set-sec-head">
            <span className="set-note">{pct(resolved)}% of calls between services are matched to an operation.{!filled && ` Setting SEARCH_URL would raise that to ${pct(CALL_SITES.resolved + CALL_SITES.searchUrl)}%.`}</span>
          </div>
        </section>
      </div>
    </SettingsPage>
  );
}
