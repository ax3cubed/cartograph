//! `acme-scale`: 9 repositories generated to board 00's numbers: 62 services, 1,140 modules,
//! 8,312 source files and 35,439 commits, on one six-year timeline.
//!
//! Each repository is a pnpm workspace with one package per service (`services/<svc>/`, exporting
//! `./*` from `./src/*.ts`). The first commit writes the manifests; after that, files are created
//! evenly across the history, and every commit also rewrites one existing file with new imports:
//! relative ones inside its service and `@acme/<svc>/…` ones across services. All choices come from
//! one seeded generator, and the history is written with one `git fast-import` per repository, which
//! is what keeps generation well under 60 s.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use cartograph_git::fixture::Fixture;

use crate::{Rng, relative, share};

pub(crate) const SOURCE: &str = include_str!("acme.rs");

pub const REPOS: usize = 9;
pub const SERVICES: usize = 62;
pub const MODULES: usize = 1_140;
pub const FILES: usize = 8_312;
pub const COMMITS: usize = 35_439;
const SEED: u64 = 0x00ac_3e5c_a1e0_0000;
/// 2019-01-01T00:00:00Z, and six years of history after it.
const START: i64 = 1_546_300_800;
const SPAN: i64 = 6 * 365 * 86_400;
const AUTHORS: [&str; 12] = [
    "Ana Lima",
    "Ben Ode",
    "Chen Wei",
    "Dara Kim",
    "Eli Novak",
    "Fay Grant",
    "Gus Moreau",
    "Hana Sato",
    "Ivo Petrov",
    "Jia Rao",
    "Kofi Mensah",
    "Lena Berg",
];

/// The repository names, `acme-01` to `acme-09`.
pub fn repo_names() -> Vec<String> {
    (1..=REPOS).map(|r| format!("acme-{r:02}")).collect()
}

struct File {
    service: usize,
    path: String,
    /// The import specifier other services use.
    package_spec: String,
    revision: u32,
}

pub(crate) fn generate(root: &Path) {
    let mut rng = Rng::new(SEED);
    let (mut service, mut module) = (0, 0);
    for (r, name) in repo_names().into_iter().enumerate() {
        let services: Vec<usize> = (service..service + share(SERVICES, REPOS, r)).collect();
        service += services.len();
        let mut files = Vec::new();
        for &s in &services {
            for _ in 0..share(MODULES, SERVICES, s) {
                for n in 0..share(FILES, MODULES, module) {
                    let rel = format!("m{module:04}/f{n}");
                    files.push(File {
                        service: s,
                        path: format!("services/svc-{s:02}/src/{rel}.ts"),
                        package_spec: format!("@acme/svc-{s:02}/{rel}"),
                        revision: 0,
                    });
                }
                module += 1;
            }
        }
        let commits = share(COMMITS, REPOS, r);
        let dir = root.join(&name);
        let f = Fixture::init(&dir);
        let stream = history(&mut rng, r, &services, &mut files, commits);
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&f.dir)
            .args(["fast-import", "--quiet", "--done"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .stdin(Stdio::piped())
            .spawn()
            .expect("git fast-import runs");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(stream.as_bytes())
            .expect("fast-import reads the stream");
        assert!(child.wait().expect("fast-import ends").success());
        f.git(&["reset", "-q", "--hard"]);
    }
    assert_eq!((service, module), (SERVICES, MODULES));
}

/// The fast-import stream of one repository.
fn history(
    rng: &mut Rng,
    r: usize,
    services: &[usize],
    files: &mut [File],
    commits: usize,
) -> String {
    let mut out = String::new();
    let blob = |out: &mut String, path: &str, body: &str| {
        let _ = write!(out, "M 100644 inline {path}\ndata {}\n{body}\n", body.len());
    };
    let n = files.len();
    let created = |c: usize| {
        // File j is created at commit 1 + j * (commits - 1) / files.
        (c.saturating_sub(1) * n).div_ceil(commits - 1)..(c * n).div_ceil(commits - 1).min(n)
    };
    for c in 0..commits {
        let time = START + (c as i64 * SPAN) / commits as i64 + r as i64 * 37;
        let author = AUTHORS[rng.below(AUTHORS.len())];
        let email = author.to_lowercase().replace(' ', ".");
        let svc = services[rng.below(services.len())];
        let msg = format!("svc-{svc:02}: change {c}\n");
        let _ = write!(
            out,
            "commit refs/heads/main\nauthor {author} <{email}@acme.example> {time} +0000\ncommitter {author} <{email}@acme.example> {time} +0000\ndata {}\n{msg}",
            msg.len()
        );
        if c == 0 {
            blob(
                &mut out,
                "package.json",
                "{\n  \"name\": \"acme\",\n  \"private\": true\n}\n",
            );
            blob(
                &mut out,
                "pnpm-workspace.yaml",
                "packages:\n  - services/*\n",
            );
            for s in services {
                blob(
                    &mut out,
                    &format!("services/svc-{s:02}/package.json"),
                    &format!(
                        "{{\n  \"name\": \"@acme/svc-{s:02}\",\n  \"exports\": {{ \"./*\": \"./src/*.ts\" }}\n}}\n"
                    ),
                );
            }
            out.push('\n');
            continue;
        }
        // New files, then one rewrite of an older file. Imports only point at lower-numbered files,
        // which already exist.
        let new = created(c);
        let mut touch: Vec<usize> = new.clone().collect();
        if new.start > 0 {
            touch.push(rng.below(new.start));
        }
        for j in touch {
            files[j].revision += 1;
            let mut body = format!("// rev {}\n", files[j].revision);
            let targets: BTreeSet<usize> = (0..rng.below(4))
                .filter(|_| j > 0)
                .map(|_| rng.below(j.max(1)))
                .collect();
            for k in targets {
                let spec = if files[k].service == files[j].service {
                    relative(&files[j].path, files[k].path.trim_end_matches(".ts"))
                } else {
                    files[k].package_spec.clone()
                };
                let _ = writeln!(body, "import * as i{k} from \"{spec}\";");
            }
            let _ = writeln!(body, "export const v{j} = {};", files[j].revision);
            blob(&mut out, &files[j].path, &body);
        }
        out.push('\n');
    }
    out.push_str("done\n");
    out
}
