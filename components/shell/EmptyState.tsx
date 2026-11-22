import type { LucideIcon } from "lucide-react";
import Link from "next/link";
import "./shell.css";

/** Decision I9: say what will appear, why it isn't there yet, and offer the one action that fills it. */
export interface EmptyStateAction { label: string; href?: string; onClick?: () => void }

export default function EmptyState({ icon: Icon, title, body, action }: {
  icon?: LucideIcon; title: string; body: string; action?: EmptyStateAction;
}) {
  return (
    <div className="sh-empty" role="status">
      {Icon && <span className="sh-empty-icon"><Icon size={22} aria-hidden /></span>}
      <h2>{title}</h2>
      <p>{body}</p>
      {action && (action.href
        ? <Link className="btn primary" href={action.href}>{action.label}</Link>
        : <button type="button" className="btn primary" onClick={action.onClick}>{action.label}</button>)}
    </div>
  );
}
