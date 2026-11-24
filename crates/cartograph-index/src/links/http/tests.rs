use proptest::prelude::*;

use super::*;

/// (route pattern, normalised path): Express, Next, Flask, Django, FastAPI, Go, Gin, chi, Spring
/// and template-literal spellings.
const ROUTES: &[(&str, &str)] = &[
    // Express
    ("/orders/:id", "/orders/{}"),
    ("/orders/:id?", "/orders/{}"),
    ("/users/:uid/posts/:pid", "/users/{}/posts/{}"),
    ("/files/*", "/files/{*}"),
    ("/orders/", "/orders"),
    ("//orders//x", "/orders/x"),
    ("/a/:id(\\d+)", "/a/{}"),
    ("/ab+cd", "/{*}"),
    ("/", "/"),
    ("", "/"),
    ("*", "/{*}"),
    ("/v1/:a-:b", "/v1/{}"),
    // Next
    ("/blog/[slug]", "/blog/{}"),
    ("/docs/[...slug]", "/docs/{*}"),
    ("/docs/[[...slug]]", "/docs/{*}"),
    ("/api/users/[id]/posts", "/api/users/{}/posts"),
    ("/api/[a]/[b]", "/api/{}/{}"),
    // Flask
    ("/users/<id>", "/users/{}"),
    ("/users/<int:id>", "/users/{}"),
    ("/f/<path:p>", "/f/{*}"),
    ("/u/<string:name>/x", "/u/{}/x"),
    ("/u/<uuid:id>/", "/u/{}"),
    // Django
    ("orders/<int:pk>/", "/orders/{}"),
    ("^orders/(?P<id>\\d+)/$", "/orders/{*}"),
    ("^$", "/"),
    ("^api/v1/items/$", "/api/v1/items"),
    ("^(?P<a>x)/y", "/{*}"),
    ("items/<slug:s>/edit", "/items/{}/edit"),
    // FastAPI
    ("/items/{item_id}", "/items/{}"),
    ("/files/{file_path:path}", "/files/{*}"),
    ("/items/{id:int}", "/items/{}"),
    ("/a/{b}/c/{d}", "/a/{}/c/{}"),
    // Go mux, chi, gin
    ("/orders/{id}", "/orders/{}"),
    ("/static/{path...}", "/static/{*}"),
    ("/{$}", "/"),
    ("/a/{id:[0-9]+}", "/a/{}"),
    ("/a/{rest:.*}", "/a/{*}"),
    ("/a/{rest:.+}", "/a/{*}"),
    ("/users/:id", "/users/{}"),
    ("/static/*filepath", "/static/{*}"),
    ("/health", "/health"),
    ("/api/v1/ping/", "/api/v1/ping"),
    // Spring
    ("/api/orders/{id}", "/api/orders/{}"),
    ("/api/**", "/api/{*}"),
    ("/api/{*path}", "/api/{*}"),
    ("/orders/{id}/items/{itemId}", "/orders/{}/items/{}"),
    ("/orders/{id:\\d+}", "/orders/{}"),
    ("/x/item-{id}", "/x/{}"),
    // template literals and format verbs
    ("/orders/${id}", "/orders/{}"),
    ("/a/${x/y}/b", "/a/{}/b"),
    ("/orders/%s", "/orders/{}"),
    ("/orders/%d/items", "/orders/{}/items"),
    ("/orders/${a + b}/x", "/orders/{}/x"),
    ("/a/${`${b}`}/c", "/a/{}/c"),
    ("/a/%s/b/%v", "/a/{}/b/{}"),
    // literals stay as they are
    ("/Orders/List", "/Orders/List"),
    ("/a.b/c_d/e-f", "/a.b/c_d/e-f"),
    ("/%7Euser", "/%7Euser"),
    ("/a/{*}/b", "/a/{*}"),
    ("/{}/x", "/{}/x"),
];

