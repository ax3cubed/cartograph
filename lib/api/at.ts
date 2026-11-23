// The map's date picker works in local calendar days; the API's `at=` reads a bare YYYY-MM-DD as 00:00 UTC.

const pad = (n: number) => String(n).padStart(2, "0");

/** The local calendar day of `d`, as `<input type="date">` spells it. */
export const localDay = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** The last second of a picked local day, as an RFC 3339 instant with this zone's offset on that day (DST
 * included). Sent bare, the day would read as 00:00 UTC: its own commits left out, and the wrong day for
 * anyone not on UTC. */
export const endOfLocalDay = (day: string) => {
  const off = -new Date(`${day}T23:59:59`).getTimezoneOffset(); // minutes east of UTC
  const a = Math.abs(off);
  return `${day}T23:59:59${off >= 0 ? "+" : "-"}${pad(Math.floor(a / 60))}:${pad(a % 60)}`;
};
