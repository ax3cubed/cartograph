//! D1: read each repo's HEAD, compare it with the indexed commit, count commits behind (capped at
//! 1,000), for 30 repos. No network: gix is built without network features, and the driver runs this
//! under a sandbox that denies every network operation.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::R;

pub const CAP: usize = 1000;

#[derive(Debug, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub indexed: String,
}

pub fn read_state(file: &Path) -> R<Vec<Entry>> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(file)?)?;
    let arr = v.as_array().ok_or("state must be an array")?;
    Ok(arr
        .iter()
        .map(|e| Entry {
            path: PathBuf::from(e["path"].as_str().unwrap_or_default()),
            indexed: e["indexed"].as_str().unwrap_or_default().to_owned(),
        })
        .collect())
}

/// One repo: (commits behind, capped?). Opens the repo the way a read would, every time.
pub fn behind(e: &Entry) -> R<(usize, bool)> {
    let repo = gix::open_opts(&e.path, gix::open::Options::isolated())?;
    let head = repo.head_id()?.detach();
    let indexed = gix::ObjectId::from_hex(e.indexed.as_bytes())?;
    if head == indexed {
        return Ok((0, false));
    }
    let mut n = 0;
    for info in repo
        .rev_walk([head])
        .first_parent_only()
        .with_hidden([indexed])
        .all()?
    {
        info?;
        n += 1;
        if n > CAP {
            return Ok((CAP, true));
        }
    }
    Ok((n, false))
}

pub fn check_all(state: &[Entry], threads: usize) -> R<Vec<(usize, bool)>> {
    if threads <= 1 {
        return state.iter().map(behind).collect();
    }
    let chunk = state.len().div_ceil(threads);
    std::thread::scope(|s| {
        let handles: Vec<_> = state
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    c.iter()
                        .map(|e| behind(e).map_err(|e| e.to_string()))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        let mut out = Vec::new();
        for h in handles {
            for r in h.join().expect("no panic") {
                out.push(r?);
            }
        }
        Ok(out)
    })
}

/// Times `runs` checks inside this process. The first is the cold one for this process.
pub fn time(state: &[Entry], runs: usize, threads: usize) -> R<serde_json::Value> {
    let mut ms = Vec::new();
    let mut result = Vec::new();
    for _ in 0..runs {
        let t = Instant::now();
        result = check_all(state, threads)?;
        ms.push(t.elapsed().as_secs_f64() * 1e3);
    }
    let behind: Vec<_> = result
        .iter()
        .map(|(n, capped)| {
            if *capped {
                format!("{n}+")
            } else {
                n.to_string()
            }
        })
        .collect();
    // Where the time goes, one repo at a time, after the timed runs.
    let per_repo_ms: Vec<f64> = state
        .iter()
        .map(|e| {
            let t = Instant::now();
            let _ = self::behind(e);
            (t.elapsed().as_secs_f64() * 1e4).round() / 10.0
        })
        .collect();
    Ok(
        serde_json::json!({"repos": state.len(), "threads": threads, "ms": ms, "behind": behind, "per_repo_ms": per_repo_ms}),
    )
}

/// Builds `n` repos under `dir` with `git fast-import` (no network, no hooks), and a state file
/// naming each one's indexed commit. Behind counts cycle through 0, 3, 40 and, for every tenth
/// repo, 1,500 (past the cap). `extra` adds real repos, e.g. the remotion clone at 2b1b0c9.
pub fn fixture(dir: &Path, n: usize, extra: &[(PathBuf, String)]) -> R<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let mut state = Vec::new();
    for (p, rev) in extra {
        let repo = gix::open_opts(p, gix::open::Options::isolated())?;
        state.push(serde_json::json!({"path": p, "indexed": repo.rev_parse_single(rev.as_str())?.detach().to_string()}));
    }
    for i in 0..n.saturating_sub(extra.len()) {
        let path = dir.join(format!("repo-{i:02}"));
        if !path.exists() {
            run(Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .arg(&path))?;
        }
        let behind = if i % 10 == 9 { 1500 } else { [0, 3, 40][i % 3] };
        let total = 60 + behind;
        let mut stream = String::new();
        for c in 1..=total {
            let body = format!("export const v{c} = {c};\n");
            let msg = format!("commit {c}");
            let time = 1_600_000_000 + c * 60;
            stream.push_str(&format!("commit refs/heads/main\nmark :{c}\ncommitter Bench <bench@example.invalid> {time} +0000\ndata {}\n{msg}\n", msg.len()));
            if c > 1 {
                stream.push_str(&format!("from :{}\n", c - 1));
            }
            stream.push_str(&format!(
                "M 100644 inline src/f{}.ts\ndata {}\n{}\n",
                c % 50,
                body.len(),
                body
            ));
        }
        let mut child = Command::new("git")
            .args(["-C"])
            .arg(&path)
            .args(["fast-import", "--quiet", "--force"])
            .stdin(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(stream.as_bytes())?;
        if !child.wait()?.success() {
            return Err("fast-import failed".into());
        }
        // Real repos keep some refs packed; pack half of them.
        if i % 2 == 0 {
            run(Command::new("git")
                .arg("-C")
                .arg(&path)
                .args(["pack-refs", "--all"]))?;
        }
        let repo = gix::open_opts(&path, gix::open::Options::isolated())?;
        let head = repo.head_id()?.detach();
        let indexed = repo
            .rev_parse_single(format!("{head}~{behind}").as_str())?
            .detach();
        state.push(serde_json::json!({"path": path, "indexed": indexed.to_string()}));
    }
    let file = dir.join(format!("state-{n}.json"));
    std::fs::write(&file, serde_json::to_vec_pretty(&state)?)?;
    Ok(file)
}

fn run(c: &mut Command) -> R<()> {
    let s = c
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()?;
    if s.success() {
        Ok(())
    } else {
        Err(format!("{c:?} failed").into())
    }
}