/// (call URL, normalised path): scheme, host, query and fragment dropped.
const CALLS: &[(&str, &str)] = &[
    ("http://orders:8080/orders/1?x=1#f", "/orders/1"),
    ("https://Example.com//a//b/", "/a/b"),
    ("http://orders", "/"),
    ("http://orders/", "/"),
    ("/orders/1/", "/orders/1"),
    ("orders/1", "/orders/1"),
    ("/orders?id=1", "/orders"),
    ("/orders#top", "/orders"),
    ("https://user:pw@api.example.com:8443/v1/x", "/v1/x"),
    ("http://a/b?c=http://d/e", "/b"),
    ("/orders/${id}", "/orders/{}"),
    ("/orders/%s?x=1", "/orders/{}"),
];

#[test]
fn routes_normalise_to_their_golden_form() {
    for (raw, want) in ROUTES {
        assert_eq!(normalise_route("GET", raw).path(), *want, "{raw:?}");
    }
}

#[test]
fn calls_normalise_to_their_golden_form() {
    for (raw, want) in CALLS {
        assert_eq!(normalise("Get", raw).path(), *want, "{raw:?}");
    }
    assert_eq!(normalise("GET", "http://x/a/b").to_string(), "get /a/b");
}

/// (call URL, route pattern, full / partial / none)
const PAIRS: &[(&str, &str, &str)] = &[
    // Express
    ("/orders/1", "/orders/:id", "full"),
    ("/orders/${id}", "/orders/:id", "full"),
    ("/orders", "/orders/:id", "none"),
    ("/orders/1/items", "/orders/:id", "none"),
    ("/orders/1", "/orders", "none"),
    ("/orders", "/orders", "full"),
    ("/orders/", "/orders", "full"),
    ("/Orders", "/orders", "none"),
    ("/orders/new", "/orders/:id", "full"),
    ("/files/a/b/c", "/files/*", "partial"),
    ("/files/a", "/files/*", "partial"),
    ("/files", "/files/*", "none"),
    ("/other/a", "/files/*", "none"),
    ("/anything", "*", "partial"),
    ("/", "*", "none"),
    ("/users/1/posts/2", "/users/:uid/posts/:pid", "full"),
    ("/users/1/posts", "/users/:uid/posts/:pid", "none"),
    ("/users/1/comments/2", "/users/:uid/posts/:pid", "none"),
    ("http://orders:8080/orders/${id}", "/orders/:id", "full"),
    ("/orders/1?expand=items", "/orders/:id", "full"),
    // Next
    ("/blog/hello", "/blog/[slug]", "full"),
    ("/blog", "/blog/[slug]", "none"),
    ("/blog/a/b", "/blog/[slug]", "none"),
    ("/docs/a/b/c", "/docs/[...slug]", "partial"),
    ("/docs/a", "/docs/[[...slug]]", "partial"),
    ("/docs", "/docs/[...slug]", "none"),
    ("/api/users/7/posts", "/api/users/[id]/posts", "full"),
    ("/api/users/7", "/api/users/[id]/posts", "none"),
    ("/api/${a}/${b}", "/api/[a]/[b]", "full"),
    ("/api/x", "/api/[a]/[b]", "none"),
    // Flask
    ("/users/5", "/users/<id>", "full"),
    ("/users/5", "/users/<int:id>", "full"),
    ("/users", "/users/<int:id>", "none"),
    ("/f/a/b/c", "/f/<path:p>", "partial"),
    ("/f", "/f/<path:p>", "none"),
    ("/u/bob/x", "/u/<string:name>/x", "full"),
    ("/u/bob/y", "/u/<string:name>/x", "none"),
    ("/u/bob", "/u/<string:name>/x", "none"),
    ("/u/1", "/u/<uuid:id>/", "full"),
    // Django
    ("/orders/9", "orders/<int:pk>/", "full"),
    ("/orders/9/", "orders/<int:pk>/", "full"),
    ("/orders", "orders/<int:pk>/", "none"),
    ("/orders/9", "^orders/(?P<id>\\d+)/$", "partial"),
    ("/orders", "^orders/(?P<id>\\d+)/$", "none"),
    ("/other/9", "^orders/(?P<id>\\d+)/$", "none"),
    ("/", "^$", "full"),
    ("/x", "^$", "none"),
    ("/api/v1/items", "^api/v1/items/$", "full"),
    ("/api/v1/items/1", "^api/v1/items/$", "none"),
    ("/items/a/edit", "items/<slug:s>/edit", "full"),
    ("/items/a", "items/<slug:s>/edit", "none"),
    // FastAPI
    ("/items/3", "/items/{item_id}", "full"),
    ("/items", "/items/{item_id}", "none"),
    ("/items/3/x", "/items/{item_id}", "none"),
    ("/files/a/b", "/files/{file_path:path}", "partial"),
    ("/files", "/files/{file_path:path}", "none"),
    ("/items/3", "/items/{id:int}", "full"),
    ("/a/1/c/2", "/a/{b}/c/{d}", "full"),
    ("/a/1/d/2", "/a/{b}/c/{d}", "none"),
    // Go mux, chi, gin
    ("/orders/1", "/orders/{id}", "full"),
    ("/orders", "/orders/{id}", "none"),
    ("/static/a/b.css", "/static/{path...}", "partial"),
    ("/static", "/static/{path...}", "none"),
    ("/", "/{$}", "full"),
    ("/x", "/{$}", "none"),
    ("/a/12", "/a/{id:[0-9]+}", "full"),
    ("/a/x/y", "/a/{rest:.*}", "partial"),
    ("/a", "/a/{rest:.*}", "none"),
    ("/users/9", "/users/:id", "full"),
    ("/static/js/app.js", "/static/*filepath", "partial"),
    ("/health", "/health", "full"),
    ("/healthz", "/health", "none"),
    ("/api/v1/ping", "/api/v1/ping/", "full"),
    // Spring
    ("/api/orders/4", "/api/orders/{id}", "full"),
    ("/api/orders", "/api/orders/{id}", "none"),
    ("/api/a/b/c", "/api/**", "partial"),
    ("/api", "/api/**", "none"),
    ("/api/a", "/api/{*path}", "partial"),
    ("/orders/1/items/2", "/orders/{id}/items/{itemId}", "full"),
    ("/orders/1/items", "/orders/{id}/items/{itemId}", "none"),
    ("/x/item-9", "/x/item-{id}", "full"),
    ("/x/other", "/x/item-{id}", "full"),
    // template literals and formats
    ("/orders/${id}/items", "/orders/%d/items", "full"),
    ("/orders/%s", "/orders/${id}", "full"),
    ("/a/${x}/c", "/a/${`${b}`}/c", "full"),
    ("/a/%s/b/%v", "/a/${x}/b/${y}", "full"),
    // a literal never matches another literal
    ("/orders/1", "/orders/2", "none"),
    ("/a/b/c", "/a/b/d", "none"),
    ("/a/b", "/b/a", "none"),
    // parameters on both sides
    ("/a/${x}", "/a/b", "full"),
    ("/a/b", "/a/{x}", "full"),
    ("/${x}/${y}", "/a/b", "full"),
    ("/${x}/${y}", "/a/b/c", "none"),
    ("/${x}", "/{*}", "partial"),
    ("/a/b/c", "/a/{*}", "partial"),
    ("/a/b/c", "/{*}", "partial"),
    ("/b/b/c", "/a/{*}", "none"),
    // the root
    ("/", "/", "full"),
    ("", "/", "full"),
    ("/x", "/", "none"),
    ("/", "/x", "none"),
    // query and fragment never count
    ("/orders?a=/b/c", "/orders", "full"),
    ("/orders#/b/c", "/orders", "full"),
    // more across frameworks
    ("/v2/prices/9", "/v2/prices/{sku}", "full"),
    ("/v2/prices", "/v2/prices/{sku}", "none"),
    ("/v1/prices/9", "/v2/prices/{sku}", "none"),
    ("/accounts/1/balance", "/accounts/<int:id>/balance", "full"),
    ("/accounts/1/balance/", "/accounts/:id/balance", "full"),
    ("/accounts/1", "/accounts/:id/balance", "none"),
    ("/search?q=a", "/search", "full"),
    ("/search/x", "/search", "none"),
    ("/login", "/login/", "full"),
    ("/logout", "/login/", "none"),
    ("/admin/users/3/roles", "/admin/users/{id}/roles", "full"),
    ("/admin/users/3/roles/4", "/admin/users/{id}/roles", "none"),
    ("/admin/x", "/admin/{*}", "partial"),
    ("/admin", "/admin/{*}", "none"),
    ("/api/${v}/ping", "/api/:version/ping", "full"),
    ("/api/${v}/ping", "/api/v1/pong", "none"),
    ("/graphql", "/graphql", "full"),
    ("/graphql/x", "/graphql", "none"),
    ("/hooks/stripe", "/hooks/<name>", "full"),
    ("/hooks", "/hooks/<name>", "none"),
    ("/media/a/b.png", "/media/<path:file>", "partial"),
    ("/media", "/media/<path:file>", "none"),
    ("/t/9/edit", "/t/[id]/edit", "full"),
    ("/t/9/edit/x", "/t/[id]/edit", "none"),
    ("/p/1/2/3", "/p/[...all]", "partial"),
    ("/events/1/stream", "/events/:id/stream", "full"),
    ("/events/stream", "/events/:id/stream", "none"),
    ("/x/y", "/x/{y:\\d+}", "full"),
    ("/x", "/x/{y:\\d+}", "none"),
    ("/pets/rex", "/pets/{name}", "full"),
    ("/pets/rex/toys", "/pets/{name}/toys", "full"),
    ("/pets/rex/food", "/pets/{name}/toys", "none"),
    ("/co/1", "/co/%s", "full"),
    ("/co", "/co/%s", "none"),
    // long literals
    ("/a/b/c/d/e/f/g", "/a/b/c/d/e/f/g", "full"),
    ("/a/b/c/d/e/f/g", "/a/b/c/d/e/f/h", "none"),
    ("/a/b/c/d/e/f/g", "/a/b/c/d/e/f/:x", "full"),
    ("/a/b/c/d/e/f", "/a/b/c/d/e/f/:x", "none"),
];

