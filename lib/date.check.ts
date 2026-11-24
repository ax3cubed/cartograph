// Run: pnpm check
import assert from "node:assert/strict";
import { fmtDate, fmtNum } from "./date.ts";

const t = Date.UTC(2025, 8, 3, 14, 5);
assert.equal(fmtDate(t, "dmy"), "3 Sep 2025");
assert.equal(fmtDate(t, "dm"), "3 Sep");
assert.equal(fmtDate(t, "my"), "Sep 2025");
assert.equal(fmtDate(t, "m"), "Sep");
assert.equal(fmtDate(t, "y"), "2025");
assert.equal(fmtDate(t, "md"), "Sep 3");
assert.equal(fmtDate(t, "full"), "Wednesday 3 September 2025");
assert.equal(fmtDate(t, "dmy-hm"), "3 Sep 2025, 14:05");
assert.equal(fmtDate("2025-09-03", "dmy"), "3 Sep 2025", "a bare day is that UTC day");
assert.equal(fmtDate(new Date(t), "dm"), "3 Sep");
assert.equal(fmtDate("nonsense", "dmy"), "");
assert.equal(fmtNum(8312), "8,312");
assert.equal(fmtNum(1234567), "1,234,567");
assert.equal(fmtNum(999), "999");
assert.equal(fmtNum(-60000), "-60,000");
assert.equal(fmtNum(0.5), "0.5");
console.log("date ok");
