"use client";

import { useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import Link from "next/link";
import { AlertTriangle, Bell, Hammer, RefreshCw, SlidersHorizontal } from "lucide-react";
import { getActivity } from "@/lib/source";
import type { ActivityItem, Freshness } from "@/lib/model";
import { usePopover } from "./usePopover";
import { ago } from "./ago";
import { behindText } from "./IndexStatus";
import { useLive } from "@/lib/api/live";
import { activityRows, cut, hasMoreActivity, loadActivity, markRead, moreActivity, onActivity, subscribeActivity, type Row } from "@/lib/api/events";
import { toast } from "./toast";
import "./activity.css";

const PAGE = 50;
const NONE: Row[] = [];
const ICON = { index: Hammer, sync: RefreshCw, correction: SlidersHorizontal, warning: AlertTriangle } as const;
const READ_KEY = "cartograph.activity-read"; // demo only: ids the user has read; live read state is on the server

const loadRead = (): string[] => { try { return JSON.parse(localStorage.getItem(READ_KEY) ?? "[]"); } catch { return []; } };
const saveRead = (ids: string[]) => { try { localStorage.setItem(READ_KEY, JSON.stringify(ids)); } catch { /* storage can be blocked */ } };

type Item = ActivityItem & { href?: string; detail?: string };

/** Decision D5: the bell, its unread count, and the activity list. Toasts (Toasts.tsx) are the live half. */
export default function Activity({ freshness }: { freshness?: Freshness }) {
  const { open, setOpen, ref } = usePopover();
  const live = useLive();
  // Read state of the rows this component adds itself (and of every demo row); a live row's is the server's.
  const [read, setRead] = useState<string[]>([]);
  const [shown, setShown] = useState(PAGE);
  useEffect(() => { if (!live) setRead(loadRead()); }, [live]);
  // FE-16: live rows come from GET /api/v1/activity and the event stream (lib/api/events.ts).
  const stored = useSyncExternalStore(subscribeActivity, activityRows, () => NONE);
  const more = useSyncExternalStore(subscribeActivity, hasMoreActivity, () => false);
  useEffect(() => { if (live) loadActivity(); }, [live]);
  useEffect(() => (live ? onActivity((r) => toast(cut(r.text))) : undefined), [live]);

  // A stale or failed index adds a row of its own, from the same Freshness as the chip.
  const items = useMemo<Item[]>(() => {
    const base: Item[] = live ? stored : getActivity().map((a) => (freshness && a.kind === "index" ? { ...a, at: freshness.builtAt } : a));
    const href = "/settings/indexing";
    if (freshness?.state === "behind") return [{ id: "live-behind", at: freshness.builtAt, kind: "warning", text: `The index is ${behindText(freshness)} commits behind HEAD. Run cartograph sync to update it.`, unread: true, href }, ...base];
    if (freshness?.state === "failed") return [{ id: "live-failed", at: freshness.builtAt, kind: "warning", text: "The index can't be read. Run cartograph sync; it resumes from the last batch.", unread: true, href }, ...base];
    return base;
  }, [freshness, live, stored]);
  const isUnread = (a: ActivityItem) => a.unread && !read.includes(a.id);
  const unread = items.filter(isUnread).length;
  const mark = (ids: string[], all = false) => {
    const next = [...new Set([...read, ...ids])]; setRead(next);
    if (!live) { saveRead(next); return; }
    const server = ids.filter((id) => stored.some((r) => r.id === id));
    if (all) markRead(); else if (server.length) markRead(server);
  };

  return (
    <div className="sh-pop-wrap" ref={ref}>
      <button type="button" className="sh-bell" aria-label={`Activity, ${unread} unread`} aria-haspopup="dialog" aria-expanded={open} onClick={() => setOpen(!open)}>
        <Bell size={17} aria-hidden />
        {unread > 0 && <span className="sh-bell-n" aria-hidden>{unread}</span>}
      </button>
      {open && (
        <section className="sh-pop sh-activity" role="dialog" aria-label="Activity">
          <header>
            <b>Activity</b>
            <button type="button" className="sh-link" disabled={!unread} onClick={() => mark(items.map((a) => a.id), true)}>Mark all as read</button>
          </header>
          {items.length === 0 ? <p className="sh-pop-note">Index builds, syncs and corrections show up here once they happen.</p> : (
            <ul>
              {items.slice(0, shown).map((a) => {
                const Icon = ICON[a.kind];
                // the text is untrusted (repo and author names, error messages): rendered as text, never as HTML
                const body: ReactNode = <><span className={`sh-activity-ic is-${a.kind}`}><Icon size={15} aria-hidden /></span><span className="sh-activity-t">{cut(a.text)}<small>{a.detail && <>{cut(a.detail)} · </>}{ago(a.at)}{isUnread(a) && " · unread"}</small></span></>;
                return (
                  <li key={a.id} className={isUnread(a) ? "is-unread" : undefined}>
                    {a.href
                      ? <Link href={a.href} className="sh-activity-row" onClick={() => { mark([a.id]); setOpen(false); }}>{body}</Link>
                      : <div className="sh-activity-row">{body}</div>}
                  </li>
                );
              })}
              {(items.length > shown || (live && more)) && <li><button type="button" className="sh-link" onClick={() => { setShown(shown + PAGE); if (live && items.length <= shown + PAGE) void moreActivity(); }}>Show more</button></li>}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}
