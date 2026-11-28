"use client";

// The sixth tab (FE-11, H6, D8): the architecture rules of cartograph.yaml, the violations each one has now, and
// whether every commit passes (…/rules, …/rules/violations, …/rules/commits). It has no board; it uses the Coverage
// page's table pattern. A rule's reason, the service and path names and commit messages are repo text: rendered as text.
import { useState } from "react";
import { ShieldCheck } from "lucide-react";
import type { Commit, Rule, RuleCommit, RuleViolation } from "@/lib/api";
import { usePaged } from "@/lib/api/paged";
import { useWsRead, wsRoute } from "@/lib/api/live";
import { Cell, Chip, Page, ViewSplit, fmtDate, levelLabel, levelTone, useHashState, useView } from "./shared";
import VGrid from "./VGrid";
import "./rules.css";

const KINDS: Record<string, string> = { forbid: "Forbid", no_cycles: "No cycles" };
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
/** What `cartograph check` would say: an error-severity rule with a violation no baseline accepts fails. */
const open = (r: Rule) => Math.max(0, r.violations - r.baselined);
const fails = (r: Rule) => r.severity === "error" && open(r) > 0;

const HEAD = { id: "rl", title: "Rules: which links the architecture forbids", purpose: "Each rule, the violations it has now, and whether every commit passes." };

export default function Rules() {
  const { ws, indexing } = useView();
  const res = useWsRead<Rule[]>(ws, "rules");
  const rules = res.data ?? [];
  const h = useHashState();
  const sel = rules.find((r) => r.id === h.get("rule")) ?? rules[0];
  const [shown, setShown] = useState<"live" | "all">("live");
  const violations = usePaged<RuleViolation>(ws, sel ? wsRoute(ws, "rules/violations") : null, { rule: sel?.id });
  const commits = usePaged<RuleCommit>(ws, rules.length ? wsRoute(ws, "rules/commits") : null);
  const failing = rules.filter(fails).length;
  const rows = shown === "live" ? violations.rows.filter((v) => v.live) : violations.rows;

  return (
    <Page head={HEAD} icon={ShieldCheck} indexing={indexing} when={!!res.data && rules.length === 0}
      empty={{ title: "No rules yet", body: "Rules say which links aren't allowed, such as a web service reading a table. Each one shows its current violations and whether every commit passes. Add a rules list to cartograph.yaml, then index again.", action: { label: "Open services settings", href: "/settings/services" } }}>
      <ViewSplit id="rl" title="Rules: which links the architecture forbids" asideLabel="Selected rule"
        purpose={res.error ? "The rules can't be read." : `${plural(rules.length, "rule")}, ${failing ? `${failing} failing` : "none failing"}. Pick one to see its violations.`}
        actions={<span aria-live="polite"><Chip tone={failing ? "bad" : "ok"}>{failing ? `${plural(failing, "rule")} failing` : "All rules pass"}</Chip></span>}
        aside={sel ? (
          <>
            <div className="rl-head"><h2>{sel.id}</h2><Chip tone={fails(sel) ? "bad" : "ok"}>{fails(sel) ? "Fail" : "Pass"}</Chip></div>
            <span className="view-sub">{KINDS[sel.kind] ?? sel.kind} · {sel.severity}. {plural(sel.violations, "violation")} now, {sel.baselined} accepted in the baseline.</span>
            {sel.description && <p className="view-note">{sel.description}</p>}
            <div role="group" aria-label="Violations shown" className="view-chips">
              <button type="button" className="view-filter" aria-pressed={shown === "live"} onClick={() => setShown("live")}>Holding now</button>
              <button type="button" className="view-filter" aria-pressed={shown === "all"} onClick={() => setShown("all")}>Ever held</button>
            </div>
            <VGrid rows={rows} rowId={(v) => `${v.kind}|${v.from}|${v.to}|${v.op ?? ""}`} label="Violations" cols="rl-viol" rowHeight={64} maxHeight={420}
              select={() => {}} hasMore={violations.hasMore} more={violations.more} loading={violations.loading}
              empty={<p className="view-note">{shown === "live" ? "No violation holds now." : "This rule has never been broken."}</p>}
              head={<><span role="columnheader">Link</span><span role="columnheader">State</span></>}
              render={(v) => (
                <>
                  <Cell className="rl-link"><span className="view-mono view-cell">{v.from}</span><span className="view-mono view-muted view-cell">→ {v.to}{v.op ? ` · ${v.op}` : ""}</span></Cell>
                  <Cell className="view-chips">
                    <Chip tone={v.live ? "bad" : ""}>{v.live ? "Holds" : "Fixed"}</Chip>
                    {v.baselined && <Chip tone="acc">Baselined</Chip>}
                    {v.level && <Chip tone={levelTone(v.level)}>{levelLabel(v.level)}</Chip>}
                  </Cell>
                </>
              )} />
          </>
        ) : <p className="view-note">{res.loading ? "Loading…" : "No rule is selected."}</p>}>
        {res.error ? <p className="view-sub" role="alert">{res.error.what}. {res.error.why}. Try: {res.error.try_}.</p> : (
          <>
            <VGrid rows={rules} rowId={(r) => r.id} label="Rules" cols="rl-cols" rowHeight={56} sel={sel?.id} select={(rule) => h.set({ rule })} hasMore={false} more={() => {}} loading={res.loading}
              head={<><span role="columnheader">Rule</span><span role="columnheader">Kind</span><span role="columnheader">Severity</span><span role="columnheader">Violations</span><span role="columnheader">Baselined</span><span role="columnheader">Status</span></>}
              render={(r) => (
                <>
                  <Cell className="view-mono">{r.id}</Cell>
                  <Cell>{KINDS[r.kind] ?? r.kind}</Cell>
                  <Cell>{r.severity}</Cell>
                  <Cell className="view-mono">{r.violations}</Cell>
                  <Cell className="view-mono">{r.baselined}</Cell>
                  <Cell><Chip tone={fails(r) ? "bad" : "ok"}>{fails(r) ? "Fail" : "Pass"}</Chip></Cell>
                </>
              )} />
            <p className="view-note">Arrow keys move between rules. Enter opens the details. A rule fails when its severity is error and a violation isn&apos;t in the baseline.</p>
            <h2 className="view-k">Commits, newest first: does the architecture pass after each one</h2>
            <VGrid rows={commits.rows} rowId={(c) => c.commit.sha} label="Commits" cols="rl-commit" rowHeight={52} maxHeight={420}
              select={() => {}} hasMore={commits.hasMore} more={commits.more} loading={commits.loading}
              empty={<p className="view-note">No commits are indexed yet.</p>}
              head={<><span role="columnheader">Commit</span><span role="columnheader">Message</span><span role="columnheader">Date</span><span role="columnheader">Result</span></>}
              render={(c) => <CommitCells c={c} />} />
          </>
        )}
      </ViewSplit>
    </Page>
  );
}

function CommitCells({ c }: { c: RuleCommit }) {
  const k: Commit = c.commit;
  return (
    <>
      <Cell className="view-mono">{k.sha.slice(0, 7)}</Cell>
      <Cell>{k.message}</Cell>
      <Cell className="view-muted">{fmtDate(k.date)}</Cell>
      <Cell><Chip tone={c.failing ? "bad" : "ok"}>{c.failing ? `Fail · ${c.failing}` : "Pass"}</Chip></Cell>
    </>
  );
}
