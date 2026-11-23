//! Everything the CLI prints goes through here: human lines or `{"schema":1,…}` on stdout, errors
//! on stderr, with control characters stripped from every human line (terminal escape injection).

use std::borrow::Cow;
use std::io::{BufRead, IsTerminal, Write};

use cartograph_core::{ExitCode, Report};
use serde_json::{Map, Value};

/// The JSON output's version. Bump it when a field changes meaning or goes away.
pub const SCHEMA: u32 = 1;

/// How this run prints, from the global flags and the environment.
pub struct Out {
    pub json: bool,
    pub quiet: bool,
    /// Colour on stderr, where errors go. Nothing on stdout is coloured yet.
    pub color: bool,
    /// A result went to stdout, so an error after it (`status --check`) doesn't add a second
    /// JSON object.
    printed: std::sync::atomic::AtomicBool,
}

impl Out {
    pub fn new(json: bool, quiet: bool, no_color: bool) -> Self {
        let env = |k| std::env::var_os(k).filter(|v| !v.is_empty());
        Self {
            json,
            quiet,
            color: color_on(
                no_color,
                env("NO_COLOR").is_some(),
                env("CLICOLOR_FORCE").is_some_and(|v| v != "0"),
                std::io::stderr().is_terminal(),
            ),
            printed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Prints a result: `human` line by line, or `json` (an object) with `"schema":1` first.
    /// `--quiet` drops the human lines but never the JSON someone asked for.
    pub fn print(&self, human: &str, json: Value) {
        if self.json {
            let mut obj = Map::new();
            obj.insert("schema".into(), SCHEMA.into());
            if let Value::Object(fields) = json {
                obj.extend(fields);
            }
            write_stdout(&Value::Object(obj).to_string());
            self.printed
                .store(true, std::sync::atomic::Ordering::Relaxed);
        } else if !self.quiet {
            for line in human.lines() {
                write_stdout(&sanitize(line));
            }
        }
    }

    /// Prints an error as three lines on stderr (E1). With `--json` it is also the one object on
    /// stdout, `{"schema":1,"error":{"code","what","why","try"}}` (SRV-07's error shape), unless a
    /// result is already there: a script reading stdout always gets an object.
    pub fn error(&self, r: &Report) {
        if self.json && !self.printed.load(std::sync::atomic::Ordering::Relaxed) {
            let code = match r.exit_code {
                ExitCode::Usage => "usage",
                ExitCode::Behind => "behind",
                _ => "failed",
            };
            let error = serde_json::json!({
                "code": code, "what": r.what, "why": r.why, "try": r.action,
            });
            let mut obj = Map::new();
            obj.insert("schema".into(), SCHEMA.into());
            obj.insert("error".into(), error);
            write_stdout(&Value::Object(obj).to_string());
        }
        let label = if self.color {
            "\x1b[31merror:\x1b[0m"
        } else {
            "error:"
        };
        eprintln!(
            "{label} {}\n  why: {}\n  try: {}",
            sanitize(&r.what),
            sanitize(&r.why),
            sanitize(&r.action)
        );
    }

    /// Asks before a destructive step. `--yes` skips the question; without a terminal to ask on,
    /// it fails and names `--yes`.
    pub fn confirm(&self, question: &str, yes: bool) -> Result<(), Report> {
        if yes {
            return Ok(());
        }
        if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
            return Err(Report {
                what: format!("not confirmed: {}", sanitize(question)),
                why: "there is no terminal to ask on".into(),
                action: "run the same command with --yes".into(),
                exit_code: ExitCode::Usage,
            });
        }
        eprint!("{} [y/N] ", sanitize(question));
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if matches!(answer.trim(), "y" | "Y" | "yes") {
            return Ok(());
        }
        Err(Report {
            what: "stopped; nothing was changed".into(),
            why: "the answer wasn't yes".into(),
            action: "run it again and answer y, or pass --yes".into(),
            exit_code: ExitCode::Error,
        })
    }
}

/// `--no-color` and `NO_COLOR` win, then `CLICOLOR_FORCE`, then whether stderr is a terminal.
fn color_on(no_color_flag: bool, no_color_env: bool, force: bool, tty: bool) -> bool {
    !no_color_flag && !no_color_env && (force || tty)
}

/// Writes one line to stdout. A closed pipe (`cartograph tool … | head`) ends the process quietly.
// ponytail: exit 0 on EPIPE instead of resetting SIGPIPE, which needs `unsafe` (forbidden in this
// workspace). Switch to `-Zon-broken-pipe=kill`'s stable form when it lands.
fn write_stdout(line: &str) {
    let mut out = std::io::stdout().lock();
    if let Err(e) = writeln!(out, "{line}")
        && e.kind() == std::io::ErrorKind::BrokenPipe
    {
        std::process::exit(0);
    }
}

/// Removes control characters (C0, DEL, C1, so ESC and CSI), which would let a repo, branch or
/// path name or a commit subject move the cursor, rewrite the screen or set the window title.
pub fn sanitize(s: &str) -> Cow<'_, str> {
    if s.chars().any(char::is_control) {
        Cow::Owned(s.chars().filter(|c| !c.is_control()).collect())
    } else {
        Cow::Borrowed(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_escapes_and_line_breaks() {
        assert_eq!(
            sanitize("src/\x1b[2J\x1b]0;pwned\x07a.ts"),
            "src/[2J]0;pwneda.ts"
        );
        assert_eq!(sanitize("a\u{9b}31mb\r\nc\x7f"), "a31mbc");
        assert!(matches!(sanitize("plain/path.ts"), Cow::Borrowed(_)));
    }

    #[test]
    fn colour_rules() {
        assert!(color_on(false, false, false, true));
        assert!(!color_on(false, false, false, false));
        assert!(
            color_on(false, false, true, false),
            "CLICOLOR_FORCE without a terminal"
        );
        assert!(
            !color_on(false, true, true, true),
            "NO_COLOR beats CLICOLOR_FORCE"
        );
        assert!(
            !color_on(true, false, true, true),
            "--no-color beats everything"
        );
    }
}