fn verdict(call: &str, route: &str) -> &'static str {
    let c = normalise("get", call);
    let r = normalise_route("get", route);
    match match_segs(&c.segs, &r.segs) {
        Some((Match::Full, _)) => "full",
        Some((Match::Partial, _)) => "partial",
        None => "none",
    }
}

#[test]
fn template_pairs_match_as_the_golden_table_says() {
    for (call, route, want) in PAIRS {
        assert_eq!(verdict(call, route), *want, "{call:?} against {route:?}");
    }
    assert!(ROUTES.len() + CALLS.len() + PAIRS.len() >= 200);
}

#[test]
fn a_call_with_an_unknown_prefix_matches_the_end_of_a_route() {
    let t = |parts: &[(bool, &str)]| call_target(parts).unwrap();
    let c = t(&[(true, "base"), (false, "/orders")]);
    assert!(c.prefix_unknown && c.host.is_none());
    let route = |p: &str| normalise_route("get", p).segs;
    assert!(match_suffix(&c.segs, &route("/api/v1/orders")).is_some());
    assert!(match_suffix(&c.segs, &route("/orders")).is_some());
    assert!(match_suffix(&c.segs, &route("/orders/:id")).is_none());
    assert!(match_suffix(&c.segs, &route("/api/{*}")).is_none());
}

