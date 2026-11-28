//! XS-15: routes from the framework adapters link to the calls that reach them, with the controller,
//! router and group prefixes and the file-system path of Next.js joined into the route.

use cartograph_facts::FactCache;
use cartograph_git::fixture::Fixture;
use cartograph_index::structure::ServiceSpec;
use cartograph_index::sync::{SyncOptions, SyncOutcome, Workspace, sync};
use cartograph_index::timeline::RepoSpec;
use cartograph_store::Index;

const SERVERS: [(&str, &str); 5] = [
    (
        "orders/src/orders.controller.ts",
        "import { Controller, Get } from \"@nestjs/common\";\n@Controller(\"orders\")\nexport class C {\n  @Get(\":id\")\n  one() {}\n}\n",
    ),
    (
        "orders/app/api/items/[id]/route.ts",
        "export async function GET() { return Response.json({}); }\n",
    ),
    (
        "orders/billing.py",
        "from fastapi import APIRouter, FastAPI\napp = FastAPI()\nrouter = APIRouter(prefix=\"/billing\")\n\n@router.get(\"/{id}\")\ndef one(id: int):\n    return {}\n\napp.include_router(router, prefix=\"/api\")\n",
    ),
    (
        "orders/ship.go",
        "package main\n\nimport \"github.com/gin-gonic/gin\"\n\nfunc main() {\n\tr := gin.Default()\n\tv1 := r.Group(\"/v1\")\n\tv1.GET(\"/ship/:id\", h)\n}\n",
    ),
    (
        "orders/routes/api.php",
        "<?php\nRoute::get('/stock/{id}', [S::class, 'show']);\n",
    ),
];

const CALLS: [(&str, &str); 5] = [
    ("web/src/a.ts", "orders/"),
    ("web/src/b.ts", "api/items/"),
    ("web/src/c.ts", "api/billing/"),
    ("web/src/d.ts", "v1/ship/"),
    ("web/src/e.ts", "api/stock/"),
];

#[test]
fn prefixes_and_file_paths_make_the_route_the_caller_reaches() {
    let d = tempfile::tempdir().unwrap();
    let mut f = Fixture::init(&d.path().join("repo"));
    f.write(
        "docker-compose.yml",
        "services:\n  orders:\n    build: ./orders\n  web:\n    build: ./web\n",
    );
    f.write("orders/package.json", r#"{"name":"orders"}"#);
    f.write("web/package.json", r#"{"name":"web"}"#);
    for (p, src) in SERVERS {
        f.write(p, src);
    }
    for (p, path) in CALLS {
        f.write(
            p,
            &format!(
                "export const load = (id: string) => fetch(\"http://orders:8080/{path}\" + id);\n"
            ),
        );
    }
    f.commit("everything");
    let svc = |name: &str| ServiceSpec {
        name: name.into(),
        repo: "repo".into(),
        path: Some(name.into()),
    };
    let w = Workspace {
        dir: d.path().join("ws"),
        repos: vec![RepoSpec {
            name: "repo".into(),
            path: f.dir.clone(),
            ..Default::default()
        }],
        services: vec![svc("orders"), svc("web")],
        ..Default::default()
    };
    let mut cache = FactCache::open(&d.path().join("facts.db")).unwrap();
    match sync(&w, &SyncOptions::default(), &mut cache).unwrap() {
        SyncOutcome::Synced(_) => {}
        SyncOutcome::AlreadyRunning => panic!("nothing else is running"),
    }
    let ix = Index::open_read(&w.index_path()).unwrap();
    let mut got: Vec<String> = ix
        .conn
        .prepare(
            "SELECT (SELECT path FROM file_path p WHERE p.file = ns.file LIMIT 1) || ' -> ' ||
                    (SELECT path FROM file_path p WHERE p.file = nd.file LIMIT 1)
             FROM link l JOIN node ns ON ns.id = l.src JOIN node nd ON nd.id = l.dst
             WHERE l.kind = 'http'",
        )
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    got.sort();
    assert_eq!(
        got,
        [
            "web/src/a.ts -> orders/src/orders.controller.ts",
            "web/src/b.ts -> orders/app/api/items/[id]/route.ts",
            "web/src/c.ts -> orders/billing.py",
            "web/src/d.ts -> orders/ship.go",
            "web/src/e.ts -> orders/routes/api.php",
        ]
    );
}
