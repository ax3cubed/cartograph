//! IDX-02 spike. Each subcommand prints one JSON object to stdout. See bench/idx-spike/README.md.

mod fresh;
mod resolve;
mod sqlite;
mod walk;

use std::path::PathBuf;
use std::time::Instant;

pub type R<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub fn median(v: &[f64]) -> f64 {
    percentile(v, 0.5)
}

pub fn percentile(v: &[f64], p: f64) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * p).round() as usize]
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn num(args: &[String], name: &str, default: usize) -> usize {
    flag(args, name)
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn open(path: &str) -> R<gix::Repository> {
    let mut repo = gix::open_opts(path, gix::open::Options::isolated())?;
    repo.object_cache_size_if_unset(64 * 1024 * 1024);
    Ok(repo)
}

fn main() -> R<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let pos = |i: usize| {
        args.get(i)
            .cloned()
            .ok_or_else(|| format!("missing argument {i}; see README.md"))
    };
    let out = match args.first().map(String::as_str) {
        // walk <repo> <rev> <gix|git2|git> [--renames] [--runs N]
        Some("walk") => {
            let (repo, rev, imp) = (pos(1)?, pos(2)?, pos(3)?);
            let renames = args.iter().any(|a| a == "--renames");
            let mut ms = Vec::new();
            let mut last = Vec::new();
            for _ in 0..num(&args, "--runs", 1) {
                let t = Instant::now();
                last = match imp.as_str() {
                    "gix" => walk::gix_changes(&open(&repo)?, &rev, renames)?,
                    "git2" => walk::git2_changes(&repo, &rev, renames)?,
                    "git" => walk::git_log_changes(&repo, &rev, renames)?,
                    other => return Err(format!("unknown implementation {other}").into()),
                };
                ms.push(t.elapsed().as_secs_f64() * 1e3);
            }
            serde_json::json!({
                "experiment": "walk", "impl": imp, "renames": renames, "ms": ms, "median_ms": median(&ms),
                "commits": last.len(),
                "changed_paths": last.iter().map(|c| c.paths.len()).sum::<usize>(),
                "renames_found": last.iter().map(|c| c.renames.len()).sum::<usize>(),
            })
        }
        // agree <repo> <rev>: per-commit agreement of gix and libgit2 with git log, without and with renames.
        Some("agree") => {
            let (repo, rev) = (pos(1)?, pos(2)?);
            let g = open(&repo)?;
            let mut res = serde_json::Map::new();
            for renames in [false, true] {
                let git = walk::git_log_changes(&repo, &rev, renames)?;
                let gx = walk::gix_changes(&g, &rev, renames)?;
                let g2 = walk::git2_changes(&repo, &rev, renames)?;
                let key = if renames { "renames_m50" } else { "no_renames" };
                res.insert(key.into(), serde_json::json!({"gix_vs_git": walk::agreement(&gx, &git), "git2_vs_git": walk::agreement(&g2, &git)}));
            }
            serde_json::json!({"experiment": "agree", "rev": rev, "result": res})
        }
        // resolve <repo> <rev> [--runs N]
        Some("resolve") => {
            let (repo, rev) = (pos(1)?, pos(2)?);
            let mut v = resolve::at_commit(&open(&repo)?, &rev, num(&args, "--runs", 10))?;
            v["experiment"] = "resolve".into();
            v
        }
        // replay <repo> <rev> [--pairs 24] [--gap 50]
        Some("replay") => {
            let (repo, rev) = (pos(1)?, pos(2)?);
            let mut v = resolve::replay_check(&open(&repo)?, &rev, num(&args, "--pairs", 24), num(&args, "--gap", 50))?;
            v["experiment"] = "replay".into();
            v
        }
        // fresh-fixture <dir> <n> [--extra <repo>@<rev>]
        Some("fresh-fixture") => {
            let dir = PathBuf::from(pos(1)?);
            let n: usize = pos(2)?.parse()?;
            let extra: Vec<(PathBuf, String)> = flag(&args, "--extra")
                .map(|e| {
                    let (p, r) = e.rsplit_once('@').unwrap_or((e.as_str(), "HEAD"));
                    vec![(PathBuf::from(p), r.to_owned())]
                })
                .unwrap_or_default();
            let file = fresh::fixture(&dir, n, &extra)?;
            serde_json::json!({"experiment": "fresh-fixture", "state": file})
        }
        // fresh <state.json> [--runs N] [--threads T]
        Some("fresh") => {
            let state = fresh::read_state(&PathBuf::from(pos(1)?))?;
            let mut v = fresh::time(&state, num(&args, "--runs", 10), num(&args, "--threads", 1))?;
            v["experiment"] = "fresh".into();
            v
        }
        // sqlite <dir> <remotion|synthetic> [--runs N]
        Some("sqlite") => {
            let mut v = sqlite::bench(&PathBuf::from(pos(1)?), &pos(2)?, num(&args, "--runs", 10))?;
            v["experiment"] = "sqlite".into();
            v
        }
        _ => return Err("usage: idx-spike walk|agree|resolve|replay|fresh-fixture|fresh|sqlite …; see README.md".into()),
    };
    println!("{}", serde_json::to_string(&out)?);
    Ok(())
}