#[test]
fn a_literal_host_is_split_from_the_path() {
    let t = call_target(&[(false, "http://Orders:8080/orders/"), (true, "id")]).unwrap();
    assert_eq!(
        t.host,
        Some(Host {
            name: "orders".into(),
            port: Some(8080)
        })
    );
    assert_eq!(path_string(&t.segs), "/orders/{}");
    assert!(!t.prefix_unknown);
    let t = call_target(&[(false, "http://"), (true, "host"), (false, "/x")]).unwrap();
    assert!(t.host.is_none() && t.prefix_unknown);
    assert!(call_target(&[(true, "url")]).is_none());
    assert!(call_target(&[(false, "http://[::1]/x")]).is_none());
}

#[test]
fn mounts_join_and_a_catch_all_prefix_swallows_the_rest() {
    let p = normalise_route("use", "/api").segs;
    let r = normalise_route("get", "/orders/:id").segs;
    assert_eq!(path_string(&join_mount(&p, &r)), "/api/orders/{}");
    let p = normalise_route("use", "/api/*").segs;
    assert_eq!(path_string(&join_mount(&p, &r)), "/api/{*}");
}

#[test]
fn caps_hold() {
    let long = "/a".repeat(500);
    let t = normalise_path(&long, false);
    assert!(t.len() <= MAX_SEGMENTS);
    assert_eq!(t.last(), Some(&Seg::Rest));
    let wide = format!("/{}", "x".repeat(5000));
    let t = normalise_path(&wide, false);
    assert!(path_string(&t).len() <= MAX_TEMPLATE);
}

