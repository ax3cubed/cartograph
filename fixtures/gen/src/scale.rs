//! I1's scale fixture as an index: 62 services, 1,140 modules, 60,000 files and 200,000 live
//! links, written straight into the tables the readers use (no git history behind it), at one
//! commit. The file `InvoiceRepo.ts` (node 14242) is among them. Used by the performance gates
//! (HD-09) and the MCP output budget (SRV-15).

use std::path::Path;

/// I1's fixture as an index: 62 services, 1,140 modules, 60,000 files and 200,000 live links,
/// written straight into the tables the layout and search read. The file `InvoiceRepo.ts` is
/// among them. Returns the open connection.
pub fn index(path: &Path) -> cartograph_store::Index {
    use cartograph_store::rusqlite::params;
    const SERVICES: i64 = 62;
    const MODULES: i64 = 1_140;
    const FILES: i64 = 60_000;
    const LINKS: i64 = 200_000;
    let ix = cartograph_store::Index::create(path).unwrap();
    let c = &ix.conn;
    c.execute_batch(
        "BEGIN; INSERT INTO repo (id, name, path, shallow) VALUES (1, 'scale', '/x', 0);
                     INSERT INTO commits VALUES (0, 1, x'00', 0, 0, 'a', 's');",
    )
    .unwrap();
    let mut rng = 0x5eed_u64;
    let mut next = |n: i64| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % n as u64) as i64
    };
    for s in 0..SERVICES {
        c.execute(
            "INSERT INTO node (id, kind, key, name, service) VALUES (?1, 'service', ?2, ?3, ?1)",
            params![1 + s, format!("service:scale:svc{s}"), format!("svc{s}")],
        )
        .unwrap();
        c.execute("INSERT INTO node_span VALUES (?1, 0, NULL)", [1 + s])
            .unwrap();
    }
    let module_id = |m: i64| 1_000 + m;
    for m in 0..MODULES {
        let s = m % SERVICES;
        c.execute(
            "INSERT INTO node (id, kind, key, name, parent, service) VALUES (?1, 'module', ?2, ?3, ?4, ?4)",
            params![module_id(m), format!("module:svc{s}/m{m}"), format!("m{m}"), 1 + s],
        )
        .unwrap();
        c.execute("INSERT INTO node_span VALUES (?1, 0, NULL)", [module_id(m)])
            .unwrap();
    }
    let file_node = |f: i64| 10_000 + f;
    for f in 0..FILES {
        let m = f % MODULES;
        // Names as code has them: a domain word, a role and a suffix, so about one file in 40
        // starts with "Invoice" and a few hundred share each name in different folders.
        const DOMAIN: [&str; 40] = [
            "Invoice", "Order", "Payment", "Customer", "Account", "Ledger", "Refund", "Cart",
            "Price", "Tax", "Shipment", "Stock", "Product", "Catalog", "Coupon", "Session", "User",
            "Token", "Audit", "Report", "Email", "Webhook", "Queue", "Event", "Config", "Cache",
            "Search", "Index", "Upload", "Image", "Locale", "Theme", "Route", "Layout", "Form",
            "Table", "Chart", "Export", "Import", "Billing",
        ];
        const ROLE: [&str; 12] = [
            "Service",
            "Store",
            "Client",
            "Handler",
            "Mapper",
            "Model",
            "View",
            "Controller",
            "Utils",
            "Types",
            "Hooks",
            "Schema",
        ];
        let name = if f == 4_242 {
            "InvoiceRepo.ts".to_owned()
        } else {
            format!(
                "{}{}.ts",
                DOMAIN[next(40) as usize],
                ROLE[next(12) as usize]
            )
        };
        let path = format!("svc{}/m{m}/{name}", m % SERVICES);
        c.execute("INSERT INTO file (id, repo) VALUES (?1, 1)", [f])
            .unwrap();
        c.execute(
            "INSERT INTO node (id, kind, file, parent, service) VALUES (?1, 'file', ?2, ?3, ?4)",
            params![file_node(f), f, module_id(m), 1 + m % SERVICES],
        )
        .unwrap();
        c.execute(
            "INSERT INTO file_path (file, path, from_seq) VALUES (?1, ?2, 0)",
            params![f, path],
        )
        .unwrap();
    }
    for l in 0..LINKS {
        let (a, b) = (next(FILES), next(FILES));
        // Most links stay within a module, as code does.
        let b = if l % 3 == 0 {
            b
        } else {
            a - a % MODULES + b % MODULES.min(FILES - (a - a % MODULES))
        };
        let added = c
            .execute(
                "INSERT OR IGNORE INTO link (id, kind, src, dst) VALUES (?1, 'import', ?2, ?3)",
                params![l, file_node(a), file_node(b)],
            )
            .unwrap();
        if added == 1 {
            c.execute("INSERT INTO interval (link, from_seq) VALUES (?1, 0)", [l])
                .unwrap();
        }
    }
    c.execute_batch("COMMIT").unwrap();
    ix
}

/// [`index`] with its search rows written as the indexer writes them, and the per-node fan-in.
pub fn indexed(path: &Path) -> cartograph_store::Index {
    let ix = index(path);
    let c = &ix.conn;
    c.execute_batch("BEGIN").unwrap();
    let rows: Vec<(i64, String)> = c
        .prepare("SELECT n.id, fp.path FROM node n JOIN file_path fp ON fp.file = n.file")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    for (id, path) in &rows {
        cartograph_store::search::put(c, *id, path.rsplit('/').next().unwrap(), path).unwrap();
    }
    c.execute_batch(
        "CREATE TEMP TABLE fi AS SELECT dst, count(*) AS n FROM link GROUP BY dst;
         CREATE INDEX temp.fi_dst ON fi (dst);
         UPDATE node SET fan_in = coalesce((SELECT n FROM fi WHERE fi.dst = node.id), 0);
         DROP TABLE fi;
         INSERT OR REPLACE INTO meta (key, value) VALUES ('state', 'complete');
         COMMIT; PRAGMA wal_checkpoint(TRUNCATE);",
    )
    .unwrap();
    ix
}
