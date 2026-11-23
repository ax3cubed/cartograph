//! `cartograph schedule hourly | off | status` (IDX-27, D4): one user-level job that runs
//! `cartograph sync --fetch --quiet --all-workspaces` every hour, at a minute picked at install,
//! to pick up commits teammates pushed. It is opt-in, the only unattended network use, and never
//! asks for elevation:
//!
//! - macOS: a launchd agent, `~/Library/LaunchAgents/dev.cartograph.sync.plist`;
//! - Linux: a systemd user timer with `Persistent=true`, or a crontab line where there is no
//!   systemd;
//! - Windows: a schtasks task, `Cartograph\Sync`.
//!
//! `status` compares the binary path stored in the job with this binary, so a moved binary shows.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use cartograph_core::{ExitCode, Report};
use serde_json::json;

use crate::output::Out;

const LABEL: &str = "dev.cartograph.sync";
const UNIT: &str = "cartograph-sync";
const TASK: &str = "Cartograph\\Sync";
const CRON_TAG: &str = "# cartograph-sync";
const ARGS: [&str; 4] = ["sync", "--fetch", "--quiet", "--all-workspaces"];

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The launchd agent.
pub fn plist(exe: &Path, minute: u8) -> String {
    let mut args = format!("    <string>{}</string>\n", xml(&exe.to_string_lossy()));
    for a in ARGS {
        args += &format!("    <string>{a}</string>\n");
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>StartCalendarInterval</key>
  <dict>
    <key>Minute</key>
    <integer>{minute}</integer>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>/dev/null</string>
  <key>StandardErrorPath</key>
  <string>/dev/null</string>
</dict>
</plist>
"#
    )
}

/// A path as one systemd or shell word.
fn quoted(exe: &Path) -> String {
    format!("'{}'", exe.to_string_lossy().replace('\'', "'\\''"))
}

/// The systemd user service and timer.
pub fn systemd(exe: &Path, minute: u8) -> (String, String) {
    let service = format!(
        "[Unit]\nDescription=Cartograph: fetch and sync every workspace\n\n[Service]\nType=oneshot\nExecStart={} {}\n",
        quoted(exe),
        ARGS.join(" ")
    );
    let timer = format!(
        "[Unit]\nDescription=Cartograph: hourly fetch and sync\n\n[Timer]\nOnCalendar=*-*-* *:{minute:02}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n"
    );
    (service, timer)
}

/// The crontab line, where there is no systemd.
pub fn cron_line(exe: &Path, minute: u8) -> String {
    format!(
        "{minute} * * * * {} {} {CRON_TAG}",
        quoted(exe),
        ARGS.join(" ")
    )
}

/// The schtasks arguments that create the task.
pub fn schtasks_create(exe: &Path, minute: u8) -> Vec<String> {
    [
        "/Create",
        "/SC",
        "HOURLY",
        "/MO",
        "1",
        "/ST",
        &format!("00:{minute:02}"),
        "/TN",
        TASK,
        "/TR",
        &format!("\"{}\" {}", exe.to_string_lossy(), ARGS.join(" ")),
        "/F",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// A minute from 0 to 59 that isn't the same on every machine (git maintenance does the same).
fn random_minute() -> u8 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos()),
    );
    (h.finish() % 60) as u8
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn has_systemd() -> bool {
    quiet(Command::new("systemctl").args(["--user", "show-environment"]))
}

