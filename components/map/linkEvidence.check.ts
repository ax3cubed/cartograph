// XS-06: the inspector's evidence groups follow the payload. Run with `pnpm check`.
import assert from "node:assert/strict";
import { opText, roles, siteLabel } from "./linkEvidence.ts";
import type { EvidenceSite, LinkDetail } from "../../lib/api/types.gen.ts";

const site = (file: string, line: number): EvidenceSite => ({ file, line, end_line: line, kind: "template", extractor: "fetch@1.0.0", snippet: "fetch(`/x`)" });
const d = { evidence: { contract: [], caller: [site("web/src/api.ts", 2)], handler: [site("orders/src/server.ts", 4)] } } as unknown as LinkDetail;

assert.deepEqual(roles(d).map((g) => g.title), ["Caller", "Handler"], "an empty contract group is left out");
assert.equal(siteLabel(roles(d)[0].sites[0]), "web/src/api.ts:2");
assert.equal(opText({ method: "get", template: "/orders/{}" }), "GET /orders/{}");
assert.deepEqual(roles({ evidence: { contract: [], caller: [], handler: [] } } as unknown as LinkDetail), []);
console.log("linkEvidence.check: ok");
