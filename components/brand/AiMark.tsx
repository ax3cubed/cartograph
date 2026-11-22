// Cartograph AI mark: the brand dial (open ring, spoke, accent node) drawn as a speech bubble.
// "Answers drawn from the map". One 2px stroke on a 24 grid; reads from 14px up.
// The accent node takes `--aimark-accent` (default `var(--accent)`); set it to `currentColor`
// on accent-filled grounds. `active` draws the spoke out to the accent node while an answer streams.
import type { SVGProps } from "react";
import "./aimark.css";

export interface AiMarkProps extends Omit<SVGProps<SVGSVGElement>, "children"> {
  size?: number;
  /** Subtle motion while an answer streams. Respects reduced motion. */
  active?: boolean;
  /** Accessible name; omit when decorative (next to a text label). */
  title?: string;
}

/** Ring open on the right like the brand mark, with a chat tail at the lower left. */
const BUBBLE = "M18.66 5.34A8 8 0 1 0 6.45 15.59L3.5 21l6.76-2.48A8 8 0 0 0 18.66 16.66";

export function AiMark({ size = 16, active, title, className, ...rest }: AiMarkProps) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={2}
      strokeLinecap="round" strokeLinejoin="round"
      className={className ? `aimark ${className}` : "aimark"}
      role={title ? "img" : undefined} aria-hidden={title ? undefined : true} data-active={active || undefined} {...rest}>
      {title && <title>{title}</title>}
      <path d={BUBBLE} />
      <path className="aimark-spoke" d="M13 11 18.66 5.34" pathLength={1} />
      <circle cx="13" cy="11" r="2.5" fill="currentColor" stroke="none" />
      <circle className="aimark-node" cx="18.66" cy="5.34" r="2.25" stroke="none" />
    </svg>
  );
}

export default AiMark;
