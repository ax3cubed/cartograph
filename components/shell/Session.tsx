"use client";

import { useEffect, type CSSProperties, type ReactNode } from "react";
import { Copy } from "lucide-react";
import { boot } from "@/lib/api/session";
import { clear } from "@/lib/api/cache";
import { startEvents } from "@/lib/api/events";
import { useMode } from "@/lib/api/live";
import { copy } from "./IndexStatus";
import "./session.css";

// Hidden, not unmounted, while boot decides: the page keeps its layout and shows nothing a token-less tab shouldn't.
const HIDE: CSSProperties = { display: "contents", visibility: "hidden" };

/**
 * FE-03: decides once per page whether this tab talks to the binary, and gates every route on it.
 * A tab the binary serves without a session token gets the "open from the CLI" screen and no data.
 */
export default function SessionGate({ children }: { children: ReactNode }) {
  const mode = useMode();
  useEffect(boot, []);
  useEffect(() => {
    if (mode === "live") return startEvents();
    if (mode === "no-session") clear();
  }, [mode]);
  if (mode === "no-session") return <OpenFromCli />;
  return <div style={mode === "checking" ? HIDE : { display: "contents" }}>{children}</div>;
}

function OpenFromCli() {
  const cmd = "cartograph open";
  return (
    <main className="ss-wrap">
      <section className="ss-card" aria-labelledby="ss-t">
        <h1 id="ss-t">Open Cartograph from the terminal</h1>
        <p>This tab has no session. Cartograph answers only the tab it opened, so another page or another user on this machine can&apos;t read your index.</p>
        <p>Run this, and the app opens in a new tab:</p>
        <div className="ss-cmd">
          <code><span aria-hidden>$ </span>{cmd}</code>
          <button type="button" className="btn" aria-label="Copy command" onClick={() => copy(cmd)}><Copy size={13} aria-hidden />Copy</button>
        </div>
        <p className="ss-note">If Cartograph restarted, its old tabs end up here too: each start uses a new session.</p>
      </section>
    </main>
  );
}
