//! `cartograph open` and `cartograph serve` (SRV-05, SRV-06): one local server per user, on
//! loopback, found again through `server.json`.

use std::io::IsTerminal;

use cartograph_core::{ExitCode, Report, logging};
use cartograph_discovery::registry::Registry;
use cartograph_server::{ServeOptions, ServerInfo, running};
use serde_json::json;

use crate::link;
use crate::output::Out;
use crate::{Global, data_dir};

/// Opens the app, at the view a `cartograph://` link names (I7) if one is given: reuses the
/// running server when there is one, else starts one and serves until Ctrl-C.
pub fn open(out: &Out, g: &Global, link: Option<&str>) -> Result<(), Report> {
    let data = data_dir()?;
    let link = match link {
        None => None,
        Some(l) => {
            let names: Vec<String> = Registry::load(&data)
                .map(|(r, _)| r.workspaces.into_iter().map(|w| w.name).collect())
                .unwrap_or_default();
            Some(link::parse(l, &names)?)
        }
    };
    if let Some(info) = running(&data) {
        announce(out, &info, true, link.as_ref());
        return Ok(());
    }
    run(out, g, None, None, None, true, link)
}

/// Registers (or removes) `cartograph://` with the desktop, only when asked (SRV-18).
pub fn scheme(out: &Out, register: bool) -> Result<(), Report> {
    let fail = |what: &str, why: String| Report {
        what: what.into(),
        why,
        action: "register it by hand, or skip it: `cartograph open` works without it".into(),
        exit_code: ExitCode::Error,
    };
    let exe = std::env::current_exe()
        .map_err(|e| fail("can't find this program", e.to_string()))?
        .to_string_lossy()
        .into_owned();
    link::safe_exe(&exe)?;
    let run = |prog: &str, args: &[String]| {
        std::process::Command::new(prog)
            .args(args)
            .stdout(std::process::Stdio::null())
            .status()
            .map_err(|e| e.to_string())
            .and_then(|s| s.success().then_some(()).ok_or_else(|| s.to_string()))
    };
    let said = if cfg!(windows) {
        let reg = std::path::Path::new(
            &std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("System32\\reg.exe");
        let reg = reg.to_string_lossy().into_owned();
        let calls = if register {
            link::reg_adds(&exe)
        } else {
            vec![vec![
                "delete".into(),
                r"HKCU\Software\Classes\cartograph".into(),
                "/f".into(),
            ]]
        };
        for c in &calls {
            run(&reg, c).map_err(|e| fail("can't change the registry", e))?;
        }
        "under HKCU"
    } else if cfg!(target_os = "macos") {
        return Err(fail(
            "macOS registers cartograph:// through the app bundle",
            "the CLI has no bundle to register".into(),
        ));
    } else {
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
        let dir = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| home.map(|h| h.join(".local/share")))
            .ok_or_else(|| fail("can't find your data folder", "HOME is not set".into()))?
            .join("applications");
        let file = dir.join("cartograph-handler.desktop");
        if register {
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&file, link::desktop_entry(&exe)))
                .map_err(|e| fail("can't write the .desktop file", e.to_string()))?;
            run(
                "/usr/bin/xdg-mime",
                &[
                    "default".into(),
                    "cartograph-handler.desktop".into(),
                    "x-scheme-handler/cartograph".into(),
                ],
            )
            .map_err(|e| fail("xdg-mime failed", e))?;
        } else if let Err(e) = std::fs::remove_file(&file)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(fail("can't remove the .desktop file", e.to_string()));
        }
        "with a .desktop file"
    };
    let verb = if register { "registered" } else { "removed" };
    out.print(
        &format!("cartograph:// {verb} {said}"),
        json!({ "scheme": "cartograph", "registered": register }),
    );
    Ok(())
}

/// Serves without opening a browser, for development and tests.
/// `fixture` (debug builds, HD-04) serves that data folder instead of the user's.
pub fn serve(
    out: &Out,
    g: &Global,
    host: Option<String>,
    port: Option<u16>,
    fixture: Option<std::path::PathBuf>,
) -> Result<(), Report> {
    run(out, g, host, port, fixture, false, None)
}

fn run(
    out: &Out,
    g: &Global,
    host: Option<String>,
    port: Option<u16>,
    fixture: Option<std::path::PathBuf>,
    browser: bool,
    link: Option<link::Link>,
) -> Result<(), Report> {
    let data = match fixture {
        Some(d) => d,
        None => data_dir()?,
    };
    let _log = logging::init(&data.join("logs"), g.verbose).ok();
    cartograph_server::serve(
        ServeOptions {
            data_dir: data,
            host,
            port,
            home: None,
        },
        |info| announce(out, info, browser, link.as_ref()),
    )
    .map_err(|e| Report::from_error(&e))
}

/// Prints the address with its token (stdout, so only this user sees it) and, at a terminal,
/// opens the browser on it.
fn announce(out: &Out, info: &ServerInfo, browser: bool, link: Option<&link::Link>) {
    let url = link.map_or_else(|| info.url(), |l| l.url(&info.origin(), &info.token));
    out.print(
        &format!("Cartograph is at {url}"),
        json!({ "url": url, "port": info.port, "pid": info.pid }),
    );
    if browser && std::io::stdout().is_terminal() {
        launch(&url);
    }
}

/// Hands the URL to the platform's opener by absolute path, never a PATH lookup. Best effort: the
/// address is already printed.
fn launch(url: &str) {
    #[cfg(target_os = "macos")]
    let cmd = std::process::Command::new("/usr/bin/open").arg(url).spawn();
    #[cfg(windows)]
    let cmd = std::process::Command::new(
        std::path::Path::new(
            &std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()),
        )
        .join("System32\\rundll32.exe"),
    )
    .args(["url.dll,FileProtocolHandler", url])
    .spawn();
    #[cfg(not(any(target_os = "macos", windows)))]
    let cmd = std::process::Command::new("/usr/bin/xdg-open")
        .arg(url)
        .spawn();
    let _ = cmd;
}
