"use client";

import { useMemo } from "react";
import { useRouter } from "next/navigation";
import { SquarePen } from "lucide-react";
import { getAskHistory } from "@/lib/source";
import Ask from "../Ask";
import ChatMenu from "./ChatMenu";
import ModeSwitch from "./ModeSwitch";
import { LATEST, activeThread, askStore, focusComposer, run, useAsk } from "./store";

const hrefFor = (id: string) => `/#focus=${encodeURIComponent(id)}&back=/ask`;
const answerHref = (turn: string) => `/#answer=${encodeURIComponent(turn)}&back=/ask`;
const DAY = 86_400_000;
const age = (ms: number) => {
  const d = Math.floor((Date.now() - ms) / DAY);
  return d < 1 ? "today" : d < 7 ? `${d}d` : d < 60 ? `${Math.floor(d / 7)}w` : `${Math.floor(d / 30)}mo`;
};
/** How many recent chats a new chat lists under the composer. */
const RECAP = 6;

/**
 * /ask: the chat with room for long sessions, composed like Claire's full page. One header row: the chat's
 * title opens the history menu (search, chats by age, earlier sessions); New chat and "Open in" on the right.
 * A new chat centres the greeting and composer, with recent chats below in place of the starters; once a
 * question is asked, the conversation takes a centred column and the composer pins to the bottom.
 * Evidence links to the map with a way back here.
 */
export default function AskPage() {
  const s = useAsk();
  const router = useRouter();
  const past = useMemo(() => getAskHistory(), []);
  const thread = activeThread(s);
  const newChat = () => { askStore.newChat(); focusComposer(); };
  const recent = thread ? [] : s.threads.slice(0, RECAP);

  return (
    <div className="ask-page">
      <section className={`ask-main${thread ? "" : " is-new"}${recent.length ? " has-recent" : ""}`} aria-label="Ask">
        <header className="ask-page-head">
          <h1 className="ask-sr">Ask</h1>
          <ChatMenu threads={s.threads} active={thread} live={!!s.live} onOpen={askStore.openThread} onDelete={askStore.deleteThread}
            past={past} onRerun={(question, asOf) => { askStore.newChat(); run(question, asOf ?? LATEST); }} />
          <span className="ask-page-gap" />
          <button type="button" className="ask-icon" aria-label="New chat" title="New chat" disabled={!thread} onClick={newChat}><SquarePen size={15} aria-hidden /></button>
          <ModeSwitch current="page" />
        </header>
        <Ask variant="page" t={LATEST} answer={null} onAnswer={() => {}} onOpen={(id) => router.push(hrefFor(id))}
          hrefFor={hrefFor} answerHref={answerHref} />
        {recent.length > 0 && (
          <nav className="ask-recap" aria-label="Recent chats">
            {recent.map((th) => (
              <button key={th.id} type="button" className="ask-recap-row" onClick={() => askStore.openThread(th.id)}>
                <span>{th.title}</span><time dateTime={new Date(th.at).toISOString()}>{age(th.at)}</time>
              </button>
            ))}
          </nav>
        )}
      </section>
    </div>
  );
}
