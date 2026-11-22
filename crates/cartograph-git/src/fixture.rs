//! Small git repositories built at test time with the git command, so no fixture is committed.
//! Test support only: nothing in the product calls this.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Fixture {
    pub dir: PathBuf,
    clock: i64,
}

impl Fixture {
    /// `git init` in `dir`, with a fixed identity and no user or system config.
    pub fn init(dir: &Path) -> Self {
        std::fs::create_dir_all(dir).expect("mkdir");
        let f = Self {
            dir: dir.to_owned(),
            clock: 1_600_000_000,
        };
        f.git(&["init", "-q", "-b", "main"]);
        f
    }

    pub fn git(&self, args: &[&str]) -> String {
        self.try_git(args)
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"))
    }

    /// Like [`Fixture::git`], but a failing command is an `Err` with its stderr.
    pub fn try_git(&self, args: &[&str]) -> Result<String, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.dir)
            .args(["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.com")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.com")
            .env("GIT_AUTHOR_DATE", format!("{} +0000", self.clock))
            .env("GIT_COMMITTER_DATE", format!("{} +0000", self.clock))
            .output()
            .expect("git runs");
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).into_owned());
        }
        Ok(String::from_utf8(out.stdout)
            .expect("utf-8")
            .trim_end()
            .to_owned())
    }

    pub fn write(&self, path: &str, content: &str) {
        let p = self.dir.join(path);
        std::fs::create_dir_all(p.parent().expect("has a parent")).expect("mkdir");
        std::fs::write(p, content).expect("write");
    }

    pub fn remove(&self, path: &str) {
        let _ = std::fs::remove_file(self.dir.join(path));
    }

    /// Commits the working tree on top of HEAD (or as a root commit). Returns the sha.
    pub fn commit(&mut self, msg: &str) -> String {
        let head = self.try_git(&["rev-parse", "-q", "--verify", "HEAD^{commit}"]);
        let parents: Vec<&str> = head.iter().map(String::as_str).collect();
        self.commit_with_parents(msg, &parents)
    }

    /// Commits the working tree with the given parents, first parent first, and moves HEAD to it.
    /// Any tree with any parents is a valid history, so merges never conflict.
    pub fn commit_with_parents(&mut self, msg: &str, parents: &[&str]) -> String {
        self.clock += 60;
        self.git(&["add", "-A"]);
        let tree = self.git(&["write-tree"]);
        let mut args = vec!["commit-tree", tree.as_str(), "-m", msg];
        for p in parents {
            args.extend(["-p", p]);
        }
        let sha = self.git(&args);
        self.git(&["update-ref", "HEAD", &sha]);
        sha
    }

    /// Moves the clock, so the next commit's date can go backwards (dates are not monotonic).
    pub fn set_clock(&mut self, t: i64) {
        self.clock = t;
    }

    /// `git ls-tree -r <rev>`, regular files only.
    pub fn ls_tree(&self, rev: &str) -> BTreeMap<String, String> {
        let out = self.git(&["ls-tree", "-r", "-z", rev]);
        out.split('\0')
            .filter(|l| !l.is_empty())
            .filter_map(|l| {
                let (meta, path) = l.split_once('\t')?;
                let mut m = meta.split(' ');
                let (mode, kind, oid) = (m.next()?, m.next()?, m.next()?);
                (kind == "blob" && mode.starts_with("100"))
                    .then(|| (path.to_owned(), oid.to_owned()))
            })
            .collect()
    }
}
