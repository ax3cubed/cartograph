// One date formatter for the whole app. Month names are written here, never taken from Intl: Node's ICU spells
// September "Sept" in en-GB and WebKit's "Sep", and the static HTML must match what the browser renders (#418).

export const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
export const MONTHS_LONG = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS_LONG = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

export type DateStyle = "dmy" | "dm" | "my" | "m" | "y" | "md" | "full" | "dmy-hm";

const pad = (n: number) => String(n).padStart(2, "0");

/** `t` as ms, ISO text or a Date. A bare YYYY-MM-DD is that UTC day. */
export function fmtDate(t: number | string | Date, style: DateStyle, zone: "utc" | "local" = "utc"): string {
  const d = typeof t === "string" ? new Date(/^\d{4}-\d{2}-\d{2}$/.test(t) ? `${t}T00:00:00Z` : t) : new Date(t);
  if (Number.isNaN(d.getTime())) return "";
  const u = zone === "utc";
  const y = u ? d.getUTCFullYear() : d.getFullYear();
  const m = u ? d.getUTCMonth() : d.getMonth();
  const day = u ? d.getUTCDate() : d.getDate();
  switch (style) {
    case "dmy": return `${day} ${MONTHS[m]} ${y}`;
    case "dm": return `${day} ${MONTHS[m]}`;
    case "my": return `${MONTHS[m]} ${y}`;
    case "m": return MONTHS[m];
    case "y": return String(y);
    case "md": return `${MONTHS[m]} ${day}`;
    case "full": return `${DAYS_LONG[u ? d.getUTCDay() : d.getDay()]} ${day} ${MONTHS_LONG[m]} ${y}`;
    case "dmy-hm": return `${day} ${MONTHS[m]} ${y}, ${pad(u ? d.getUTCHours() : d.getHours())}:${pad(u ? d.getUTCMinutes() : d.getMinutes())}`;
  }
}

/** Thousands with commas: 8312 → "8,312". Fractions keep up to 2 digits. */
export function fmtNum(n: number): string {
  if (!Number.isFinite(n)) return String(n);
  const neg = n < 0;
  const [int, frac] = String(Math.round(Math.abs(n) * 100) / 100).split(".");
  return `${neg ? "-" : ""}${int.replace(/\B(?=(\d{3})+(?!\d))/g, ",")}${frac ? `.${frac}` : ""}`;
}