fn pattern() -> impl Strategy<Value = String> {
    prop_oneof![
        "[ -~]{0,120}",
        "[a-z0-9/:{}<>\\[\\]*.$%?#_\\\\^()|+ -]{0,160}",
        prop::collection::vec(
            prop_oneof![
                Just(":id".to_owned()),
                Just("{id}".to_owned()),
                Just("<int:id>".to_owned()),
                Just("[id]".to_owned()),
                Just("${x}".to_owned()),
                Just("%s".to_owned()),
                Just("*".to_owned()),
                Just("[...slug]".to_owned()),
                Just("{path:.*}".to_owned()),
                "[a-z]{1,6}",
            ],
            0..80
        )
        .prop_map(|v| format!("/{}", v.join("/"))),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn normalise_is_idempotent(p in pattern(), m in "[A-Za-z]{0,20}") {
        for t in [normalise(&m, &p), normalise_route(&m, &p)] {
            prop_assert_eq!(normalise(&t.method, &t.path()), t.clone());
            prop_assert_eq!(normalise_route(&t.method, &t.path()), t.clone());
            prop_assert!(t.segs.len() <= MAX_SEGMENTS);
            prop_assert!(t.path().len() <= MAX_TEMPLATE);
            // A catch-all is only ever last.
            if let Some(i) = t.segs.iter().position(|s| *s == Seg::Rest) {
                prop_assert_eq!(i, t.segs.len() - 1);
            }
        }
    }

    /// Fuzz: any text, any length, never panics, never over the caps.
    #[test]
    fn normaliser_survives_anything(s in "\\PC{0,4000}") {
        let t = normalise("get", &s);
        prop_assert!(t.path().len() <= MAX_TEMPLATE);
        let r = normalise_route("get", &s);
        prop_assert!(r.segs.len() <= MAX_SEGMENTS);
        let _ = call_target(&[(false, &s), (true, "x"), (false, &s)]);
    }

    #[test]
    fn a_template_matches_itself(p in pattern()) {
        let t = normalise_route("get", &p);
        prop_assert!(match_segs(&t.segs, &t.segs).is_some());
        let c = normalise("get", &p);
        prop_assert!(match_segs(&c.segs, &c.segs).is_some());
    }
}

// ---- the resolver ----

#[derive(Default)]
struct Log {
    open: BTreeSet<Pair>,
    unresolved: BTreeSet<SiteKey>,
}

impl LinkSink for Log {
    fn open(&mut self, p: &Pair, _: Match) {
        assert!(self.open.insert(p.clone()), "opened twice");
    }
    fn close(&mut self, p: &Pair) {
        assert!(self.open.remove(p), "closed what was not open");
    }
    fn unresolved(&mut self, c: &SiteKey, _: Reason) {
        assert!(self.unresolved.insert(c.clone()));
    }
    fn unresolved_closed(&mut self, c: &SiteKey) {
        assert!(self.unresolved.remove(c));
    }
}

fn key(file: i64, line: u32, kind: Kind) -> SiteKey {
    SiteKey {
        file,
        span: [line, 0, line, 9],
        kind,
    }
}

fn call(file: i64, line: u32, service: i64, method: &str, url: &[(bool, &str)]) -> Site {
    Site::Call(CallSite {
        key: key(file, line, Kind::Call),
        service,
        method: method.into(),
        target: call_target(url).expect("a target"),
    })
}

fn route(file: i64, line: u32, service: i64, method: &str, pattern: &str) -> Site {
    Site::Route(RouteSite {
        key: key(file, line, Kind::Route),
        service,
        method: method.into(),
        segs: normalise_route(method, pattern).segs,
    })
}

fn alias(host: &str, port: Option<u16>, service: i64) -> Alias {
    Alias {
        host: host.into(),
        port,
        service,
    }
}

fn step(r: &mut HttpResolver, log: &mut Log, d: Delta) {
    r.apply(0, &d, log);
}

#[test]
fn a_host_picks_the_service_and_the_route_in_it() {
    let mut r = HttpResolver::new();
    let mut log = Log::default();
    step(
        &mut r,
        &mut log,
        Delta {
            add: vec![
                route(1, 3, 10, "get", "/orders/:id"),
                route(2, 3, 20, "get", "/orders/:id"),
                call(
                    3,
                    5,
                    30,
                    "get",
                    &[(false, "http://orders:8080/orders/"), (true, "id")],
                ),
            ],
            alias_add: vec![alias("orders", Some(8080), 10), alias("billing", None, 20)],
            ..Default::default()
        },
    );
    let live = r.live();
    assert_eq!(live.len(), 1);
    let (pair, how) = live.iter().next().unwrap();
    assert_eq!(pair.route.file, 1);
    assert_eq!(*how, Match::Full);
    // The route goes: the call is not-found in that service, then comes back.
    step(
        &mut r,
        &mut log,
        Delta {
            remove: vec![key(1, 3, Kind::Route)],
            ..Default::default()
        },
    );
    assert!(r.live().is_empty());
    assert_eq!(r.unresolved().iter().next().unwrap().1, Reason::NotFound);
    assert_eq!(log.unresolved.len(), 1);
    step(
        &mut r,
        &mut log,
        Delta {
            add: vec![route(1, 3, 10, "get", "/orders/:id")],
            ..Default::default()
        },
    );
    assert_eq!(r.live().len(), 1);
    assert!(log.unresolved.is_empty());
}

#[test]
fn without_a_host_one_full_match_is_kept_and_many_are_ambiguous() {
    let c = call(9, 1, 99, "get", &[(false, "/items/"), (true, "id")]);
    let mut r = HttpResolver::new();
    let mut log = Log::default();
    step(
        &mut r,
        &mut log,
        Delta {
            add: vec![c.clone(), route(1, 1, 1, "get", "/items/:id")],
            ..Default::default()
        },
    );
    assert_eq!(r.live().len(), 1);
    for n in 2..=4 {
        step(
            &mut r,
            &mut log,
            Delta {
                add: vec![route(n, 1, n, "get", "/items/:id")],
                ..Default::default()
            },
        );
    }
    // Four services answer it: more than three candidates.
    assert!(r.live().is_empty());
    assert_eq!(r.unresolved().iter().next().unwrap().1, Reason::Ambiguous);
}

#[test]
fn the_most_specific_route_of_a_service_wins() {
    let mut r = HttpResolver::new();
    step(
        &mut r,
        &mut Log::default(),
        Delta {
            add: vec![
                route(1, 1, 1, "get", "/orders/:id"),
                route(1, 2, 1, "get", "/orders/new"),
                call(2, 1, 2, "get", &[(false, "/orders/new")]),
            ],
            ..Default::default()
        },
    );
    let live = r.live();
    assert_eq!(live.len(), 1);
    assert_eq!(live.iter().next().unwrap().0.route.span[0], 2);
}

#[test]
fn an_all_route_answers_any_method_and_a_unknown_prefix_links_when_unique() {
    let mut r = HttpResolver::new();
    step(
        &mut r,
        &mut Log::default(),
        Delta {
            add: vec![
                route(1, 1, 1, "all", "/api/v1/orders"),
                call(2, 1, 2, "post", &[(true, "BASE"), (false, "/orders")]),
            ],
            ..Default::default()
        },
    );
    let live = r.live();
    assert_eq!(live.len(), 1);
    assert_eq!(live.iter().next().unwrap().1, Match::Partial);
}

#[test]
fn a_change_touches_only_the_calls_with_its_key() {
    let mut r = HttpResolver::new();
    let mut log = Log::default();
    let mut add = vec![route(1, 1, 1, "get", "/orders/:id")];
    for i in 0..50 {
        add.push(call(2, i + 1, 2, "get", &[(false, "/orders/1")]));
        add.push(call(3, i + 1, 2, "get", &[(false, "/users/1")]));
    }
    step(
        &mut r,
        &mut log,
        Delta {
            add,
            ..Default::default()
        },
    );
    assert_eq!(r.live().len(), 50);
    let mut touched = BTreeSet::new();
    r.affected(
        &RouteSite {
            key: key(1, 1, Kind::Route),
            service: 1,
            method: "get".into(),
            segs: normalise_route("get", "/orders/:id").segs,
        },
        &mut touched,
    );
    assert_eq!(touched.len(), 50);
    assert!(touched.iter().all(|k| k.file == 2));
}

/// One random world: routes and calls over a tiny alphabet, so they collide often.
#[derive(Debug, Clone)]
enum Op {
    Route(u8, u8, u8, String),
    Call(u8, u8, u8, String, Option<u8>),
    Remove(u8),
    Alias(u8, u8),
    Unalias(u8, u8),
}

fn arb_op() -> impl Strategy<Value = Op> {
    let path = prop::collection::vec(
        prop_oneof![
            Just("a".to_owned()),
            Just("b".to_owned()),
            Just(":id".to_owned()),
            Just("*".to_owned())
        ],
        0..4,
    )
    .prop_map(|v| format!("/{}", v.join("/")));
    prop_oneof![
        (0u8..4, 0u8..6, 0u8..3, path.clone()).prop_map(|(f, l, s, p)| Op::Route(f, l, s, p)),
        (
            4u8..8,
            0u8..6,
            0u8..3,
            path.clone(),
            prop::option::of(0u8..2)
        )
            .prop_map(|(f, l, s, p, h)| Op::Call(f, l, s, p, h)),
        (0u8..8).prop_map(Op::Remove),
        (0u8..2, 0u8..3).prop_map(|(h, s)| Op::Alias(h, s)),
        (0u8..2, 0u8..3).prop_map(|(h, s)| Op::Unalias(h, s)),
    ]
}

proptest! {
    /// Replay equals from-scratch: after every step, the links the incremental resolver holds are
    /// the ones a fresh resolver finds in the same sites, and the events it sent add up to them.
    #[test]
    fn replay_equals_from_scratch(steps in prop::collection::vec(prop::collection::vec(arb_op(), 1..5), 1..14)) {
        let mut r = HttpResolver::new();
        let mut log = Log::default();
        let mut sites: BTreeMap<SiteKey, Site> = BTreeMap::new();
        let mut aliases: BTreeSet<Alias> = BTreeSet::new();
        for ops in steps {
            let mut d = Delta::default();
            for op in ops {
                match op {
                    Op::Route(f, l, s, p) => {
                        let site = route(i64::from(f), u32::from(l), i64::from(s), "get", &p);
                        sites.insert(match &site { Site::Route(r) => r.key.clone(), _ => unreachable!() }, site.clone());
                        d.add.push(site);
                    }
                    Op::Call(f, l, s, p, h) => {
                        let url = match h {
                            Some(h) => format!("http://h{h}{p}"),
                            None => p,
                        };
                        let site = call(i64::from(f), u32::from(l), i64::from(s), "get", &[(false, &url)]);
                        sites.insert(match &site { Site::Call(c) => c.key.clone(), _ => unreachable!() }, site.clone());
                        d.add.push(site);
                    }
                    Op::Remove(f) => {
                        let gone: Vec<SiteKey> = sites.keys().filter(|k| k.file == i64::from(f)).cloned().collect();
                        for k in gone {
                            sites.remove(&k);
                            d.remove.push(k);
                        }
                    }
                    Op::Alias(h, s) => {
                        let a = alias(&format!("h{h}"), None, i64::from(s));
                        if aliases.insert(a.clone()) {
                            d.alias_add.push(a);
                        }
                    }
                    Op::Unalias(h, s) => {
                        let a = alias(&format!("h{h}"), None, i64::from(s));
                        if aliases.remove(&a) {
                            match d.alias_add.iter().position(|x| *x == a) {
                                Some(i) => {
                                    d.alias_add.remove(i);
                                }
                                None => d.alias_remove.push(a),
                            }
                        }
                    }
                }
            }
            // An op that adds then removes the same key in one delta is applied in the delta's
            // own order (removes first), so keep one delta per consistent snapshot.
            d.add.retain(|s| {
                let k = match s { Site::Call(c) => &c.key, Site::Route(r) => &r.key };
                sites.get(k) == Some(s)
            });
            r.apply(0, &d, &mut log);
            let all: Vec<Site> = sites.values().cloned().collect();
            let al: Vec<Alias> = aliases.iter().cloned().collect();
            let (live, unresolved) = from_scratch(&all, &al);
            prop_assert_eq!(&r.live(), &live);
            prop_assert_eq!(&r.unresolved(), &unresolved);
            let opened: BTreeSet<Pair> = live.iter().map(|(p, _)| p.clone()).collect();
            prop_assert_eq!(&log.open, &opened);
            let un: BTreeSet<SiteKey> = unresolved.iter().map(|(k, _)| k.clone()).collect();
            prop_assert_eq!(&log.unresolved, &un);
        }
    }

    /// The trie answers exactly what matching each route directly does.
    #[test]
    fn the_trie_agrees_with_direct_matching(
        routes in prop::collection::vec("/[ab:*]{1,1}(/[ab:*]{1,1}){0,3}", 0..8),
        call in "/[ab]{1,1}(/[ab]{1,1}){0,3}",
    ) {
        let mut node = Node::default();
        let mut segs = Vec::new();
        for (i, p) in routes.iter().enumerate() {
            let s = normalise_route("get", p).segs;
            node.insert(&s, &key(i as i64, 0, Kind::Route));
            segs.push(s);
        }
        let c = normalise("get", &call).segs;
        let mut got = Vec::new();
        node.walk(&c, 0, &mut got);
        let got: BTreeSet<(i64, Match, usize)> = got.into_iter().map(|(k, m, l)| (k.file, m, l)).collect();
        let want: BTreeSet<(i64, Match, usize)> = segs
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match_segs(&c, s).map(|(m, l)| (i as i64, m, l)))
            .collect();
        prop_assert_eq!(got, want);
    }
}
