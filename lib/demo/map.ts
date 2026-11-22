// Links the indexer scored below 0.30. They are hidden on the map until "Show weak links" is on.
import { confidence, type Link } from "../model.ts";

export const WEAK_LINKS: Link[] = [
  { from: "notify/channels/sms.ts", to: "auth/users/repo.go", type: "calls", added: "ab83e1d",
    confidence: confidence(0.24, { kind: "name-match", note: "A template variable named user matches a field in the user repo", file: "notifications/src/channels/sms.ts", line: 22 }) },
  { from: "gateway/routes/products.ts", to: "catalog/pricing/rules.py", type: "calls", added: "4c81a9e",
    confidence: confidence(0.27, { kind: "name-match", note: "Both mention a discount code; no import or request links them", file: "api-gateway/src/routes/products.ts", line: 40 }) },
];
