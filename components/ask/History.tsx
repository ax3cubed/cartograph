"use client";

import { useId, useState } from "react";
import { History as HistoryIcon } from "lucide-react";
import type { AskTurn } from "@/lib/model";
import { byId, commitBySha, dayMs } from "@/lib/workspace";
import { modelName } from "@/lib/ai";
import { fmtDay, isoDay } from "./format";
import "./ask.css";

export type HistTurn = AskTurn & { asOf?: number };

/** Decision D15: an answer is stale when something it cited no longer exists in the index. */
export function staleReason(t: AskTurn): string | null {
  const gone = t.citations.find((c) => (c.node && !byId.has(c.node)) || (c.commit && !commitBySha.has(c.commit)));
  return gone ? `cites ${gone.node ?? gone.commit}, which is no longer in the index` : null;
}

const strip = (s: string) => s.replace(/\s*\[(?:e)?\d+\]/g, "");

function Turn({ turn, t, onRerun, onOpen }: { turn: HistTurn; t: number; onRerun: (q: string, asOf: number) => void; onOpen?: () => void }) {
  const id = useId();
  const [day, setDay] = useState(isoDay(turn.asOf ?? t));
  const stale = staleReason(turn);
  return (
    <li className="ask-turn">
      <b>{turn.question}</b>
      <p>{strip(turn.answer)}</p>
      <div className="ask-turn-row">
        <span className="ask-chip">{turn.model === "map" ? "Answered from the map" : `Worded by ${modelName(turn.model)}`}{turn.asOf ? ` · as of ${fmtDay(turn.asOf)}` : ""}</span>
        {stale && <span className="ask-chip warn" title={stale}>Out of date: {stale}</span>}
      </div>
      <div className="ask-turn-row">
        {onOpen && <button type="button" className="ask-small" onClick={onOpen}>Open answer</button>}
        <label htmlFor={id}>As of</label>
        <input id={id} type="date" value={day} onChange={(e) => setDay(e.target.value)} />
        <button type="button" className="ask-small" disabled={!day} onClick={() => onRerun(turn.question, dayMs(day))}>Re-run question</button>
      </div>
    </li>
  );
}

/** Questions from earlier sessions, folded at the top of the Ask thread. */
export default function History({ turns, t, onRerun, onOpen }: {
  turns: HistTurn[]; t: number; onRerun: (q: string, asOf: number) => void; onOpen?: (id: string) => void;
}) {
  if (!turns.length) return null;
  return (
    <details className="ask-history">
      <summary><HistoryIcon size={14} aria-hidden /> Past questions · {turns.length}</summary>
      <ul className="ask-history-list">
        {turns.map((x) => <Turn key={x.id} turn={x} t={t} onRerun={onRerun} onOpen={onOpen && x.id.startsWith("s") ? () => onOpen(x.id) : undefined} />)}
      </ul>
    </details>
  );
}
