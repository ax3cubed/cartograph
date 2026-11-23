"use client";

import { useEffect, useMemo, useState, type ReactNode } from "react";
import Link from "next/link";
import { AlertTriangle, Bell, Hammer, RefreshCw, SlidersHorizontal } from "lucide-react";
import { getActivity } from "@/lib/source";
import type { ActivityItem, Freshness } from "@/lib/model";
import { ACTIVITY_HREF } from "@/lib/demo/shell";
import { usePopover } from "./usePopover";
import { ago } from "./ago";
import { behindText } from "./IndexStatus";
import { useLive } from "@/lib/api/live";
import "./activity.css";

const ICON = { index: Hammer, sync: RefreshCw, correction: SlidersHorizontal, warning: AlertTriangle } as const;
const READ_KEY = "cartograph.activity-read"; // ids the user has read; read state survives reloads

const loadRead = (): string[] => { try { return JSON.parse(localStorage.getItem(READ_KEY) ?? "[]"); } catch { return []; } };
const saveRead = (ids: string[]) => { try { localStorage.setItem(READ_KEY, JSON.stringify(ids)); } catch { /* storage can be blocked */ } };

/** Decision D5: the bell, its unread count, and the activity list. Toasts (Toasts.tsx) are the live half. */
export default function Activity({ freshness }: { freshness?: Freshness }) {
  const { open, setOpen, ref } = usePopover();
  const [read, setRead] = useState<string[]>([]);
  useEffect(() => { setRead(loadRead()); }, []);

  // the build row takes its time from the same Freshness as the chip; a stale or failed index adds a row of its own.
  // Live, only those rows: the demo's corrections and syncs aren't this workspace's (the activity store is FE-14).
  const live = useLive();
  const items = useMemo<ActivityItem[]>(() => {
    const base = live ? [] : getActivity().map((a) => (freshness && a.kind === "index" ? { ...a, at: freshness.builtAt } : a));
    if (freshness?.state === "behind") return [{ id: "live-behind", at: freshness.builtAt, kind: "warning", text: `The index is ${behindText(freshness)} commits behind HEAD. Run cartograph sync to update it.`, unread: true }, ...base];
    if (freshness?.state === "failed") return [{ id: "live-failed", at: freshness.builtAt, kind: "warning", text: "The index can't be read. Run cartograph sync; it resumes from the last batch.", unread: true }, ...base];
    return base;
  }, [freshness, live]);
  const isUnread = (a: ActivityItem) => a.unread && !read.includes(a.id);
  const unread = items.filter(isUnread).length;
  const mark = (ids: string[]) => { const next = [...new Set([...read, ...ids])]; setRead(next); saveRead(next); };

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
            <button type="button" className="sh-link" disabled={!unread} onClick={() => mark(items.map((a) => a.id))}>Mark all as read</button>
          </header>
          {items.length === 0 ? <p className="sh-pop-note">Index builds, syncs and corrections show up here once they happen.</p> : (
            <ul>
              {items.map((a) => {
                const Icon = ICON[a.kind];
                const href = ACTIVITY_HREF[a.id] ?? (a.id === "live-behind" || a.id === "live-failed" ? "/settings/indexing" : undefined);
                const body: ReactNode = <><span className={`sh-activity-ic is-${a.kind}`}><Icon size={15} aria-hidden /></span><span className="sh-activity-t">{a.text}<small>{ago(a.at)}{isUnread(a) && " · unread"}</small></span></>;
                return (
                  <li key={a.id} className={isUnread(a) ? "is-unread" : undefined}>
                    {href
                      ? <Link href={href} className="sh-activity-row" onClick={() => { mark([a.id]); setOpen(false); }}>{body}</Link>
                      : <div className="sh-activity-row">{body}</div>}
                  </li>
                );
              })}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}