/// Where the job lives on this machine, and the binary path it runs, if installed.
pub(crate) fn installed() -> Option<(String, String)> {
    if cfg!(target_os = "macos") {
        let p = home()?
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        let text = std::fs::read_to_string(&p).ok()?;
        let start = text.find("<array>\n    <string>")? + "<array>\n    <string>".len();
        let end = text[start..].find("</string>")? + start;
        Some((
            p.display().to_string(),
            text[start..end]
                .replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&amp;", "&"),
        ))
    } else if cfg!(windows) {
        let out = Command::new("schtasks")
            .args(["/Query", "/TN", TASK, "/V", "/FO", "LIST"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let tr = text
            .lines()
            .find(|l| l.trim_start().starts_with("Task To Run:"))?;
        let exe = tr.split('"').nth(1)?.to_owned();
        out.status.success().then(|| (TASK.to_owned(), exe))
    } else {
        let unit = home()?
            .join(".config/systemd/user")
            .join(format!("{UNIT}.service"));
        if let Ok(text) = std::fs::read_to_string(&unit) {
            let exe = text
                .lines()
                .find_map(|l| l.strip_prefix("ExecStart='"))?
                .split('\'')
                .next()?
                .to_owned();
            return Some((unit.display().to_string(), exe));
        }
        let out = Command::new("crontab").arg("-l").output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let line = text.lines().find(|l| l.ends_with(CRON_TAG))?;
        let exe = line.split('\'').nth(1)?.to_owned();
        Some(("crontab".into(), exe))
    }
}

fn crontab_without_ours() -> String {
    let out = Command::new("crontab").arg("-l").output();
    let text = out
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    text.lines()
        .filter(|l| !l.ends_with(CRON_TAG))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn write_crontab(text: &str) -> bool {
    use std::io::Write;
    let Ok(mut child) = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
    else {
        return false;
    };
    let ok = child
        .stdin
        .take()
        .is_some_and(|mut s| s.write_all(text.as_bytes()).is_ok());
    ok && child.wait().is_ok_and(|s| s.success())
}

fn uid() -> String {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn failed(what: &str, e: impl std::fmt::Display) -> Report {
    Report {
        what: format!("can't {what}"),
        why: e.to_string(),
        action: "check that the user's scheduler runs, or schedule `cartograph sync --fetch --all-workspaces` by hand".into(),
        exit_code: ExitCode::Error,
    }
}

/// Removes the job, whatever form it has on this OS. Nothing to remove is not an error.
fn off() -> Result<(), Report> {
    if cfg!(target_os = "macos") {
        let Some(h) = home() else { return Ok(()) };
        let p = h
            .join("Library/LaunchAgents")
            .join(format!("{LABEL}.plist"));
        quiet(Command::new("launchctl").args(["bootout", &format!("gui/{}/{LABEL}", uid())]));
        match std::fs::remove_file(&p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return Err(failed("remove the launchd agent", e));
            }
            _ => {}
        }
    } else if cfg!(windows) {
        quiet(Command::new("schtasks").args(["/Delete", "/TN", TASK, "/F"]));
    } else {
        let Some(h) = home() else { return Ok(()) };
        let dir = h.join(".config/systemd/user");
        if dir.join(format!("{UNIT}.timer")).exists() {
            quiet(Command::new("systemctl").args([
                "--user",
                "disable",
                "--now",
                &format!("{UNIT}.timer"),
            ]));
            for f in [format!("{UNIT}.timer"), format!("{UNIT}.service")] {
                let _ = std::fs::remove_file(dir.join(f));
            }
            quiet(Command::new("systemctl").args(["--user", "daemon-reload"]));
        }
        if Command::new("crontab")
            .arg("-l")
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(CRON_TAG))
        {
            let rest = crontab_without_ours();
            if !write_crontab(&rest) {
                return Err(failed("edit the crontab", "crontab - failed"));
            }
        }
    }
    Ok(())
}

fn hourly(exe: &Path, minute: u8) -> Result<String, Report> {
    // Installing again replaces the job, so there is only ever one.
    off()?;
    let h = home().ok_or_else(|| failed("find the home folder", "HOME is not set"))?;
    if cfg!(target_os = "macos") {
        let dir = h.join("Library/LaunchAgents");
        std::fs::create_dir_all(&dir).map_err(|e| failed("create ~/Library/LaunchAgents", e))?;
        let p = dir.join(format!("{LABEL}.plist"));
        std::fs::write(&p, plist(exe, minute)).map_err(|e| failed("write the launchd agent", e))?;
        if !quiet(
            Command::new("launchctl")
                .args(["bootstrap", &format!("gui/{}", uid())])
                .arg(&p),
        ) {
            return Err(failed(
                "load the launchd agent",
                "launchctl bootstrap failed",
            ));
        }
        Ok(format!("launchd agent {LABEL}"))
    } else if cfg!(windows) {
        if !quiet(Command::new("schtasks").args(schtasks_create(exe, minute))) {
            return Err(failed(
                "create the scheduled task",
                "schtasks /Create failed",
            ));
        }
        Ok(format!("scheduled task {TASK}"))
    } else if has_systemd() {
        let dir = h.join(".config/systemd/user");
        std::fs::create_dir_all(&dir).map_err(|e| failed("create ~/.config/systemd/user", e))?;
        let (service, timer) = systemd(exe, minute);
        std::fs::write(dir.join(format!("{UNIT}.service")), service)
            .map_err(|e| failed("write the service", e))?;
        std::fs::write(dir.join(format!("{UNIT}.timer")), timer)
            .map_err(|e| failed("write the timer", e))?;
        quiet(Command::new("systemctl").args(["--user", "daemon-reload"]));
        if !quiet(Command::new("systemctl").args([
            "--user",
            "enable",
            "--now",
            &format!("{UNIT}.timer"),
        ])) {
            return Err(failed("start the timer", "systemctl --user enable failed"));
        }
        Ok(format!("systemd user timer {UNIT}.timer"))
    } else {
        let text = crontab_without_ours() + &cron_line(exe, minute) + "\n";
        if !write_crontab(&text) {
            return Err(failed("edit the crontab", "crontab - failed"));
        }
        Ok("a crontab line".into())
    }
}

pub fn run(out: &Out, action: crate::Schedule) -> Result<(), Report> {
    let exe = std::env::current_exe().map_err(|e| failed("find the cartograph binary", e))?;
    match action {
        crate::Schedule::Hourly => {
            let minute = random_minute();
            let what = hourly(&exe, minute)?;
            out.print(
                &format!("installed {what}: `cartograph sync --fetch --all-workspaces` at minute {minute} of every hour"),
                json!({ "schedule": "hourly", "minute": minute, "job": what }),
            );
        }
        crate::Schedule::Off => {
            off()?;
            out.print("the hourly sync is off", json!({ "schedule": "off" }));
        }
        crate::Schedule::Status => {
            match installed() {
                None => out.print("no hourly sync", json!({ "schedule": "off" })),
                Some((job, stored)) => {
                    let stale = Path::new(&stored) != exe;
                    let mut text = format!("hourly sync: {job}");
                    if stale {
                        text += &format!(
                            "\n✗ it runs {stored}, not this binary ({}); run `cartograph schedule hourly`",
                            exe.display()
                        );
                    }
                    out.print(&text, json!({ "schedule": "hourly", "job": job, "binary": stored, "stale": stale }));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_job_files() {
        let exe = Path::new("/opt/carto & co/cartograph");
        let p = plist(exe, 7);
        assert!(p.contains("    <string>/opt/carto &amp; co/cartograph</string>\n    <string>sync</string>\n    <string>--fetch</string>\n    <string>--quiet</string>\n    <string>--all-workspaces</string>\n"));
        assert!(p.contains("<key>Minute</key>\n    <integer>7</integer>"));
        let (service, timer) = systemd(exe, 7);
        assert_eq!(
            service,
            "[Unit]\nDescription=Cartograph: fetch and sync every workspace\n\n[Service]\nType=oneshot\nExecStart='/opt/carto & co/cartograph' sync --fetch --quiet --all-workspaces\n"
        );
        assert!(timer.contains("OnCalendar=*-*-* *:07:00\nPersistent=true\n"));
        assert_eq!(
            cron_line(exe, 7),
            "7 * * * * '/opt/carto & co/cartograph' sync --fetch --quiet --all-workspaces # cartograph-sync"
        );
        assert_eq!(
            schtasks_create(Path::new("C:\\Tools\\cartograph.exe"), 7),
            [
                "/Create",
                "/SC",
                "HOURLY",
                "/MO",
                "1",
                "/ST",
                "00:07",
                "/TN",
                "Cartograph\\Sync",
                "/TR",
                "\"C:\\Tools\\cartograph.exe\" sync --fetch --quiet --all-workspaces",
                "/F"
            ]
        );
        assert!(random_minute() < 60);
    }
}
