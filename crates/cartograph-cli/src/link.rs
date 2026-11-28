//! `cartograph://<workspace-name>[/route]#<state>` share links (I7, SRV-18). A link carries view
//! state only: the workspace must be in the registry, the hash keys come from an allowlist, and
//! values are length-capped, so a link can never name a path, a URL or a command.

use cartograph_core::{ExitCode, Report};

/// The hash keys a link may carry (I7); anything else is dropped.
pub const KEYS: [&str; 10] = [
    "v", "t", "lod", "focus", "trace", "commit", "cmp", "kinds", "minc", "view",
];
const MAX_VALUE: usize = 200;
const MAX_ROUTE: usize = 32;
const MAX_LINK: usize = 2048;

#[derive(Debug, PartialEq, Eq)]
pub struct Link {
    pub workspace: String,
    pub route: String,
    pub state: Vec<(String, String)>,
}

fn bad(why: &str) -> Report {
    Report {
        what: "that isn't a Cartograph link".into(),
        why: why.into(),
        action: "copy it again from Share in the app".into(),
        exit_code: ExitCode::Usage,
    }
}

fn decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let h = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(h, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Parses `link` against the registered workspace names.
pub fn parse(link: &str, names: &[String]) -> Result<Link, Report> {
    if link.len() > MAX_LINK {
        return Err(bad("it is too long"));
    }
    let rest = link
        .strip_prefix("cartograph://")
        .ok_or_else(|| bad("it doesn't start with cartograph://"))?;
    let (head, hash) = rest.split_once('#').unwrap_or((rest, ""));
    let (name, route) = head.split_once('/').unwrap_or((head, ""));
    let workspace = decode(name).ok_or_else(|| bad("the workspace name is garbled"))?;
    if !names.contains(&workspace) {
        return Err(Report {
            what: format!("no workspace named {}", crate::output::sanitize(&workspace)),
            why: "the link names a workspace this machine hasn't registered".into(),
            action: "run `cartograph status` to see the workspaces here".into(),
            exit_code: ExitCode::Usage,
        });
    }
    let route_ok = route.len() <= MAX_ROUTE
        && route
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !route_ok {
        return Err(bad("the view name isn't one of ours"));
    }
    let mut state: Vec<(String, String)> = Vec::new();
    for pair in hash.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        if !KEYS.contains(&k) || state.iter().any(|(s, _)| s == k) {
            continue;
        }
        let v = decode(v).ok_or_else(|| bad("a value is garbled"))?;
        if v.chars().count() > MAX_VALUE || v.chars().any(char::is_control) {
            return Err(bad("a value is too long or has control characters"));
        }
        state.push((k.to_owned(), v));
    }
    Ok(Link {
        workspace,
        route: route.to_owned(),
        state,
    })
}

impl Link {
    /// The local address that shows this view: the server's origin, the route, the token and the
    /// state, all in the fragment so nothing reaches a log.
    pub fn url(&self, origin: &str, token: &str) -> String {
        let mut u = format!("{origin}/{}#k={token}", self.route);
        for (k, v) in &self.state {
            u.push_str(&format!("&{k}={}", encode(v)));
        }
        u
    }
}

/// The `.desktop` file that makes `cartograph://` links open here (Linux).
pub fn desktop_entry(exe: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Cartograph\nNoDisplay=true\n\
         Exec=\"{exe}\" open \"%u\"\nMimeType=x-scheme-handler/cartograph;\n"
    )
}

/// The `reg` calls that register the handler under HKCU (Windows).
pub fn reg_adds(exe: &str) -> Vec<Vec<String>> {
    let k = r"HKCU\Software\Classes\cartograph";
    let a = |args: &[&str]| args.iter().map(|s| (*s).to_owned()).collect();
    vec![
        a(&["add", k, "/ve", "/d", "URL:Cartograph", "/f"]),
        a(&["add", k, "/v", "URL Protocol", "/d", "", "/f"]),
        vec![
            "add".into(),
            format!(r"{k}\shell\open\command"),
            "/ve".into(),
            "/d".into(),
            format!("\"{exe}\" open \"%1\""),
            "/f".into(),
        ],
    ]
}

/// Refuses a program path that could break out of the quoted handler line.
pub fn safe_exe(exe: &str) -> Result<(), Report> {
    if exe
        .chars()
        .any(|c| matches!(c, '"' | '%' | '\n' | '\r' | '\0'))
    {
        return Err(bad(
            "the program path has characters a handler line can't hold",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn names() -> Vec<String> {
        vec!["shop".into(), "my team".into()]
    }

    #[test]
    fn a_link_opens_the_same_node_date_and_filters() {
        let l = parse(
            "cartograph://shop/coverage#v=1&t=2026-03-01&focus=a%2Fb.ts&kinds=http,grpc&evil=1&focus=x",
            &names(),
        )
        .unwrap();
        assert_eq!(
            l.url("http://127.0.0.1:7000", "TOK"),
            "http://127.0.0.1:7000/coverage#k=TOK&v=1&t=2026-03-01&focus=a%2Fb.ts&kinds=http%2Cgrpc"
        );
        assert!(parse("cartograph://my%20team", &names()).is_ok());
    }

    #[test]
    fn a_link_that_names_more_than_a_view_is_refused() {
        for l in [
            "http://shop",
            "cartograph://nope",
            "cartograph://shop/../etc",
            "cartograph://shop/C:",
            "cartograph://shop#focus=%ZZ",
            "cartograph://shop#focus=%0A",
        ] {
            assert!(parse(l, &names()).is_err(), "{l}");
        }
        let long = format!("cartograph://shop#focus={}", "a".repeat(201));
        assert!(parse(&long, &names()).is_err());
    }

    #[test]
    fn handlers_quote_the_argument() {
        assert!(desktop_entry("/usr/bin/cartograph").contains("open \"%u\""));
        assert_eq!(reg_adds(r"C:\c.exe")[2][4], "\"C:\\c.exe\" open \"%1\"");
        assert!(safe_exe("/a\"b").is_err());
    }

    proptest! {
        // Whatever the input, parsing never panics, and what comes out is allowlisted and capped.
        #[test]
        fn parsing_is_total_and_its_output_is_view_state_only(s in "\\PC{0,300}", t in "[a-z%0-9&=#/:.]{0,120}") {
            for input in [s.clone(), format!("cartograph://shop/{t}"), format!("cartograph://shop#{t}"), format!("cartograph://{s}")] {
                if let Ok(l) = parse(&input, &names()) {
                    prop_assert!(names().contains(&l.workspace));
                    prop_assert!(l.route.len() <= MAX_ROUTE);
                    for (k, v) in &l.state {
                        prop_assert!(KEYS.contains(&k.as_str()));
                        prop_assert!(v.chars().count() <= MAX_VALUE && !v.chars().any(char::is_control));
                    }
                    let url = l.url("http://127.0.0.1:1", "T");
                    prop_assert!(!url["http://127.0.0.1:1/".len()..].contains("://"));
                }
            }
        }
    }
}
